// opencode-traffic-light 状态推送插件
//
// 把 opencode 的 session 状态实时推送到本机运行的 Rust 监控器。
// 监控器默认监听 127.0.0.1:9912，可用环境变量 OPENCODE_TL_PORT 覆盖。
//
// 核心设计：每个 opencode 进程 = 1 个灯泡。
// 用 "pid:<进程号>" 作为 session_id，保证每个终端恰好一个灯泡，
// 不受 subagent、历史 session、idle 状态等问题的干扰。
//
// 状态映射（按优先级聚合）:
//   任一 session busy                      -> running  -> 红灯
//   任一 session 有挂起权限/提问            -> input    -> 黄灯
//   否则                                    -> done     -> 绿灯

import type { Plugin } from "@opencode-ai/plugin";

function monitorUrl(): string {
  const port = process.env.OPENCODE_TL_PORT ?? "9912";
  return `http://127.0.0.1:${port}`;
}

/** 从事件 properties 中尽量提取 sessionID（兼容多种事件结构） */
function extractSessionID(properties: any): string | undefined {
  if (!properties) return undefined;
  return properties.sessionID ?? properties.sessionId ?? properties.info?.id ?? properties.info?.sessionID;
}

export default (async () => {
  // 用 PID 作为灯泡的唯一标识——每个 opencode 进程恰好一个灯泡
  const PID_KEY = `pid:${process.pid}`;

  // 每个 session 当前是否有挂起的权限请求/提问
  const pendingInput = new Map<string, number>();
  // 缓存 session 标题 / 项目路径 / 状态
  const sessionTitles = new Map<string, string>();
  const sessionProjects = new Map<string, string>();
  const sessionStates = new Map<string, "running" | "done" | "input">();
  // 记录已知的 subagent session（有 parentID），不计入聚合
  const subagentSessions = new Set<string>();

  /** 聚合所有非 subagent session 的状态，推送一个灯泡 */
  async function pushOverall(): Promise<void> {
    // 优先级: running > input > done
    let state: "running" | "done" | "input" = "done";
    let bestSid: string | undefined;

    for (const [sid, s] of sessionStates) {
      if (subagentSessions.has(sid)) continue;
      if (s === "running") {
        state = "running";
        bestSid = sid;
        break;
      }
      if (s === "input" && state !== "running") {
        state = "input";
        bestSid = sid;
      }
      if (state === "done" && !bestSid) {
        bestSid = sid;
      }
    }

    const title = bestSid ? sessionTitles.get(bestSid) : undefined;
    const project = bestSid ? sessionProjects.get(bestSid) : undefined;

    try {
      await fetch(`${monitorUrl()}/status`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ session_id: PID_KEY, state, project, title }),
      });
    } catch {
      // 监控器没开就静默忽略
    }
  }

  // 启动时立即注册本进程（绿灯），让监控器马上看到灯泡
  pushOverall();

  // 每 5 秒重推聚合状态，保持灯泡存活 + 监控器重启后自动恢复
  const refreshTimer = setInterval(() => {
    pushOverall();
  }, 5000);

  return {
    event: async ({ event }) => {
      const et = event.type;

      // --- 权限请求：opencode 需要用户确认 ---
      if (et === "permission.updated" || et === "permission.asked" || et === "permission.requested") {
        const sid = extractSessionID(event.properties);
        if (!sid || subagentSessions.has(sid)) return;
        const cur = (pendingInput.get(sid) ?? 0) + 1;
        pendingInput.set(sid, cur);
        sessionStates.set(sid, "input");
        await pushOverall();
        return;
      }

      // --- v2 新版提问机制 ---
      if (et === "question.asked" || et === "permission.v2.asked") {
        const sid = extractSessionID(event.properties);
        if (!sid || subagentSessions.has(sid)) return;
        const cur = (pendingInput.get(sid) ?? 0) + 1;
        pendingInput.set(sid, cur);
        sessionStates.set(sid, "input");
        await pushOverall();
        return;
      }

      // --- 权限已回复 / 提问已回答 ---
      if (et === "permission.replied" || et === "question.replied" || et === "question.rejected" || et === "permission.v2.replied") {
        const sid = extractSessionID(event.properties);
        if (!sid) return;
        const cur = (pendingInput.get(sid) ?? 1) - 1;
        pendingInput.set(sid, Math.max(0, cur));
        // 不主动改状态，等后续 session.status 事件纠正
        return;
      }

      // --- session 状态变化 ---
      if (et === "session.status") {
        const { sessionID, status } = event.properties ?? {};
        if (!sessionID) return;
        if (subagentSessions.has(sessionID)) return;
        if (status.type === "busy") {
          sessionStates.set(sessionID, "running");
        } else if (status.type === "idle") {
          if ((pendingInput.get(sessionID) ?? 0) > 0) {
            sessionStates.set(sessionID, "input");
          } else {
            sessionStates.set(sessionID, "done");
          }
        }
        await pushOverall();
        return;
      }

      // --- session 创建/更新：获取标题 ---
      if (et === "session.created" || et === "session.updated") {
        const info = event.properties?.info;
        const sid = info?.id;
        if (!sid) return;
        // subagent（有 parentID）不计入聚合
        if (info.parentID) {
          subagentSessions.add(sid);
          return;
        }
        if (info.title) {
          sessionTitles.set(sid, info.title);
        }
        if (info.directory) {
          sessionProjects.set(sid, info.directory);
        }
        // 如果还没有状态，默认 done
        if (!sessionStates.has(sid)) {
          sessionStates.set(sid, "done");
        }
        await pushOverall();
        return;
      }

      // --- session 删除 ---
      if (et === "session.deleted") {
        const sid = event.properties?.info?.id;
        if (!sid) return;
        pendingInput.delete(sid);
        sessionTitles.delete(sid);
        sessionProjects.delete(sid);
        sessionStates.delete(sid);
        subagentSessions.delete(sid);
        // 不发 /remove——进程还活着，灯泡保留，重新聚合即可
        await pushOverall();
        return;
      }
    },
    dispose: async () => {
      clearInterval(refreshTimer);
      // 进程退出时主动移除灯泡
      try {
        await fetch(`${monitorUrl()}/remove`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ session_id: PID_KEY }),
        });
      } catch {}
    },
  };
}) satisfies Plugin;
