# Vibe Terminal application icon

The mint terminal prompt is the application's brand mark. The header and Windows executable use the same geometry and colors.

| File | Purpose |
| --- | --- |
| `../logo.svg` | Vector source mark |
| `../logo.png` | 128px transparent raster image embedded for the native title bar |
| `app-icon.png` | 1024px transparent PNG for reuse and previews |
| `app.ico` | Windows icon bundle: 16, 20, 24, 32, 40, 48, 64, 128 and 256px |

Regenerate the committed assets with Python and Pillow:

```sh
python scripts/generate-icons.py
```

Normal builds do not require Python or Pillow. The generated assets are embedded at compile time.

On Windows, `build.rs` embeds `app.ico` as icon group **1** using `tauri-winres`. This ID is deliberate: GPUI's Windows backend loads icon resource 1 and installs it as the window-class icon, which Windows also uses for the taskbar. Explorer reads the same group from the executable. `tauri-winres::set_icon()` defaults to a different ID, so use `set_icon_with_id(..., "1")` here.

The application identity is `dev.vibe.terminal`, displayed as **Vibe Terminal**. Native header rendering uses the embedded `logo.png`; it does not depend on finding an image next to the executable.

Verified on Windows with `cargo build --locked` and an isolated application window. All nine embedded frames decoded successfully. The window-class icon and Windows' extracted executable icon rendered identically when the verification process used per-monitor DPI awareness. Native-window evidence is under `target/icon-verification/`.
