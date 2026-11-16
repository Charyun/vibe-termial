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
fn mouse(message: u32, down: bool, scale: f32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW};
    unsafe {
        let title: Vec<u16> = "Vibe Agent status regression\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        assert!(!hwnd.is_null());
        let x = (243.0 * scale).round() as i32;
        let y = (20.0 * scale).round() as i32;
        SendMessageW(
            hwnd,
            message,
            if down { 1 } else { 0 },
            ((y << 16) | (x & 0xffff)) as isize,
        );
    }
}
#[test]
#[ignore = "opens a native window, drives local PTYs and clicks the completed Agent tab"]
fn native_agent_tab_spins_completes_and_clears_on_click() {
    let root = std::env::temp_dir().join(format!("vibe-activity-ui-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
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
                    mouse(0x200, false, scale);
                    smol::Timer::after(Duration::from_millis(80)).await;
                    mouse(0x201, true, scale);
                    mouse(0x202, false, scale);
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
                        "clicking the terminal tab must acknowledge its completion"
                    );
                    smol::Timer::after(Duration::from_millis(350)).await;
                    assert_eq!(
                        cx.update(|cx| view.read(cx).agent_activity.get(&first).copied()),
                        Some(Indicator::None)
                    );
                    capture("acknowledged").await;
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
