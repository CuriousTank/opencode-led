// opencode-traffic-light 状态推送插件
//
// 把 opencode 的 session 状态实时推送到本机运行的 Rust 监控器。
// 监控器默认监听 127.0.0.1:9912，可用环境变量 OPENCODE_TL_PORT 覆盖。
//
// 安装：把本文件放到任一位置，opencode 会自动发现：
//   - 项目级: <project>/.opencode/plugin/status-pusher.ts
//   - 全局级: ~/.config/opencode/plugin/status-pusher.ts
//
// 状态映射:
//   session.status = busy                  -> running  -> 红灯
//   session.status = idle (无挂起权限)     -> done     -> 绿灯
//   permission.updated (权限请求挂起)      -> input    -> 黄灯

import type { Plugin } from "@opencode-ai/plugin";

function monitorUrl(): string {
  const port = process.env.OPENCODE_TL_PORT ?? "9912";
  return `http://127.0.0.1:${port}`;
}

async function heartbeat(sessionID: string): Promise<void> {
  try {
    await fetch(`${monitorUrl()}/heartbeat`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ session_id: sessionID }),
    });
  } catch {}
}

/** 从事件 properties 中尽量提取 sessionID（兼容多种事件结构） */
function extractSessionID(properties: any): string | undefined {
  if (!properties) return undefined;
  return properties.sessionID ?? properties.sessionId ?? properties.info?.id ?? properties.info?.sessionID;
}

export default (async (input) => {
  const { client } = input;

  // 记录每个 session 当前是否有挂起的权限请求/提问
  const pendingInput = new Map<string, number>();
  // 记录所有活跃的 sessionID（用于心跳）
  const knownSessions = new Set<string>();
  // 缓存 session 标题（从 session.created / session.updated 事件获取）
  const sessionTitles = new Map<string, string>();
  // 缓存 session 最后已知状态，用于 title 更新时重新推送
  const sessionStates = new Map<string, "running" | "done" | "input">();

  async function push(sessionID: string, state: "running" | "done" | "input", project?: string): Promise<void> {
    sessionStates.set(sessionID, state);
    const title = sessionTitles.get(sessionID);
    try {
      await fetch(`${monitorUrl()}/status`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ session_id: sessionID, state, project, title }),
      });
    } catch {
      // 监控器没开就静默忽略，避免污染 opencode 日志
    }
  }

  /** 仅更新 title（不改变灯泡颜色），用缓存中最后已知的状态重新推送 */
  async function pushTitleUpdate(sessionID: string, project?: string): Promise<void> {
    const state = sessionStates.get(sessionID);
    if (!state) return;
    await push(sessionID, state, project);
  }

  // === 启动时主动报到：把当前已有的活跃 session 推送给监控器 ===
  // 这样监控器晚启动时也能立刻看到已存在的会话（绿灯/红灯）
  try {
    const [listRes, statusRes] = await Promise.all([
      client.session.list(),
      client.session.status(),
    ]);
    const sessions = (listRes as any).data ?? [];
    const statuses = (statusRes as any).data ?? {};
    // 构建 id -> session 的查找表，用于补充 title / directory
    const sessionMap = new Map<string, any>();
    for (const s of sessions) {
      sessionMap.set(s.id, s);
    }
    // 只推送 statuses 中出现的 session（活跃的），避免历史 session 闪现
    for (const [sid, st] of Object.entries(statuses)) {
      knownSessions.add(sid);
      const s = sessionMap.get(sid);
      if (s?.title) sessionTitles.set(sid, s.title);
      const state: "running" | "done" | "input" =
        (st as any)?.type === "busy" || (st as any)?.type === "retry" ? "running" : "done";
      await push(sid, state, s?.directory);
    }
  } catch {
    // client 调用失败不影响后续事件监听
  }

  // 每 5 秒发送一次心跳，让监控器知道本 opencode 进程还活着。
  // 进程退出后 setInterval 自然停止，监控器在 ~12 秒后自动清理对应灯泡。
  const heartbeatTimer = setInterval(() => {
    for (const sid of knownSessions) {
      heartbeat(sid);
    }
  }, 5000);

  return {
    event: async ({ event }) => {
      const et = event.type;

      // --- 权限请求：opencode 需要用户确认（执行命令、编辑文件等）---
      if (et === "permission.updated" || et === "permission.asked" || et === "permission.requested") {
        const sid = extractSessionID(event.properties);
        if (!sid) return;
        knownSessions.add(sid);
        const cur = (pendingInput.get(sid) ?? 0) + 1;
        pendingInput.set(sid, cur);
        await push(sid, "input");
        return;
      }

      // --- v2 新版提问机制：opencode 向用户提出选择/填空 ---
      if (et === "question.asked" || et === "permission.v2.asked") {
        const sid = extractSessionID(event.properties);
        if (!sid) return;
        knownSessions.add(sid);
        const cur = (pendingInput.get(sid) ?? 0) + 1;
        pendingInput.set(sid, cur);
        await push(sid, "input");
        return;
      }

      // --- 权限已回复 / 提问已回答 ---
      if (et === "permission.replied" || et === "question.replied" || et === "question.rejected" || et === "permission.v2.replied") {
        const sid = extractSessionID(event.properties);
        if (!sid) return;
        const cur = (pendingInput.get(sid) ?? 1) - 1;
        pendingInput.set(sid, Math.max(0, cur));
        // 不主动改灯，等后续 session.status 事件纠正
        return;
      }

      // --- session 状态变化 ---
      if (et === "session.status") {
        const { sessionID, status } = event.properties ?? {};
        if (!sessionID) return;
        knownSessions.add(sessionID);
        if (status.type === "busy") {
          await push(sessionID, "running");
        } else if (status.type === "idle") {
          // idle 时若有挂起权限/提问 -> 黄灯；否则绿灯
          if ((pendingInput.get(sessionID) ?? 0) > 0) {
            await push(sessionID, "input");
          } else {
            await push(sessionID, "done");
          }
        }
        return;
      }

      // --- session 创建/更新：获取标题 ---
      if (et === "session.created" || et === "session.updated") {
        const info = event.properties?.info;
        const sid = info?.id;
        if (!sid) return;
        knownSessions.add(sid);
        if (info.title) {
          const prev = sessionTitles.get(sid);
          sessionTitles.set(sid, info.title);
          // title 变了就推一次（保持灯泡颜色不变）
          if (prev !== info.title) {
            await pushTitleUpdate(sid, info.directory);
          }
        }
        return;
      }

      // --- session 删除 ---
      if (et === "session.deleted") {
        const sid = event.properties?.info?.id;
        if (!sid) return;
        pendingInput.delete(sid);
        knownSessions.delete(sid);
        sessionTitles.delete(sid);
        sessionStates.delete(sid);
        try {
          await fetch(`${monitorUrl()}/remove`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ session_id: sid }),
          });
        } catch {}
        return;
      }
    },
    dispose: async () => {
      clearInterval(heartbeatTimer);
    },
  };
}) satisfies Plugin;
