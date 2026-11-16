use super::{Arc, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use futures_util::FutureExt;
use gpui_kit::{AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use std::{fs, sync::atomic::AtomicBool};
use vibe_terminal::activity::Indicator;
async fn capture(name: &str) {
    let Some(directory) = std::env::var_os("VIBE_NATIVE_CAPTURE_DIR").map(std::path::PathBuf::from)
    else {
        return;
    };
    fs::create_dir_all(&directory).unwrap();
    let output = directory.join(format!("agent-status-{name}.png"));
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
        "native capture failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn mouse(message: u32, down: bool, scale: f32, sidebar: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW};
    unsafe {
        let title: Vec<u16> = "Vibe Agent status regression\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        assert!(!hwnd.is_null());
        let (x, y) = if sidebar {
            (100.0, 134.0)
        } else {
            (243.0, 20.0)
        };
        let x = (x * scale).round() as i32;
        let y = (y * scale).round() as i32;
        SendMessageW(
            hwnd,
            message,
            if down { 1 } else { 0 },
            ((y << 16) | (x & 0xffff)) as isize,
        );
    }
}
#[test]
#[ignore = "opens a native window, drives local PTYs and verifies Agent status and Ctrl+C exit"]
fn native_agent_tab_spins_completes_and_clears_on_click() {
    let root = std::env::temp_dir().join(format!("vibe-activity-ui-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let helper = root.join("agent.cjs");
    fs::write(
        &helper,
        include_str!("../../tests/fixtures/activity-agent.cjs"),
    )
    .unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    let profile = CliProfile {
        id: "native-status".into(),
        name: "Pi fixture".into(),
        command: "node".into(),
        args: vec![helper.to_string_lossy().into_owned()],
        kind: CliKind::Pi,
    };
    session.new_tab(profile.clone());
    let first = session.selected().unwrap();
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    let cleanup = root.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(gpui_kit::component::TitleBar::title_bar_options()),
                    app_owns_titlebar_drag: true,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.0), px(100.0)),
                        size(px(1280.0), px(800.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe Agent status regression");
                    cx.new(|cx| Ui::new(session, window, cx))
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    smol::Timer::after(Duration::from_millis(800)).await;
                    handle
                        .update(cx, |_, _, cx| {
                            view.update(cx, |ui, _| ui.session.send(b"W".to_vec()))
                        })
                        .unwrap();
                    let mut working = false;
                    for _ in 0..60 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        working = cx.update(|cx| {
                            view.read(cx).agent_activity.get(&first) == Some(&Indicator::Working)
                        });
                        if working {
                            break;
                        }
                    }
                    assert!(working, "the tab must show its Agent's working state");
                    smol::Timer::after(Duration::from_millis(100)).await;
                    capture("working").await;
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                ui.new_terminal_with(profile, window, cx);
                                assert_ne!(ui.session.selected(), Some(first));
                                ui.session.terminals[&first].send(b"D".to_vec()).unwrap();
                            })
                        })
                        .unwrap();
                    let mut completed = false;
                    for _ in 0..60 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        completed = cx.update(|cx| {
                            view.read(cx).agent_activity.get(&first) == Some(&Indicator::Completed)
                        });
                        if completed {
                            break;
                        }
                    }
                    assert!(completed, "background completion must light the green dot");
                    smol::Timer::after(Duration::from_millis(100)).await;
                    capture("completed").await;
                    let scale = handle
                        .update(cx, |_, window, _| window.scale_factor())
                        .unwrap();
                    mouse(0x200, false, scale, true);
                    smol::Timer::after(Duration::from_millis(80)).await;
                    mouse(0x201, true, scale, true);
                    mouse(0x202, false, scale, true);
                    let mut cleared = false;
                    for _ in 0..60 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        cleared = cx.update(|cx| {
                            let ui = view.read(cx);
                            ui.session.selected() == Some(first)
                                && ui.agent_activity.get(&first) == Some(&Indicator::None)
                        });
                        if cleared {
                            break;
                        }
                    }
                    assert!(
                        cleared,
                        "clicking the navigation row must acknowledge completion on both surfaces"
                    );
                    smol::Timer::after(Duration::from_millis(350)).await;
                    assert_eq!(
                        cx.update(|cx| view.read(cx).agent_activity.get(&first).copied()),
                        Some(Indicator::None)
                    );
                    cx.update(|cx| {
                        let ui = view.read(cx);
                        assert!(ui.agent_activity_indicator(first, "sidebar").is_none());
                        assert!(ui.agent_activity_indicator(first, "tab").is_none());
                    });
                    capture("sidebar-acknowledged").await;
                    handle
                        .update(cx, |_, _, cx| {
                            view.update(cx, |ui, _| ui.session.send(b"WD".to_vec()))
                        })
                        .unwrap();
                    let mut completed_again = false;
                    for _ in 0..60 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        completed_again = cx.update(|cx| {
                            view.read(cx).agent_activity.get(&first) == Some(&Indicator::Completed)
                        });
                        if completed_again {
                            break;
                        }
                    }
                    assert!(
                        completed_again,
                        "a new completion must appear on both surfaces after acknowledgment"
                    );
                    mouse(0x200, false, scale, false);
                    smol::Timer::after(Duration::from_millis(80)).await;
                    mouse(0x201, true, scale, false);
                    mouse(0x202, false, scale, false);
                    smol::Timer::after(Duration::from_millis(350)).await;
                    assert_eq!(
                        cx.update(|cx| view.read(cx).agent_activity.get(&first).copied()),
                        Some(Indicator::None)
                    );
                    capture("acknowledged").await;
                    let original = cx.update(|cx| view.read(cx).session.terminals[&first].buffer.clone());
                    handle.update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| ui.focus_current_view(window, cx));
                        window.dispatch_event(gpui_kit::PlatformInput::KeyDown(gpui_kit::KeyDownEvent {
                            keystroke: gpui_kit::Keystroke::parse("ctrl-c").unwrap(),
                            is_held: false, prefer_character_input: false,
                        }), cx);
                    }).unwrap();
                    smol::Timer::after(Duration::from_millis(350)).await;
                    cx.update(|cx| {
                        let ui = view.read(cx);
                        assert_eq!(ui.session.data.tab(first).unwrap().profile.kind, CliKind::Pi);
                        assert!(Arc::ptr_eq(&original, &ui.session.terminals[&first].buffer));
                    });
                    handle.update(cx, |_, window, cx| {
                        window.dispatch_event(gpui_kit::PlatformInput::KeyDown(gpui_kit::KeyDownEvent {
                            keystroke: gpui_kit::Keystroke::parse("ctrl-c").unwrap(),
                            is_held: false, prefer_character_input: false,
                        }), cx);
                    }).unwrap();
                    let mut returned = false;
                    for _ in 0..100 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        returned = cx.update(|cx| view.read(cx).session.data.tab(first).unwrap().profile.kind == CliKind::Shell);
                        if returned { break; }
                    }
                    assert!(returned, "Ctrl+C exiting the Agent must return the same navigation row and tab to CMD");
                    let shell = cx.update(|cx| {
                        let ui = view.read(cx);
                        assert_eq!(ui.session.data.tab(first).unwrap().display_title(), "CMD");
                        assert!(ui.agent_activity_indicator(first, "sidebar").is_none());
                        assert!(ui.agent_activity_indicator(first, "tab").is_none());
                        let terminal = &ui.session.terminals[&first];
                        let buffer = terminal.buffer.lock().unwrap();
                        assert!(!buffer.parser.screen().alternate_screen());
                        assert!(!buffer.parser.screen().bracketed_paste());
                        terminal.buffer.clone()
                    });
                    handle.update(cx, |_, window, cx| view.update(cx, |ui, cx| {
                        let other = ui.session.data.workspace().unwrap().tabs.iter().find(|t| t.id != first).unwrap().id;
                        ui.select_tab(other, window, cx);
                        ui.select_tab(first, window, cx);
                        assert!(Arc::ptr_eq(&shell, &ui.session.terminals[&first].buffer));
                        ui.session.send(b"echo NATIVE_CMD_RETURN_OK\r".to_vec());
                    })).unwrap();
                    let mut executed = false;
                    for _ in 0..100 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        executed = cx.update(|cx| view.read(cx).session.terminals[&first].buffer.lock().unwrap().parser.screen().contents().lines().any(|line| line.trim() == "NATIVE_CMD_RETURN_OK"));
                        if executed { break; }
                    }
                    assert!(executed, "returned CMD must execute commands after switching away and back");
                    assert!(cx.update(|cx| view.read(cx).session.error.is_none()), "native fixture must not leave a persistence error");
                    capture("returned-cmd").await;
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
