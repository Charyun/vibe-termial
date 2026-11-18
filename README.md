# VibeTerm

使用 **Rust + GPUI + GPUI Kit** 的 AI Coding 桌面终端管理器，面向 Windows 和 macOS。界面已从 Rune 迁移到 GPUI Kit 0.9.1，现有工作区、标签和历史数据保持兼容。

左侧管理本地项目，右侧直接展示当前项目的多标签终端。点击终端中的文件路径，可以打开右侧源码或图片预览；Windows 还支持在右侧打开 HTTP/HTTPS 网页。

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

## SSH 远程工程

1. 打开左下角“设置”→“SSH 主机”→“添加主机”，填写显示名称和主机地址。地址可以是 IP、域名或现有 SSH 配置的 Host 别名；用户名、端口、私钥路径和独立 SSH 配置文件均可选，留空沿用 OpenSSH 默认配置。
2. 点击项目导航的“＋”，选择“SSH 远端”。下方会展示已保存的主机，选择机器后填写远程项目目录（如 `/home/user/project`、`~/project` 或 `~`），点击“连接并创建工作区”。
3. 应用在该目录打开 SSH Shell。首次主机确认、密码及私钥口令直接在终端中输入。新建标签的菜单也提供远端的 Claude、Codex、Pi 和 Gemini 命令，这些程序须已安装在远端 PATH 中。

需要本机可运行 `ssh`，远端使用 Linux/macOS 的 POSIX Shell。配置和工作区会保存，重开后可重新连接；移除工作区保留远程文件。仍被工作区使用的主机不能直接移除。设置中的“测试连接”采用非交互认证，适合验证密钥或 SSH Agent；密码认证可直接创建工作区，在终端完成。

当前 SSH 工作区的文件、Git 和 Agent 会话操作在远程终端内进行；尚未同步远端文件树、内容页记录和本机图片，也不自动建立端口转发。Pi/Claude 使用独立的远端会话标识；远端 Codex 未同步到会话 ID 时会跳过恢复，不创建替代 Shell。

SSH 测试：`cargo test --locked --no-default-features --test ssh`。显式执行 `real_ssh_transport_runs_in_remote_project_and_preserves_metadata -- --ignored --nocapture --test-threads=1` 可验证回环 SSH 的密钥/密码认证及真实远端命令执行；该测试另需 Python、Paramiko、OpenSSH 和 POSIX sh，配置与测试密钥均位于隔离临时目录，不修改用户 SSH 配置。

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

- 自定义标题栏将 Logo、产品名与 40px 终端标签栏合并。右上角依次为右侧面板折叠、最小化、最大化/还原和关闭；标题栏空白区域支持拖动窗口。
- 224px 左侧项目树显示各项目的终端会话。侧栏、顶部标签和新建终端菜单共用本地 Agent 图标资源。
- 右侧面板提供“资源管理器”和“源代码控制”页签，按需读取真实项目目录、Git 暂存/工作区改动及只读差异。点击文件继续使用原有代码、图片或网页预览；折叠面板保留当前预览和未保存的编辑。

- 创建、切换、重命名和移除工作区。每个工作区对应一个本地项目目录；移除记录不会删除项目文件。
- 每个项目维护自己的终端标签。点击标签栏的 ＋ 会弹出列表：CMD 与 PowerShell 固定排在最前，其后是本机自动检测到的 Agent（Claude Code、Codex、Pi、Gemini 等），选择后在工作区目录下启动。
- Agent 标签优先显示各自会话已有的介绍：Codex 的 `thread_name`、Pi 的会话名称，以及 Claude 记录中的标题或摘要；尚未生成介绍时，使用首条真实提问的简短内容。后台约每 3 秒增量读取并更新标题，手动重命名后保留用户名称；新会话清除旧摘要。Shell 标签保持原名称。
- 关闭应用时完全退出并结束终端进程。重开后自动恢复上次选中的会话（用原会话参数续接）；切换到休眠标签时也会自动恢复，不需要额外按钮。
- 终端支持 ANSI 颜色、光标、备用屏幕、文本选择、复制粘贴、滚动和窗口尺寸变化。
- Pi 等开启鼠标交互的 TUI 支持左键点击按钮、拖动滚动条；悬停在滚动条区域时显示并加粗滑块，移开后按 TUI 的延时隐藏。终端区域使用箭头指针。Windows 下 Pi、Claude Code、Codex 的鼠标事件也会透过 ConPTY 转发。按住 Shift 拖动可使用客户端的文字选择，Ctrl 点击文件路径仍打开预览。
- 有效文件路径和 HTTP/HTTPS 网址显示下划线，支持文件行列号及软换行后的链接。
- Windows 用 Ctrl+左键打开路径或网页；macOS 用 ⌘+左键打开路径。
- Windows 网页预览使用 WebView2，支持 localhost、JavaScript、后退、前进、刷新、复制地址、外部浏览器打开及独立的开发者工具窗口。Debug 和 Release 均保留“开发者工具”按钮。需要安装 Microsoft Edge WebView2 Runtime；初始化失败时可刷新重试或使用外部浏览器。
- 网页登录状态和缓存保存在应用数据目录的 `webview/` 中。关闭预览会释放网页实例；打开设置、对话框或新建终端菜单时隐藏原生网页，返回后恢复。macOS 的内嵌网页尚未接入，可使用“浏览器打开”。
- 源码预览支持 UTF8、UTF16 LE、语法高亮、行号、目标行高亮、复制及横向滚动。只为可见行创建界面元素，文本文件上限 2 MB。
- 本地图片可从终端中的图片路径打开，支持适配面板显示和复制路径。支持常见 PNG、JPEG、WebP、GIF、BMP 文件，限制为 16 MB 和 1600 万像素。
- 可拖动右侧分隔栏调整预览宽度。
- “内容”是同一 Agent 会话的聊天视图，支持输入、发送、停止、Markdown 消息和自动更新；切换回终端继续使用同一个进程。

网页预览的真实窗口验收需显式运行（会打开隔离测试窗口和开发者工具，使用本地 HTTP 测试页验证 JavaScript 执行，再自动关闭）：

```bash
cargo test --locked --bin vibe-terminal native_web_preview_loads_javascript -- --ignored --nocapture --test-threads=1
```

双击标签可重命名。工作区条目右侧“···”打开重命名和移除入口。

## 快捷键

终端获得焦点时，Tab、Shift+Tab、Esc、功能键和 Ctrl 组合键优先发送给 Shell / Agent；例如 Ctrl+W 删除前一个单词，Ctrl+C 中断，Ctrl+V 由终端程序处理。普通文本及中文输入沿用系统输入法。以下界面快捷键仅在终端输入区之外生效。

| 快捷键 | 作用 |
| --- | --- |
| `Ctrl + T` | 新建终端（默认 CMD） |
| `Ctrl + W` | 关闭当前标签 |
| `Ctrl + Tab` / `Ctrl + Shift + Tab` | 切换到下一个 / 上一个标签 |
| `Ctrl + 1` … `Ctrl + 9` | 跳到第 N 个标签 |
| `Ctrl + ,` | 打开设置 |
| `Ctrl + Shift + C` | 复制选中的输出 |
| `Ctrl + V` | 粘贴到终端 |
| `Ctrl + 左键` | 打开文件、图片或网页预览（macOS 用 ⌘，暂只支持文件和图片） |
| `Ctrl + /` 或 `F1` | 打开设置里的快捷键页 |
| `Esc` | 关闭对话框、菜单或设置页 |

终端选中文本时，Ctrl+C 仍发送终端中断信号。
## 界面与预览

工作区直接展示终端标签；没有标签时显示新建终端入口。右侧预览区支持源码、图片和网页。Agent 的模型调用和工具执行由原 CLI 完成，客户端的终端与内容视图共用它的会话。

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

## 内容会话视图

标签栏的会话按钮在“终端”和“内容”之间切换，两者对应同一个 Agent 进程和会话。内容视图支持 Markdown 消息列表、中文与多行输入、发送、停止和恢复会话。Enter 发送，Shift + Enter 换行；使用“返回终端”可继续直接操作原 CLI。

每个标签独立保留视图和输入草稿；切换标签后不会显示其他会话的消息。消息在后台约每 250ms 增量读取，写入中的不完整 JSONL 行会等待补全。新的消息默认跟随到底部，向上阅读时保留滚动位置，也可点击“最新消息”。提交后的临时消息会与真实记录合并，避免重复显示。

输入通过原 PTY 提交，正文和回车分开发送。延迟回车绑定原进程，重启或关闭会话后不会误发给替换进程。Pi 的已有单行草稿可带入内容输入框；发送会替换旧单行输入，避免拼接。终端里已有多行草稿时，先在终端处理再发送。登录、权限确认及交互式选择继续使用终端。

回复的显示节奏以 Agent 写入会话记录为准；当前尚未接入 Orca 的完整 hook token 推送、授权卡片和附件系统。

| CLI | 数据来源 |
| --- | --- |
| Pi | 应用为每个标签传入的 `--session <data>/sessions/<tab>.jsonl` |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`，按该标签记录的 `thread-id` 取最新一份 |
| Claude Code | `~/.claude/projects/` 中按当前标签会话 ID 定位的 JSONL；未保存结构化记录时不可用 |

会话文件或 Codex 的 thread-id 尚未就绪时，也可以进入内容视图并向原进程发送消息；已绑定的记录就绪后自动加载，不猜测其他标签或全局最近会话。显示用户、助手、工具调用/结果和系统文本，跳过内部上下文与 token 统计。实现参考 [Orca 的 NativeChatBridgeView](https://github.com/stablyai/orca/blob/main/src/renderer/src/components/native-chat/NativeChatView.tsx) 与 [原 PTY 提交逻辑](https://github.com/stablyai/orca/blob/main/src/renderer/src/components/native-chat/session/terminal/send/native-chat-runtime-send.ts)。

回归测试：`cargo test --locked --test conversation`；原生窗口输入验证：`cargo test --locked --bin vibe-terminal native_content_input_reaches_same_pty_and_refreshes -- --ignored --nocapture`。后者使用隔离的本地回显 Agent，不发送模型请求。

## 设置与 Agent 检测

设置页包含 **General**、**Agent**、**快捷键** 与 **Remote**。

- **General**：输入光标样式，可选“小方块”或“竖线”，默认小方块；立即生效并自动保存，重启后保留选择。
- **快捷键**：查看终端与工作区快捷键。

- **Agent**：自动扫描 `PATH` 与常见安装目录（`%APPDATA%\npm`、`%LOCALAPPDATA%\Programs`、`~/.pi/agent/bin`、`~/.local/bin` 等），列出本机已安装的 Agent 与可执行文件路径，并用「已启用 / 已隐藏」控制它是否出现在 ＋ 菜单。CMD 与 PowerShell 是内置终端，固定排在菜单最前，不在此列，也不需要手工配置。
- **Remote**：Web 访问开关；开启后直接显示局域网链接、Tailscale 链接（取到本机 Tailscale IPv4 时显示，否则可点“重新获取”）和访问令牌，每项都可一键复制。

内置识别 Claude Code、Codex、Pi、Gemini CLI、OpenCode、Aider、Qwen Code、Cursor Agent、Amp、GitHub Copilot CLI、Goose、Crush；新增一个 Agent 只需在 `src/agents.rs` 的 `KNOWN` 里补充一行。

Windows 标准 npm `.cmd` 启动器会解析为 Node 和 JS 入口直接启动，保留参数中的引号；其他 `.cmd` / `.bat` 需要配置实际 exe。macOS 从 Finder 启动时 PATH 可能与登录终端不同，把可执行文件所在目录加入 PATH 即可被检测到。

## 数据与资源控制

数据位于系统用户数据目录的 `VibeTerminal` 中，可通过 `VIBE_TERMINAL_DATA_DIR` 指定独立目录。工作区、历史和 AI 会话文件都是本地明文数据。

- `state.json`：工作区、当前页面、标签、启动配置、目录、会话 ID 和终端尺寸。
- `history/<tab-id>.ansi`：限量历史及屏幕快照。
- `sessions/`：Pi 会话文件和 Codex ID 标记。
- `failed-restores/<tab-id>.json`：恢复失败后跳过的标签资料；其原始终端历史继续保留，不自动创建替代终端。
- `instance.lock`：防止多个实例同时覆盖同一份数据。

使用原子替换保存；配置损坏时保留原文件并报告错误。每 10 秒及退出时保存已加载终端，异常退出可能丢失最近一个保存周期的输出。旧版记录中的 `page` 字段会被忽略，已有工作区、标签和历史继续保留。

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

Windows 已验证：GPUI 原生窗口、文件路径点击和行高亮、本地图片预览、真实 PowerShell 输入输出、Unicode 文本提交、关闭时进程退出，以及重开后不自动启动终端。

macOS 的编译和原生交互仍待真机验证。IME 接口使用真实终端光标作为候选框锚点，即使 Pi 隐藏硬件光标也保留输入位置；Windows 原生 IMM 接口已在 150% 缩放下验证坐标。Windows 下由客户端启动的 Pi 进程还会启用 `PI_HARDWARE_CURSOR=1`，让 ConPTY 保留编辑光标位置，避免输入法锚点停在重绘后的行尾；不修改全局 Pi 配置。真实拼音/日文候选窗的完整组合输入、组合键和跨屏 DPI 场景仍需要人工验收。VT 支持尚未覆盖 Kitty 键盘协议、图形协议及所有终端鼠标协议；已支持向 TUI 转发滚轮、左键按下、拖动和松开，未声明鼠标模式的普通终端仍使用本地文字选择。备用屏幕本身不会触发方向键模拟：识别到的 Pi/Codex/Claude 使用原生鼠标滚轮协议（包括 CMD/PowerShell 内启动的 Agent），其他情况只滚动本地历史，不会把滚轮变成输入框的上下方向键。

已安装 Pi 时，可在隔离的 `PI_CODING_AGENT_DIR` 下设置 `PI_OFFLINE=1`，运行 `cargo test --locked --no-default-features --test pi_scroll -- --ignored` 验证真实 ConPTY 会话的滚动条双向拖动和 “Jump to latest message” 点击；测试使用生成的会话历史，不发送模型请求。

## 字体与许可证

界面使用 Geist 和 Geist Mono，字体许可证位于 `assets/fonts/`。GPUI 和 GPUI Kit 使用 Apache-2.0；本项目使用 MIT，各依赖遵循各自许可证。
