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

async function push(sessionID: string, state: "running" | "done" | "input", project?: string): Promise<void> {
  try {
    await fetch(`${monitorUrl()}/status`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ session_id: sessionID, state, project }),
    });
  } catch {
    // 监控器没开就静默忽略，避免污染 opencode 日志
  }
}

/** 从事件 properties 中尽量提取 sessionID（兼容多种事件结构） */
function extractSessionID(properties: any): string | undefined {
  if (!properties) return undefined;
  return properties.sessionID ?? properties.sessionId ?? properties.info?.id ?? properties.info?.sessionID;
}

export default (async () => {
  // 记录每个 session 当前是否有挂起的权限请求/提问
  const pendingInput = new Map<string, number>();

  return {
    event: async ({ event }) => {
      const et = event.type;

      // --- 权限请求：opencode 需要用户确认（执行命令、编辑文件等）---
      if (et === "permission.updated" || et === "permission.asked" || et === "permission.requested") {
        const sid = extractSessionID(event.properties);
        if (!sid) return;
        const cur = (pendingInput.get(sid) ?? 0) + 1;
        pendingInput.set(sid, cur);
        await push(sid, "input");
        return;
      }

      // --- v2 新版提问机制：opencode 向用户提出选择/填空 ---
      if (et === "question.asked" || et === "permission.v2.asked") {
        const sid = extractSessionID(event.properties);
        if (!sid) return;
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

      // --- session 删除 ---
      if (et === "session.deleted") {
        const sid = event.properties?.info?.id;
        if (!sid) return;
        pendingInput.delete(sid);
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
  };
}) satisfies Plugin;
