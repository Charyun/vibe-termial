# Vibe Terminal

使用 **Rust + GPUI + GPUI Kit** 的 AI Coding 桌面终端管理器，面向 Windows 和 macOS。界面已从 Rune 迁移到 GPUI Kit 0.9.1，现有工作区、标签和历史数据保持兼容。

左侧管理本地项目，右侧展示当前项目的多标签终端和项目概览。点击终端中的文件路径，可以打开右侧源码或图片预览。

## 运行

需要 rustup 和本平台的编译环境：

- Windows：Visual Studio C++ Build Tools，使用 MSVC 工具链。
- macOS：Xcode、Metal 工具及 Xcode Command Line Tools，使用相应的 Apple Rust 工具链。

项目通过 `rust-toolchain.toml` 固定 Rust 1.100.0，不修改系统默认工具链。GPUI Kit 和配套 GPUI 均从 Cargo 获取，不需要下载 Rune 或维护 `.vendor` 补丁。

```bash
cargo run --locked
```

首次下载依赖可使用 `./scripts/bootstrap.ps1`（PowerShell）或 `bash scripts/bootstrap.sh`（macOS）。脚本只下载当前平台所需的依赖。

```bash
cargo build --release --locked
```

## Web 远程访问

在设置页（左下角齿轮按钮）点击“开启 Web 访问”，应用会启动内置 Web 服务，并将带访问令牌的局域网链接复制到剪贴板。在同一局域网的电脑、手机或平板浏览器打开链接，即可查看和控制当前应用中各工作区的终端会话。

- 默认端口为 `8787`，监听 IPv4 网络接口。首次访问若被 Windows 防火墙拦截，需允许应用在所需网络中接收入站连接。
- 页面提供工作区和会话列表、实时终端、直接键盘输入、中文及多行消息输入、Esc、Tab、方向键和 Ctrl+C。支持新建、恢复、停止、重新启动及关闭会话。
- 桌面和 Web 共享同一 PTY 进程。浏览器断开不会停止 AI；关闭桌面应用会停止 Web 服务和终端进程。第一版需要桌面应用保持运行。
- 移动端首屏是会话列表，点选会话后进入对应终端页面；返回按钮可回到列表。窄屏下终端按屏幕宽度自动换行，可上下滚动查看输出，输入框适配软键盘高度。
- 多个浏览器可以同时查看和输入，没有独占控制锁；多人操作时应协调输入。断线后自动重连并恢复快照，不自动重发未确认的命令。
- 令牌每次开启服务时重新生成。带令牌的链接只通过 URL 片段传递，页面读取后从地址栏移除；不会将令牌作为 HTTP 路径或查询参数发送。仅在当前标签页的会话存储中保留令牌，点击“断开连接”会清除。
- “关闭 Web 访问”会断开浏览器并释放端口，不停止桌面会话。Web 端关闭会话与桌面端一致，会结束进程并删除该标签的显示历史；操作前有确认提示。

可以通过启动前的环境变量修改监听地址，例如 PowerShell：

```powershell
$env:VIBE_TERMINAL_WEB_BIND = '0.0.0.0:8787'
cargo run --locked
```

### Tailscale

电脑与访问设备登录同一 tailnet，且访问策略允许连接该端口时，点击桌面端“复制 Tailscale 链接”，在手机浏览器打开即可。应用仅查询本机 `tailscale ip -4`，不会替你更改 Tailscale、访问策略或防火墙。也可以手动打开 `http://<电脑的 Tailscale IPv4>:8787/`，粘贴桌面端“复制访问令牌”得到的令牌。

如需 HTTPS，可以自行使用 [Tailscale Serve](https://tailscale.com/docs/reference/tailscale-cli/serve) 代理内置服务：

```powershell
tailscale serve --bg http://127.0.0.1:8787
```

随后打开该命令给出的 HTTPS 地址，并输入访问令牌。页面会自动使用 WSS。此命令会修改本机的 Serve 配置，请根据现有配置选择；本应用不自动执行。这里使用 Serve 在 tailnet 内访问，不需要配置 Funnel。

Web 静态资源及 xterm.js 5.5.0 已嵌入可执行文件，无需访问 CDN。Web 服务使用随机令牌、同源 WebSocket 检查、连接数及消息大小限制；基础局域网链接使用 HTTP，Tailscale 远程链路由 Tailscale 提供加密，也可使用上述 HTTPS 地址。

验证后端可使用 `cargo test --locked --no-default-features --features web --test web`。其中浏览器验收夹具默认跳过，只有显式指定 `VIBE_WEB_FIXTURE_FILE` 后才应单独执行 `browser_fixture -- --ignored`；夹具只操作隔离临时目录。

Windows 产物为 `target/release/vibe-terminal.exe`；macOS 为 `target/release/vibe-terminal`。在 Mac 上运行 `bash scripts/package-macos.sh` 可以生成未签名的开发应用包。代码签名、公证和自动更新尚未配置。

## 功能

- 创建、切换、重命名和移除工作区。每个工作区对应一个本地项目目录；移除记录不会删除项目文件。
- 每个项目维护自己的终端标签。点击标签栏的 ＋ 会弹出列表：CMD 与 PowerShell 固定排在最前，其后是本机自动检测到的 Agent（Claude Code、Codex、Pi、Gemini 等），选择后在工作区目录下启动。
- 关闭应用时完全退出并结束终端进程。重开后自动恢复上次选中的会话（用原会话参数续接）；切换到休眠标签时也会自动恢复，不需要额外按钮。
- 终端支持 ANSI 颜色、光标、备用屏幕、文本选择、复制粘贴、滚动和窗口尺寸变化。
- 有效文件路径显示下划线，支持绝对路径、相对路径、行列号和软换行后的路径。
- Windows 用 Ctrl+左键打开路径；macOS 用 ⌘+左键。
- 源码预览支持 UTF8、UTF16 LE、语法高亮、行号、目标行高亮、复制及横向滚动。只为可见行创建界面元素，文本文件上限 2 MB。
- 本地图片可从项目概览的“打开图片”入口或终端中的图片路径打开，支持适配面板显示和复制路径。支持常见 PNG、JPEG、WebP、GIF、BMP 文件，限制为 16 MB 和 1600 万像素。
- 可拖动右侧分隔栏调整预览宽度。
- “内容”面板读取当前标签的对话记录：Pi 用 `--session` 写入的 JSONL，Codex 用该 `thread-id` 的 rollout 文件。

双击标签可重命名。工作区条目右侧“···”打开重命名和移除入口。

## 快捷键

| 快捷键 | 作用 |
| --- | --- |
| `Ctrl + T` | 新建终端（默认 CMD） |
| `Ctrl + W` | 关闭当前标签 |
| `Ctrl + Tab` / `Ctrl + Shift + Tab` | 切换到下一个 / 上一个标签 |
| `Ctrl + 1` … `Ctrl + 9` | 跳到第 N 个标签 |
| `Ctrl + ,` | 打开设置 |
| `Ctrl + Shift + C` | 复制选中的输出 |
| `Ctrl + V` | 粘贴到终端 |
| `Ctrl + 左键` | 打开文件或图片预览（macOS 用 ⌘） |
| `Ctrl + /` 或 `F1` | 打开设置里的快捷键页 |
| `Esc` | 关闭对话框、菜单或设置页 |

Windows 未选中文本时，`Ctrl + C` 发送终端中断信号；选中文本后走复制。
## 非终端页面

“项目概览”是实际的 GPUI 原生页面，展示当前目录、终端标签数量、运行状态和继续工作入口。页面中的卡片由原生布局和控件组合，并非终端输出或网页截图。

工作区使用 `WorkspacePage` 区分内容页面，预览区区分源码和图片。后续可以扩展聊天、工具调用卡片、任务结果和文件差异页面。当前尚未接入聊天模型 API、流式 AI 回复或 Agent 工具调用后端。

GPUI Kit 提供按钮、单行输入、文本域、主题、图标资产和窗口 Root。终端使用专用的 GPUI Element 绘制网格，通过 `EntityInputHandler` 接入平台文本输入和 IME 接口。

## 会话恢复

进程退出后，正在执行的命令和进程内存不会保留。历史输出仅供查看，不会作为命令重新发送。

| CLI | 恢复方式 |
| --- | --- |
| Shell | 在保存的目录启动新的 Shell，单独保留历史输出 |
| Claude Code | 首次用 `--session-id` 绑定标签，之后用 `--resume <id>` |
| Codex | 为该标签配置 `notify` 回调，在一次 AI 回合完成后记录 `thread-id`，之后用 `resume <id>` |
| Pi | 每个标签使用独立的 `--session <path>` 文件 |
| 自定义 CLI | 按配置启动；可在参数中指定该 CLI 的会话文件或 ID |

Codex 的通知设置仅作用于该标签启动的进程，不修改全局配置；原有其他 `notify` 程序不会同时运行。尚未完成过回合、CLI 不支持该接口或上游会话文件缺失时，应用会提示错误，并允许明确选择“新会话”。不会使用 `--last` 猜测其他标签的对话。

真实登录后的 AI 对话恢复仍需在所使用的 CLI 版本上验收；普通 Shell、历史保存及按需启动已在 Windows 原生窗口中验证。

## 内容面板

标签栏右侧的“内容”按钮把终端区切换成对话记录页面，顶部显示来源文件与消息条数，可刷新或按 `Esc` 返回终端。它只读取本机已经落盘的会话文件，不调用任何模型接口。

| CLI | 数据来源 |
| --- | --- |
| Pi | 应用为每个标签传入的 `--session <data>/sessions/<tab>.jsonl` |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`，按该标签记录的 `thread-id` 取最新一份 |
| Claude Code | 暂不可用：本机只留下 `~/.claude/sessions/<pid>.json`（进程 ID、会话 ID、目录），其中没有对话内容 |

Codex 文件按 `session_meta`、`response_item`、`event_msg` 三类记录解析，只显示用户、助手、工具和系统消息的文本。会话文件缺失、为空或格式不认识时，按钮仍然可用，点击后提示原因而不显示空面板。

## 设置与 Agent 检测

设置页分两类：**Agent** 与 **Remote**。

- **Agent**：自动扫描 `PATH` 与常见安装目录（`%APPDATA%\npm`、`%LOCALAPPDATA%\Programs`、`~/.pi/agent/bin`、`~/.local/bin` 等），列出本机已安装的 Agent 与可执行文件路径，并用「已启用 / 已隐藏」控制它是否出现在 ＋ 菜单。CMD 与 PowerShell 是内置终端，固定排在菜单最前，不在此列，也不需要手工配置。
- **Remote**：Web 访问开关；开启后直接显示局域网链接、Tailscale 链接（取到本机 Tailscale IPv4 时显示，否则可点“重新获取”）和访问令牌，每项都可一键复制。

内置识别 Claude Code、Codex、Pi、Gemini CLI、OpenCode、Aider、Qwen Code、Cursor Agent、Amp、GitHub Copilot CLI、Goose、Crush；新增一个 Agent 只需在 `src/agents.rs` 的 `KNOWN` 里补充一行。

Windows 标准 npm `.cmd` 启动器会解析为 Node 和 JS 入口直接启动，保留参数中的引号；其他 `.cmd` / `.bat` 需要配置实际 exe。macOS 从 Finder 启动时 PATH 可能与登录终端不同，把可执行文件所在目录加入 PATH 即可被检测到。

## 数据与资源控制

数据位于系统用户数据目录的 `VibeTerminal` 中，可通过 `VIBE_TERMINAL_DATA_DIR` 指定独立目录。工作区、历史和 AI 会话文件都是本地明文数据。

- `state.json`：工作区、当前页面、标签、启动配置、目录、会话 ID 和终端尺寸。
- `history/<tab-id>.ansi`：限量历史及屏幕快照。
- `sessions/`：Pi 会话文件和 Codex ID 标记。
- `instance.lock`：防止多个实例同时覆盖同一份数据。

使用原子替换保存；配置损坏时保留原文件并报告错误。每 10 秒及退出时保存已加载终端，异常退出可能丢失最近一个保存周期的输出。旧版数据没有 `page` 字段时，默认恢复为终端页面。

未加载的标签不创建 PTY，只解析当前查看的休眠标签历史。每个终端最多保留 2000 行滚动历史，持久化历史约 2 MB 的完整 ANSI 行，当前屏幕另存。PTY 输出通过合并事件唤醒界面，输出刷新限制在约 30 次/秒；空闲时仅进行低频元数据检查和保存。

这些是资源控制策略，不代表发布版本的性能测量结果。发布构建的内存、CPU 和多标签持续输出负载仍需单独测量。

## 代码结构

| 文件 | 职责 |
| --- | --- |
| `src/model.rs` | 工作区、标签、CLI 配置及页面类型 |
| `src/session.rs` | 与 GUI 无关的工作区和终端生命周期 |
| `src/terminal.rs` | PTY、VT 解析、输入输出、进程树及历史快照 |
| `src/store.rs` | 数据目录、原子保存、历史读取 |
| `src/paths.rs` | 文件路径识别、行列号及文本读取 |
| `src/render.rs` | 终端网格快照、命中测试和 GPUI 绘制 |
| `src/agents.rs` | 扫描 `PATH` 与常见目录，识别本机已安装的 Agent |
| `src/transcript.rs` | 定位并解析 Pi 与 Codex 的会话文件 |
| `src/tailscale.rs` | 从 `PATH`、常见目录和注册表定位 Tailscale |
| `src/ui.rs` | GPUI Kit 界面、原生输入、异步预览和页面切换 |

## 验证

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets --no-deps
```

仅测试独立核心：

```bash
cargo test --locked --no-default-features --lib --tests
```

Windows 原生受控测试工作区可用 `./scripts/native-smoke.ps1` 启动。此脚本创建隔离的测试工作区和演示历史，不等同于真实 AI 会话。原生验收说明见 [docs/verification.md](docs/verification.md)。

## 验证范围

Windows 已验证：GPUI 原生窗口、项目概览卡片、文件路径点击和行高亮、本地图片预览、真实 PowerShell 输入输出、Unicode 文本提交、关闭时进程退出，以及重开后不自动启动终端。

macOS 的编译和原生交互仍待真机验证。IME 接口已实现，真实拼音/日文候选窗、组合键和跨屏 DPI 场景仍需要人工验收。VT 支持尚未覆盖 Kitty 键盘协议、图形协议及所有终端鼠标协议；当前由应用处理鼠标选区和路径点击，备用屏幕滚轮映射为方向键。

## 字体与许可证

界面使用 Geist 和 Geist Mono，字体许可证位于 `assets/fonts/`。GPUI 和 GPUI Kit 使用 Apache-2.0；本项目使用 MIT，各依赖遵循各自许可证。
