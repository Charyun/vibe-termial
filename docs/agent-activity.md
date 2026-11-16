# Agent activity in terminal tabs

The 14px indicator before the Agent icon rotates during a turn and becomes a green dot for an unacknowledged completion. Idle tabs render no status element or reserved spacing. Selecting the corresponding terminal tab clears its completion, including keyboard tab selection. Selecting a tab during work leaves its spinner running. Protocol reports of a permission/question wait show amber; reported failures show red instead of a successful green. Those result notices are also acknowledged by tab selection.

Activity belongs to the terminal process buffer, independently of the selected workspace, file tab or conversation presentation. Completed notices survive unrelated output and repeated `done` reports. A new turn clears the previous result. History replay cannot issue terminal replies or generate completion notices.

## State sources

- OSC 7501 program status, including its capability query, is handled directly by the VT parser. Root state and live child state are folded together so a parent finishing while its children are working does not prematurely show completion.
- The currently installed Windows ConPTY consumes unknown OSC 7501 sequences. Managed local Codex therefore also reports completion through its existing headless `notify` helper. Each launch gets a unique activity file tied to its runtime UUID. A previous process's late completion cannot mark the replacement process complete. Normal Codex prompt submissions and explicit conversation sends start the busy indicator; empty prompts and slash commands are excluded.
- Local Pi and Claude session logs, and older Codex logs where available, are tailed off the UI thread with independent bounded cursors. Initial history is seeded without notifications. User/task-start, assistant stop reasons, aborts and completion records drive transitions. Output silence is never treated as completion.

Unknown Agents require an explicit status producer; process liveness alone is insufficient to infer turn completion. Native protocol status can also be consumed for remote terminals if the transport preserves those sequences. Local log fallbacks do not read remote workspaces.

## Verification

```text
cargo test --offline --locked --lib --test activity --test activity_notify -- --test-threads=1
cargo test --offline --locked --bin vibe-terminal native_agent_tab_spins_completes_and_clears_on_click -- --ignored --test-threads=1
```

The integration fixture runs actual PTYs and appends real-shaped Pi session records, exercising native status when transported and the log fallback on Windows. The UI test uses the product assets and window options, verifies a background tab completion, physically clicks its status slot, and checks that the notice stays cleared. Set `VIBE_NATIVE_CAPTURE_DIR` to generate working/completed/acknowledged PNGs through `scripts/capture-window.ps1`. No model requests are needed.

Reference: [Orca Agent status integrations](https://github.com/stablyai/orca/blob/main/src/main/agent-hooks/README.md).
