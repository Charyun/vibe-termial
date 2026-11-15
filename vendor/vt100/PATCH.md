# Local vt100 0.16.2 patch

Upstream: https://github.com/doy/vt100-rust, crates.io release 0.16.2. Original MIT license is retained in LICENSE.

`src/row.rs::Row::resize` now clears a wide-character lead cell when its continuation has been cut off at the new right edge. Upstream `Row::truncate` already handles this invariant, but `resize` did not. After a terminal is narrowed, overwriting such an orphan called `unwrap()` on the missing continuation in `Screen::text` (screen.rs:870), panicking the PTY reader and poisoning the app's buffer mutex.

The app regression test `shrinking_a_wide_character_at_the_right_edge_does_not_break_output` reproduces the exact upstream panic with a Chinese character at columns 9–10, a resize to 9 columns, and a subsequent ASCII write at column 9. It must pass before updating or removing this patch.

Cargo pins `vt100 =0.16.2` and applies this local source through `[patch.crates-io]`. Do not edit the global Cargo registry cache.
