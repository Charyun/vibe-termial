use super::super::{Arc, CliProfile, Duration, Ordering, Session, SettingsTab, Ui, Uuid};
use futures_util::FutureExt;
use gpui_kit::{AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use std::{fs, sync::atomic::AtomicBool};
use vibe_terminal::model::WindowMaterial;

async fn capture(label: &str) {
    let Some(root) = std::env::var_os("VIBE_MATERIAL_CAPTURE_DIR") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    fs::create_dir_all(&root).unwrap();
    let output = root.join(format!("{label}.png"));
    let result = smol::unblock(move || {
        std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-File",
                concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/capture-window.ps1"),
                "-ClientProcessId",
                &std::process::id().to_string(),
                "-OutputFile",
            ])
            .arg(output)
            .output()
    })
    .await
    .unwrap();
    assert!(
        result.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
#[ignore = "opens a native window and verifies all three material modes through Windows DWM"]
fn native_material_themes_switch_and_clear_previous_backdrops() {
    let root = std::env::temp_dir().join(format!("vibe-native-material-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("history")).unwrap();
    fs::create_dir_all(root.join("sessions")).unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let tab = vibe_terminal::model::Tab::new(CliProfile::default_shell(), root.clone());
    let id = tab.id;
    data.workspace_mut().unwrap().tabs.push(tab);
    data.workspace_mut().unwrap().selected_tab = Some(id);
    data.workspace_mut().unwrap().right_panel_open = true;
    let store = vibe_terminal::store::Store { root: root.clone() };
    store.save_history(id, b"Microsoft Windows\r\n\r\nC:\\Projects\\vibe-terminal> cargo check\r\n\x1b[32m    Checking\x1b[0m vibe-terminal v0.1.0\r\n\x1b[32m    Finished\x1b[0m dev profile\r\n\r\nC:\\Projects\\vibe-terminal> ").unwrap();
    let mut session = Session::new(store, data, None);
    session.read_only = true;
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    let cleanup = root.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(gpui_kit::component::TitleBar::title_bar_options()),
                    app_owns_titlebar_drag: true,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(140.), px(100.)),
                        size(px(1280.), px(800.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe native material themes");
                    cx.new(|cx| {
                        let mut ui = Ui::new(session, window, cx);
                        ui.demo = true;
                        ui.session.read_only = false;
                        ui
                    })
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    eprintln!("Windows build: {}", crate::material::windows_build());
                    for (material, label) in [
                        (WindowMaterial::Mica, "mica"),
                        (WindowMaterial::MicaAlt, "mica-alt"),
                        (WindowMaterial::Acrylic, "acrylic"),
                        (WindowMaterial::Mica, "mica-restored"),
                    ] {
                        handle
                            .update(cx, |_, window, cx| {
                                view.update(cx, |ui, cx| {
                                    ui.settings = true;
                                    ui.settings_tab = SettingsTab::Theme;
                                    ui.set_window_material(material, window, cx);
                                    assert_eq!(
                                        ui.session.store.load().unwrap().window_material,
                                        material
                                    );
                                    assert_eq!(
                                        ui.material_appearance,
                                        crate::material::effective(
                                            material,
                                            crate::material::windows_build()
                                        )
                                    );
                                    if crate::material::windows_build() >= 22621 {
                                        use windows_sys::Win32::Graphics::Dwm::{
                                            DWMWA_SYSTEMBACKDROP_TYPE, DwmGetWindowAttribute,
                                        };
                                        let mut backdrop = 0u32;
                                        let status = unsafe {
                                            DwmGetWindowAttribute(
                                                crate::material::hwnd(window).unwrap(),
                                                DWMWA_SYSTEMBACKDROP_TYPE as _,
                                                (&mut backdrop as *mut u32).cast(),
                                                4,
                                            )
                                        };
                                        assert!(status >= 0);
                                        assert_eq!(
                                            backdrop,
                                            match material {
                                                WindowMaterial::Mica => 2,
                                                WindowMaterial::MicaAlt => 4,
                                                WindowMaterial::Acrylic => 1,
                                            }
                                        );
                                    }
                                })
                            })
                            .unwrap();
                        smol::Timer::after(Duration::from_millis(350)).await;
                        capture(label).await;
                    }
                    handle
                        .update(cx, |_, _, cx| {
                            view.update(cx, |ui, cx| {
                                ui.settings = false;
                                cx.notify();
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(350)).await;
                    capture("terminal-mica").await;
                })
                .catch_unwind()
                .await;
                let _ = handle.update(cx, |_, _, cx| {
                    view.update(cx, |ui, _| ui.session.shutdown())
                });
                finished.store(outcome.is_ok(), Ordering::Release);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(passed.load(Ordering::Acquire));
    fs::remove_dir_all(cleanup).unwrap();
}
