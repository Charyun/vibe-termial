# Agent activity in terminal tabs

The terminal tab and navigation row share the same 14px activity indicator: tabs place it before the Agent icon, while navigation rows place it at the right after the flexible session name so Agent icons and names remain aligned. It rotates during a turn and becomes a green dot for an unacknowledged completion. Selecting either surface acknowledges the same completion. Idle tabs and navigation rows render no status element or reserved spacing. Selecting the corresponding terminal tab clears its completion, including keyboard tab selection. Selecting a tab during work leaves its spinner running. Protocol reports of a permission/question wait show amber; reported failures show red instead of a successful green. Those result notices are also acknowledged by tab selection.

Activity belongs to the terminal process buffer, independently of the selected workspace, file tab or conversation presentation. Completed notices survive unrelated output and repeated `done` reports. A new turn clears the previous result. History replay cannot issue terminal replies or generate completion notices.

## Runtime lifetime

Opening an unloaded tab starts/restores its CLI once. Once opened, the runtime and terminal buffer remain resident across tab and workspace switches, including its scrollback offset and exited state. Selection never restarts an already loaded runtime. Creating a new session or explicitly restarting replaces the runtime.

Ctrl+C is forwarded to the Agent. If it only interrupts a turn, the Agent stays attached. After an interrupted local Agent process actually exits, the same tab starts the default shell (CMD on Windows). Its title and profile change together, so both navigation and terminal tabs show CMD and lose stale Agent indicators. The shell uses a fresh VT parser to avoid inheriting Agent mouse, alternate-screen, cursor or paste modes. The old Agent binding and ANSI history are first saved under `sessions/exited-agents/<tab-id>-<runtime-id>.json` and `.ansi`; an archival failure retains the original tab. Failed restore attempts do not trigger this shell return. Remote SSH sessions are not replaced with a local shell.

## State sources

Collapsed workspace headings summarize their child sessions immediately before the workspace settings button. Working takes precedence over other states; with no active work, unread completion is shown in green (existing blocked/error states retain their colors and priority). Expanding the workspace hides the heading indicator immediately and leaves each session's own state and unread acknowledgment unchanged.

- OSC 7501 program status, including its capability query, is handled directly by the VT parser. Root state and live child state are folded together so a parent finishing while its children are working does not prematurely show completion.
- The currently installed Windows ConPTY consumes unknown OSC 7501 sequences. Managed local Codex therefore also reports completion through its existing headless `notify` helper. Each launch gets a unique activity file tied to its runtime UUID. A previous process's late completion cannot mark the replacement process complete. Normal Codex prompt submissions and explicit conversation sends start the busy indicator; empty prompts and slash commands are excluded.
- Local Pi and Claude session logs, and older Codex logs where available, are tailed off the UI thread with independent bounded cursors. Initial history only establishes the reader baseline; none of its intermediate or final phases is published as live activity. This also applies when a large history requires multiple polling batches, and it preserves any live status received during seeding. A newly appended task-start still publishes working even if an unfinished historical task left the reader baseline working. User/task-start, assistant stop reasons, aborts and completion records drive transitions. Output silence is never treated as completion.

Unknown Agents require an explicit status producer; process liveness alone is insufficient to infer turn completion. Native protocol status can also be consumed for remote terminals if the transport preserves those sequences. Local log fallbacks do not read remote workspaces.

## Verification

```text
cargo test --offline --locked --lib --test activity --test activity_notify --test agent_exit --test restore --test session -- --test-threads=1
cargo test --offline --locked --bin vibe-terminal native_agent_tab_spins_completes_and_clears_on_click -- --ignored --test-threads=1
```

The integration fixture runs actual PTYs and appends real-shaped Pi session records, exercising native status when transported and the log fallback on Windows. The UI test uses the product assets and window options, verifies a background completion, dispatches native mouse messages to both navigation and tabs, and checks acknowledgment. It injects Ctrl+C at the window input-dispatch layer, verifies that a running Agent survives the first interrupt, then checks actual process exit, CMD execution, both surfaces returning to CMD and buffer reuse after switching. This input path also works without foreground activation on a locked or background desktop. Set `VIBE_NATIVE_CAPTURE_DIR` to generate status and returned-CMD PNGs through `scripts/capture-window.ps1`. No model requests are needed.

Reference: [Orca Agent status integrations](https://github.com/stablyai/orca/blob/main/src/main/agent-hooks/README.md).
