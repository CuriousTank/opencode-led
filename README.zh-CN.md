# opencode-traffic-light

[English](./README.md) | [简体中文](./README.zh-CN.md)

一个悬浮、置顶的「红绿灯」监控器，实时反映 [opencode](https://opencode.ai) 的任务状态。

> ⚠️ **平台**：目前仅在 **Ubuntu 20.04 (X11)** 上经过验证，暂不支持 Windows 和 macOS。

- 🔴 红：opencode 正在执行任务（`session.status = busy`）
- 🟡 黄：opencode 等待你回复/介入（权限请求挂起 `permission.updated`）
- 🟢 绿：opencode 已完成任务（`session.status = idle`）

## 功能特性

- **🔴🟡🟢 实时状态** — 红（执行中）/黄（需输入）/绿（完成），活跃状态带脉冲动画。
- **📦 动态灯泡数量** — 自动追踪 opencode 进程的创建与消亡。每个运行中的 opencode session 对应一颗灯泡，session 启动/退出时灯泡实时增减，无需手动配置。
- **🖱️ 点击置顶终端** — 点击任意灯泡，瞬间将对应的 opencode 终端窗口置顶到前台（跨工作区，通过 EWMH `_NET_ACTIVE_WINDOW` 实现）。窗口匹配基于进程树遍历（`/proc`）+ 窗口标题评分。
- **💬 悬停提示** — 鼠标悬停灯泡显示 session 标题和当前状态。提示框出现在灯泡行上方，悬停期间稳定不闪烁。
- **🪟 透明置顶** — 无边框、点击穿透（XShape 输入区域），悬浮于所有窗口之上且不阻挡操作。
- **✋ 可拖拽** — 拖动任意灯泡即可重新定位。

## 演示

### 🔴 正在思考（红灯）

![](assets/ask_2x.gif)

### 🟡 询问状态（黄灯）

![](assets/choice_2x.gif)

### 🖱️ 点击灯泡置顶终端

![](assets/pinned_window_2x.gif)

### 📦 动态灯泡追踪（session 实时增减）

![](assets/dynamic_bulbs_2x.gif)

## 架构

```
opencode 进程                    Rust 监控器进程
┌─────────────────────┐         ┌──────────────────────────┐
│ 插件 status-pusher  │  HTTP   │ tiny_http (127.0.0.1:9912)│
│  ├ event:status     │ ──POST→ │  ├ 状态机 store          │
│  └ event:permission │         │  └ eframe 悬浮置顶窗口    │
└─────────────────────┘         │     红黄绿 PNG 灯泡       │
                                └──────────────────────────┘
```

- 插件（TS，~70 行）放进 opencode 的 `.opencode/plugin/` 自动加载，捕获 `session.status` / `permission.updated` 事件后 POST 推给监控器。
- 监控器（Rust，egui/eframe 渲染）监听本地端口，渲染无边框、透明、置顶、可拖拽的窗口，支持同时显示多个 session 的灯泡。

## 安装

### 方式 A：通过 .deb 包安装（推荐）

从 [GitHub Releases](https://github.com/CuriousTank/opencode-led/releases) 下载最新的 `.deb`：

```bash
sudo dpkg -i opencode-traffic-light_0.3.0_amd64.deb
sudo apt-get install -f   # 自动补齐缺失依赖
```

### 方式 B：从源码编译（需要 Rust）

```bash
cd opencode-traffic-light
cargo build --release
# 产物：target/release/opencode-traffic-light
```

Linux 编译依赖（运行/编译时）：系统的 OpenGL 运行库（大多数发行版自带）。无需 gtk/webkit。

### 安装 opencode 插件

把 `plugin/status-pusher.ts` 放到任一位置，opencode 会自动发现加载：

- **项目级**：`<project>/.opencode/plugin/status-pusher.ts`
- **全局级**：`~/.config/opencode/plugin/status-pusher.ts`

> 插件需要 opencode 已安装 `@opencode-ai/plugin` 包（opencode plugin 机制默认提供）。

通过 `.deb` 安装的话，插件位于 `/usr/share/opencode-traffic-light/plugin/status-pusher.ts`。

## 使用

```bash
# 1. 启动监控器
opencode-traffic-light          # .deb 安装的
# 或
./target/release/opencode-traffic-light  # 源码编译的

# 2. 正常使用 opencode（在已放插件的项目里）
opencode
```

启动后会弹出一个红绿灯窗口：
- **拖动**任意灯泡移动位置
- **点击**灯泡将对应终端窗口置顶到前台
- **悬停**灯泡查看 session 标题和状态
- **右键**退出

## 配置

监控器端口默认 `9912`，用环境变量覆盖：

```bash
OPENCODE_TL_PORT=8899 opencode-traffic-light
```

插件读取同一环境变量（`OPENCODE_TL_PORT`）决定推送到哪个端口。

## 自定义图标

编辑 `tools/gen_icons.py` 顶部的颜色 RGB，然后：

```bash
python3 tools/gen_icons.py   # 重新生成 assets/*.png
cargo build --release
```

## 协议

```jsonc
// 监控器监听 127.0.0.1:9912
// 插件 → 监控器
POST /status   { "session_id": "ses_xxx", "project": "/path", "state": "running|done|input" }
POST /remove   { "session_id": "ses_xxx" }
GET  /health   -> "ok"
```

`state` 取值：`running`（红）/`input`（黄）/`done`（绿）。

## License

MIT
