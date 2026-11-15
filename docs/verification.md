# GPUI migration verification

Verified on Windows with Rust 1.100.0, GPUI Kit 0.9.1 and its pinned GPUI snapshot 0.3.12. Native captures were made at 150% display scaling from the actual application window. The controlled workspace is under `.local/gpui-native`; it does not use the user's normal application data.

## Build and automated checks

- `cargo check --locked`: passed.
- `cargo test --locked`: 14 tests passed, none failed.
- `cargo fmt --check`: passed.
- `cargo clippy --locked --all-targets --no-deps`: completes; remaining warnings are style suggestions, including collapsible `if` statements. The dirty-flag sweep intentionally consumes every terminal flag and must not be changed into a short-circuiting `any` operation.

The tests cover exact per-tab resume arguments, ANSI and UTF8 parsing, scrollback snapshots, atomic persistence, compatibility with pre-migration workspace records, lazy history loading, page selection, preserving project files, valid UTF8 syntax-highlight ranges, control key encoding, and real PTY input/resize/exit.

The PTY regression test also requires the first output to notify an event-driven frontend, including when a resize occurs before the first output. Windows tests submit Unicode through the real PTY and verify the combined output rather than relying on echoed input.

## Native interaction evidence

| Scenario | Result | Evidence |
| --- | --- | --- |
| Open saved terminal without launching its process | Historical output displayed with a dormant indicator and restore action | [Restored terminal](screenshots/restored.png) |
| Switch to a non-terminal page | Real project overview, directory and live terminal-count cards render | [Overview](screenshots/overview.png) |
| Ctrl+click `src/main.rs:12` | Opens real source asynchronously with syntax highlighting and line 12 highlighted | [Code preview](screenshots/code-preview.png) |
| Ctrl+click an existing PNG path | Opens the actual local image in a fitted preview | [Image preview](screenshots/image-preview.png) |
| Restore PowerShell and submit a command | Actual output contains `NATIVE_PTY_OK` and `中文输入测试` | [Live terminal](screenshots/live-terminal.png) |
| Close the native window | App process, PowerShell and the associated console descendants exit | Process IDs queried before and after close |
| Reopen the same data directory | Prior output is visible; no PowerShell child process starts until requested | [Restored terminal](screenshots/restored.png) plus process inspection |

The initial history used for path-click testing is an explicit fixture. The live-terminal screenshot and restored command output came from a real PowerShell process. The image preview displays an actual previously captured local PNG.

Input was driven through Win32 messages into the application's real input and mouse handlers. This verifies Unicode commit and command submission, not a manual end-user IME composition session. GPUI handles keyboard-down messages in its message pump, so a keyboard test must queue them with `PostMessageW`; sending them directly to the window procedure bypasses that handling. Unicode character messages must use `SendMessageW`, not the ANSI variant.

Window capture must use a DPI-aware thread before querying window bounds. A DPI-unaware capture process produces a cropped image even when the application layout is correct.

## Remaining verification

- macOS build and native behavior, including process-tree cleanup and Finder launch environment.
- Actual Chinese/Japanese IME candidate windows and marked-text editing.
- Real authenticated Claude Code, Codex and Pi conversations across quit/resume, with the target CLI versions.
- Release-build memory, CPU, startup time and sustained multi-terminal output workloads.
- Code signing, macOS notarization and distribution packaging.

The application currently provides terminal management, a native project overview and local file/image previews. No model API, streaming chat backend or AI tool-call engine is connected by this migration.

## 桌面端 MVP 验收

自动化能覆盖参数构造、PTY 与进程清理，真实 AI 会话恢复需要本机手动确认一次（Claude Code / Codex 需要已登录的账号）：

| 步骤 | 期望 |
| --- | --- |
| 设置页 Agent 分类 | 列出本机已安装 Agent 与可执行文件路径；点「已隐藏」后该项不再出现在 ＋ 菜单 |
| 新建 Claude Code 标签并对话一轮，关闭应用后重开 | 启动即自动恢复：参数含 `--resume <首次的 session-id>`，会话内能看到上一轮内容 |
| 新建 Codex 标签完成一轮（notify 回调写入 thread-id），关闭应用后重开 | 启动即自动恢复，参数为 `resume <thread-id>`，能继续原对话；ID 缺失时终端内直接报错 |
| 新建 Pi 标签对话一轮后关闭重开 | 启动即自动恢复，参数为 `--session <同一 jsonl 路径>`，Pi 恢复原会话 |
| 强杀应用进程（任务管理器结束进程） | 关联 CLI 与 `conhost.exe` 一并退出，无孤儿进程 |
| `Ctrl + Tab` / `Ctrl + 1…9` / `Ctrl + /` | 切换标签、跳标签、打开设置里的快捷键页 |
| 标签栏「内容」（Pi / Codex） | 切到对话记录页，显示来源文件与消息条数；`Esc` 返回终端 |

已自动化的部分：

- `cargo test --locked`：恢复参数构造（Claude / Codex / Pi）、PTY 行为、持久化、Web 服务、静态页面结构。
- `cargo test --locked --test transcript_real -- --ignored`：解析本机真实 Codex rollout 文件（最新一份，实测 3 条消息）。
- `cargo test --locked --test pi_scroll -- --ignored`：用预置 jsonl 启动真实 Pi，验证渲染与滚轮转发（需要隔离的 `PI_CODING_AGENT_DIR`）。
- 孤儿进程：以父进程强杀实测，`cmd.exe` 与 `conhost.exe` 随 Job 句柄关闭退出。
