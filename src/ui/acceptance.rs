//! Explicit native visual acceptance, with dormant sessions and isolated data.
use super::{FileLink, PanelTab, PreviewAction, PreviewContent, Ui};

#[cfg(windows)]
#[test]
#[ignore = "opens a native window; set VIBE_LAYOUT_SCREENSHOTS=1 to save optional screenshots"]
fn native_workbench_layout_acceptance() {
    use gpui_kit::*;
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    use vibe_terminal::{
        model::{AppData, CliKind, CliProfile, Tab},
        session::Session,
        store::Store,
        terminal::Terminal,
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data = std::env::temp_dir().join(format!("vibe-layout-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&data).unwrap();
    let output = root.join(".local/layout-acceptance");
    std::fs::create_dir_all(&output).unwrap();
    let mut model = AppData::default();
    model.add_workspace(&root).unwrap();
    let mut tabs = Vec::new();
    for (id, name, kind) in [
        ("pi", "Pi", CliKind::Pi),
        ("codex", "Codex", CliKind::Codex),
        ("claude", "Claude Code", CliKind::Claude),
        ("gemini", "Gemini CLI", CliKind::Custom),
        ("powershell", "PowerShell", CliKind::Shell),
    ] {
        tabs.push(Tab::new(
            CliProfile {
                id: id.into(),
                name: name.into(),
                command: id.into(),
                args: vec![],
                kind,
            },
            root.clone(),
        ));
    }
    let selected = tabs[0].id;
    let workspace = model.workspace_mut().unwrap();
    workspace.tabs = tabs;
    workspace.selected_tab = Some(selected);
    let mut session = Session::new(Store { root: data.clone() }, model, None);
    session.read_only = true;
    session.terminals.insert(selected,Terminal::dormant(30,100,root.clone(),b"Native workbench layout fixture\r\n\r\n  src/ui.rs:12\r\n  README.md\r\n\r\nNo Agent process is launched in this acceptance test.\r\n"));
    let passed = Arc::new(AtomicBool::new(false));
    let completed = passed.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
            gpui_kit::component::Theme::update(cx, |theme| {
                theme.font_family = "Geist".into();
                theme.mono_font_family = "Geist Mono".into();
                theme.secondary_hover = rgb(0x414c45).into();
                theme.secondary_active = rgb(0x587360).into();
            });
            cx.text_system()
                .add_fonts(vec![
                    std::borrow::Cow::Borrowed(include_bytes!("../../assets/fonts/Geist.ttf")),
                    std::borrow::Cow::Borrowed(include_bytes!("../../assets/fonts/GeistMono.ttf")),
                ])
                .unwrap();
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1360.0), px(860.0)),
                        cx,
                    ))),
                    ..gpui_kit::component::TitleBar::window_options()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe layout acceptance");
                    cx.new(|cx| {
                        let mut ui = Ui::new(session, window, cx);
                        ui.demo = true;
                        ui
                    })
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                smol::Timer::after(Duration::from_millis(500)).await;
                capture(output.join("desktop.png")).await;
                let sidebar_scale = handle
                    .update(cx, |_, window, _| window.scale_factor())
                    .unwrap();
                for (x, expected) in [(100.0, false), (100.0, true), (24.0, false), (24.0, true)] {
                    assert_eq!(native_mouse_point(x, 104.0, sidebar_scale, true), 1);
                    smol::Timer::after(Duration::from_millis(120)).await;
                    assert_eq!(
                        cx.update(|cx| {
                            let ui = view.read(cx);
                            ui.expanded_workspaces
                                .contains(&ui.session.data.selected_workspace.unwrap())
                        }),
                        expected,
                        "workspace label/arrow must toggle exactly once per click"
                    );
                }
                let (height, footer_scale) = handle
                    .update(cx, |_, window, _| {
                        (
                            f32::from(window.viewport_size().height),
                            window.scale_factor(),
                        )
                    })
                    .unwrap();
                let footer_y = height - 24.0 - 8.0 - 20.0;
                assert_eq!(native_mouse_point(60.0, footer_y, footer_scale, true), 1);
                smol::Timer::after(Duration::from_millis(200)).await;
                assert!(
                    cx.update(|cx| view.read(cx).settings),
                    "settings button did not open settings"
                );
                capture(output.join("settings.png")).await;
                assert_eq!(native_mouse_point(60.0, footer_y, footer_scale, true), 1);
                smol::Timer::after(Duration::from_millis(200)).await;
                assert!(
                    !cx.update(|cx| view.read(cx).settings),
                    "return button moved away from the settings button's position"
                );
                let (width, scale) = handle
                    .update(cx, |_, window, _| {
                        (
                            f32::from(window.viewport_size().width),
                            window.scale_factor(),
                        )
                    })
                    .unwrap();
                let toggle_x = width - 105.0 - 8.0 - 12.0;
                let hit = native_mouse_point(toggle_x, 20.0, scale, false);
                assert_eq!(
                    hit, 1,
                    "panel button must hit HTCLIENT, not the titlebar drag region"
                );
                capture(output.join("panel-button-hover.png")).await;
                for expected_open in [true, false, true] {
                    assert_eq!(native_mouse_point(toggle_x, 20.0, scale, true), 1);
                    smol::Timer::after(Duration::from_millis(150)).await;
                    assert_eq!(
                        cx.update(|cx| view.read(cx).panel.open),
                        expected_open,
                        "native panel button click did not toggle the panel"
                    );
                }
                assert_eq!(
                    native_mouse_point(140.0, 20.0, scale, false),
                    2,
                    "brand area must remain draggable"
                );
                for (offset, expected) in [(87.5, 8), (52.5, 9), (17.5, 20)] {
                    assert_eq!(
                        native_mouse_point(width - offset, 20.0, scale, false),
                        expected,
                        "native window control hit region changed"
                    );
                }
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            ui.request_preview_action(
                                PreviewAction::File(FileLink {
                                    start: 0,
                                    end: 0,
                                    path: root.join("src/ui.rs"),
                                    line: Some(12),
                                    column: None,
                                }),
                                window,
                                cx,
                            );
                        });
                    })
                    .unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    if cx.update(|cx| view.read(cx).preview_request.is_none()) {
                        break;
                    }
                    smol::Timer::after(Duration::from_millis(50)).await;
                }
                smol::Timer::after(Duration::from_millis(500)).await;
                cx.update(|cx| {
                    let ui = view.read(cx);
                    assert!(ui.panel.open);
                    assert!(matches!(ui.preview, Some(PreviewContent::Code(_))));
                });
                capture(output.join("code-preview.png")).await;
                let changes = smol::unblock({
                    let root = root.clone();
                    move || crate::project::changes(&root)
                })
                .await
                .unwrap();
                let first = changes
                    .files
                    .iter()
                    .find(|f| f.relative == "README.md" && f.unstaged())
                    .or_else(|| {
                        changes
                            .files
                            .iter()
                            .find(|f| f.unstaged() && f.path.is_file())
                    })
                    .cloned()
                    .expect("fixture repository has a working change");
                cx.update(|cx| {
                    view.update(cx, |ui, cx| {
                        ui.panel.tab = PanelTab::Changes;
                        ui.sync_project_panel();
                        ui.open_diff(first, false);
                        cx.notify();
                    })
                });
                smol::Timer::after(Duration::from_millis(700)).await;
                capture(output.join("source-control.png")).await;
                cx.update(|cx| {
                    view.update(cx, |ui, cx| {
                        ui.detected_agents = vibe_terminal::agents::detected();
                        ui.new_menu = Some((420.0, 40.0));
                        cx.notify();
                    })
                });
                smol::Timer::after(Duration::from_millis(400)).await;
                capture(output.join("new-terminal.png")).await;
                handle
                    .update(cx, |_, window, cx| {
                        window.resize(size(px(900.0), px(600.0)));
                        view.update(cx, |ui, cx| {
                            ui.new_menu = None;
                            ui.panel.tab = PanelTab::Explorer;
                            cx.notify();
                        });
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(500)).await;
                capture(output.join("desktop-900.png")).await;
                let (height, scale) = handle
                    .update(cx, |_, window, _| {
                        (
                            f32::from(window.viewport_size().height),
                            window.scale_factor(),
                        )
                    })
                    .unwrap();
                let footer_y = height - 24.0 - 8.0 - 20.0;
                for expected in [true, false] {
                    assert_eq!(native_mouse_point(60.0, footer_y, scale, true), 1);
                    smol::Timer::after(Duration::from_millis(150)).await;
                    assert_eq!(
                        cx.update(|cx| view.read(cx).settings),
                        expected,
                        "navigation footer misaligned in narrow window"
                    );
                }
                completed.store(true, Ordering::Release);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(passed.load(Ordering::Acquire));
    let _ = std::fs::remove_dir_all(data);
}
#[cfg(windows)]
fn native_mouse_point(x: f32, y: f32, scale: f32, click: bool) -> isize {
    use windows_sys::Win32::{
        Foundation::{HWND, POINT},
        UI::WindowsAndMessaging::{
            FindWindowW, PostMessageW, SendMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
            WM_NCHITTEST,
        },
    };
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ClientToScreen(window: HWND, point: *mut POINT) -> i32;
    }
    let title: Vec<u16> = "Vibe layout acceptance\0".encode_utf16().collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        assert!(!hwnd.is_null());
        let px = (x * scale).round() as i32;
        let py = (y * scale).round() as i32;
        let client = ((px as u16 as u32) | ((py as u16 as u32) << 16)) as isize;
        SendMessageW(hwnd, WM_MOUSEMOVE, 0, client);
        let mut screen = POINT { x: px, y: py };
        assert_ne!(ClientToScreen(hwnd, &mut screen), 0);
        let position = ((screen.x as u16 as u32) | ((screen.y as u16 as u32) << 16)) as isize;
        let hit = SendMessageW(hwnd, WM_NCHITTEST, 0, position);
        if click && hit == 1 {
            assert_ne!(PostMessageW(hwnd, WM_LBUTTONDOWN, 1, client), 0);
            assert_ne!(PostMessageW(hwnd, WM_LBUTTONUP, 0, client), 0);
        }
        hit
    }
}
#[cfg(windows)]
async fn capture(path: std::path::PathBuf) {
    if std::env::var("VIBE_LAYOUT_SCREENSHOTS").as_deref() != Ok("1") {
        return;
    }
    smol::unblock(move || {
        let appdata = std::env::var_os("APPDATA").unwrap();
        let cli = std::path::PathBuf::from(appdata)
            .join("npm/node_modules/agent-desktop/bin/agent-desktop-win32-x64.exe");
        if !cli.is_file() {
            return;
        }
        let title: Vec<u16> = "Vibe layout acceptance\0".encode_utf16().collect();
        let hwnd = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW(
                std::ptr::null(),
                title.as_ptr(),
            )
        };
        assert!(!hwnd.is_null());
        let home = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".local/agent-ui");
        for attempt in 0..2 {
            let output = std::process::Command::new(&cli)
                .env("AGENT_DESKTOP_HOME", &home)
                .arg("screenshot")
                .arg("--window-id")
                .arg(format!("w-{}", hwnd as usize))
                .arg(&path)
                .output()
                .unwrap();
            if output.status.success() {
                return;
            }
            let error = String::from_utf8_lossy(&output.stdout);
            if attempt == 0 && error.contains("TIMEOUT") {
                std::thread::sleep(std::time::Duration::from_millis(250));
                continue;
            }
            // Optional screenshot tooling must not strand the native app's
            // event loop. Functional click/state assertions above stay strict.
            eprintln!(
                "optional screenshot unavailable for {}: {} {}",
                path.display(),
                error,
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
    })
    .await;
}
