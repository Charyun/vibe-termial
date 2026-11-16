fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icons/app.ico");

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_DESKTOP").is_some()
    {
        // GPUI's Windows backend loads resource ID 1 as the window-class icon.
        // The same embedded group is used by Explorer and the taskbar.
        tauri_winres::WindowsResource::new()
            .set_icon_with_id("assets/icons/app.ico", "1")
            .set("ProductName", "Vibe Terminal")
            .set("FileDescription", "Vibe Terminal")
            .set("InternalName", "vibe-terminal")
            .set("OriginalFilename", "vibe-terminal.exe")
            .compile_for(&["vibe-terminal"])
            .expect("failed to embed the Vibe Terminal Windows icon");
    }
}
