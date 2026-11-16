# Terminal performance changes and verification

This change borrows Orca's distinction between terminal ingestion, foreground presentation, and reusable rendering resources. It keeps GPUI, portable-pty, the patched vt100 parser, terminal input protocols, and per-tab resume arguments.

## Changes

- `src/ui.rs`: Output events still drain every terminal's dirty flag. The selected terminal is captured only when its frame identity changes. Background-only output no longer notifies the native view when neither the selected frame nor running-terminal membership changed. A background process exit still updates status. The first-output wake and existing output coalescing remain in place.
- `src/render.rs`: Frame identity includes the PTY instance, revision, scrollback offset, grid size, current directory, and workspace directory. New PTY instances invalidate the cache even if revision and size happen to match. Scrollback is checked separately because returning to the live screen does not necessarily increment revision.
- `src/render.rs`: Visible cells, wrap boundaries and the current directory are captured together while holding the PTY buffer mutex. Link resolution then uses the detached frame after releasing the mutex. Filesystem probes cannot hold up the reader through this mutex.
- `src/paths.rs`: A per-frame resolver deduplicates file-existence probes across visible rows, including repeated relative-candidate/final-path checks. The cache is discarded after each resolved frame. Explicit UI refreshes force revalidation, including files created or deleted without terminal output.
- `src/render.rs`: Shaped text is reused by text, weight and foreground color. The cache retains current-frame entries, drops unused entries and invalidates on DPI changes. Cursor geometry, selection, underline/box graphics and IME preedit remain prepared on every paint; block-cursor ink has a separate color key.

Link resolution is still synchronous on the UI thread; this change shortens the PTY lock duration but does not claim fully asynchronous filesystem access. It does not introduce PTY output dropping, a new parser, or a new process/session model.

## Verification

The final complete suite was run with:

```sh
cargo test --locked --no-default-features --features desktop
cargo fmt --check
```

Result: 74 tests passed, zero failures. Nine optional tests were ignored by the ordinary suite, including native-window checks, Pi checks, external session/browser fixtures and the explicit microbenchmark.

Explicit checks also passed:

```sh
PI_CODING_AGENT_DIR=E:/WorkSpace/vibe-termial/target/terminal-perf-pi cargo test --locked --no-default-features --features desktop --test pi_scroll -- --ignored --test-threads=1
cargo test --locked --no-default-features --features desktop --bin vibe-terminal native_hidden_cursor_ime_anchor_matches_terminal -- --ignored --test-threads=1 --nocapture
```

The two real Pi/ConPTY tests verified history rendering, wheel/click/drag/hover forwarding and the caret in the middle of input, without requesting model output. The native IME test reported `(480, 303)` for both expected and actual anchor coordinates at 150% scale.

New deterministic checks cover:

- 1,000 background output chunks with no repeated foreground capture, followed by correct invalidation for foreground output and a replacement PTY.
- Scrollback changes, return to live output without a revision increment, resize, current-directory changes and workspace changes.
- Detached link resolution while the live buffer is independently locked, captured-directory correctness, line/column locations and file deletion revalidation.
- Repeated file paths sharing one filesystem-cache entry, fresh positive/negative file results in subsequent frames and current-directory precedence over workspace fallback.
- 1,000 frames with identical text requiring one shaping call, with distinct entries for bold/cursor ink, invalidation on DPI changes and eviction of unused text.

## Snapshot microbenchmark

```sh
cargo test --locked --no-default-features --features desktop --bin vibe-terminal unchanged_frame_capture_microbenchmark -- --ignored --nocapture
```

A Windows Debug run with a synthetic 30 × 100 terminal and 1,000 iterations measured:

| Operation | Total time |
| --- | ---: |
| Always capture and resolve the visible frame | 2,449.918 ms |
| Check and reuse an unchanged foreground frame | 0.105 ms |

This compares the forced full-capture path with the unchanged-frame fast path. It is not a whole-application speedup, is not a Release benchmark, and excludes PTY parsing, GPU painting, Agent runtime and filesystem changes. It demonstrates the work avoided for background-only notifications. macOS and whole-application Release CPU/memory measurements were not performed in this change.
