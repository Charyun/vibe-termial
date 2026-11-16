# File previews and clipboard images

Explorer files open in the main tab strip, named by basename. Opening the same canonical file within a workspace selects its existing tab. Each file keeps its own editor, dirty flag and wrap setting. File tabs also support images. Ctrl+S saves the selected file, Ctrl+W closes it, and Ctrl+Tab / Ctrl+1–9 navigate the combined terminal/file tabs. Dirty files are guarded on close, workspace deletion and application exit; changing tabs or workspaces retains their buffers.

Ctrl/Cmd-clicking a terminal file link opens a preview inside that terminal's main content area, with the terminal on the left and the preview on the right. The divider can be dragged. Switching to a file tab hides this split; selecting the originating terminal shows it again. The resource explorer remains a separate right-hand pane.

## Image paste

Ctrl/Cmd+V inspects clipboard images before treating clipboard content as text. On Windows, native terminal paste and the conversation textarea's `on_paste` hook both enter the shared image handler. Plain text retains its normal paste behavior.

| Agent | Image transport |
| --- | --- |
| Codex | Native Ctrl+V |
| Pi, Claude Code | Native Alt+V on Windows; Ctrl+V elsewhere |
| Gemini CLI, OpenCode | Native Ctrl+V |
| Other/custom Agents and shells | Save a PNG, then paste its quoted absolute path |

Fallback images are stored under the OS temporary directory's `vibe-terminal/clipboard-images` folder. They remain available after paste so an Agent can read them when the user submits later. No route appends Enter. File preparation runs off the UI thread and binds its completion to the original terminal buffer and session generation. Closed or replaced sessions reject late results. Conversation destinations additionally carry their input entity and draft epoch.

A native image also enables an image-only conversation message. Sending remains an explicit user action. Image-only transcript blocks appear as `[图片]`; embedded base64 is not rendered as text.

SSH terminals cannot use the local OS clipboard or local temporary paths. They currently report that the image must be uploaded and referenced by its remote path.

## Verification

```text
cargo test --offline --locked --lib --test clipboard --test conversation -- --test-threads=1
cargo test --offline --locked --bin vibe-terminal native_document_tabs_preserve_editors_and_terminal_split_ownership -- --ignored --test-threads=1
cargo test --offline --locked --bin vibe-terminal native_image_clipboard_reaches_agent_or_stages_a_readable_path -- --ignored --test-threads=1
```

The native clipboard test uses Windows SendInput and a real bitmap clipboard, restores the clipboard on success or failure, and checks input received by a local Node.js PTY fixture. It verifies native keys, a readable fallback PNG, image-only submission and the conversation textarea route without model requests. Native file tests cover separate asynchronous loads, canonical deduplication, real Chinese editing, encoding/newline preservation, external-change protection, dirty-close/exit dialogs, image tabs and split ownership.

Implementation references: [Orca clipboard image saving and owner routing](https://github.com/stablyai/orca/blob/main/src/renderer/src/components/native-chat/paste/native-chat-clipboard-image-save.ts), [Codex native image key handling](https://github.com/openai/codex/blob/main/codex-rs/tui/src/chatwidget/clipboard.rs), and OpenCode's `prompt.paste` handler.
