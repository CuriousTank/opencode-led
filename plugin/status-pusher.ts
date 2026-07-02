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

export default (async () => {
  // 记录每个 session 当前是否有挂起的权限请求
  const pendingPermission = new Map<string, number>();

  return {
    event: async ({ event }) => {
      if (event.type === "permission.updated") {
        const p = event.properties;
        const sid = p?.sessionID;
        if (!sid) return;
        // 计数：有新权限请求 +1，已响应 -1
        const cur = (pendingPermission.get(sid) ?? 0) + 1;
        pendingPermission.set(sid, Math.max(0, cur));
        await push(sid, "input");
      }

      if (event.type === "permission.replied") {
        const p = event.properties;
        const sid = p?.sessionID;
        if (!sid) return;
        const cur = (pendingPermission.get(sid) ?? 1) - 1;
        pendingPermission.set(sid, Math.max(0, cur));
        // 注意：权限响应后真实状态由后续的 session.status 事件决定，
        // 这里不主动改灯，等 session.status=idle 事件来纠正。
      }

      if (event.type === "session.status") {
        const { sessionID, status } = event.properties ?? {};
        if (!sessionID) return;
        if (status.type === "busy") {
          await push(sessionID, "running");
        } else if (status.type === "idle") {
          // idle 时若无挂起权限 -> done(绿)；否则保持 input(黄)
          if ((pendingPermission.get(sessionID) ?? 0) > 0) {
            await push(sessionID, "input");
          } else {
            await push(sessionID, "done");
          }
        }
      }

      if (event.type === "session.deleted") {
        const info = event.properties?.info;
        const sid = info?.id;
        if (!sid) return;
        pendingPermission.delete(sid);
        try {
          await fetch(`${monitorUrl()}/remove`, {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ session_id: sid }),
          });
        } catch {}
      }
    },
  };
}) satisfies Plugin;
