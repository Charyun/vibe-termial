use super::{Arc, AtomicBool, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use gpui_kit::{
    AppContext, Bounds, PlatformInput, ScrollDelta, ScrollEvent, WindowBounds, WindowOptions,
    point, px, size,
};
use std::{fs, sync::Mutex};

#[test]
#[ignore = "opens a native alternate-screen terminal and verifies no wheel-to-arrow bytes reach its raw PTY"]
fn native_wheel_without_mouse_tracking_never_sends_arrow_keys() {
    let root = std::env::temp_dir().join(format!("vibe-wheel-fallback-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let helper = root.join("raw.cjs");
    let receipt = root.join("input.bin");
    fs::write(&helper, "const fs=require('node:fs');process.stdin.setRawMode(true);process.stdin.on('data',b=>fs.appendFileSync(process.argv[2],b));console.log('WHEEL_READY');").unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    session.new_tab(CliProfile {
        id: "wheel-shell".into(),
        name: "Shell fixture".into(),
        command: "node".into(),
        args: vec![
            helper.to_string_lossy().into_owned(),
            receipt.to_string_lossy().into_owned(),
        ],
        kind: CliKind::Shell,
    });
    let id = session.selected().unwrap();
    let actual = Arc::new(Mutex::new(Vec::new()));
    let received = actual.clone();
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.0), px(100.0)),
                        size(px(1100.0), px(700.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut ui = Ui::new(session, window, cx);
                        ui.demo = true;
                        ui
                    })
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                for _ in 0..100 {
                    if cx.update(|cx| {
                        view.read(cx).session.terminals[&id]
                            .buffer
                            .lock()
                            .unwrap()
                            .parser
                            .screen()
                            .contents()
                            .contains("WHEEL_READY")
                    }) {
                        break;
                    }
                    smol::Timer::after(Duration::from_millis(50)).await;
                }
                handle
                    .update(cx, |_, _, cx| {
                        view.update(cx, |ui, cx| {
                            // Model a visible alternate screen without DEC mouse tracking,
                            // exactly the state that previously selected arrow-key fallback.
                            ui.session.terminals[&id]
                                .buffer
                                .lock()
                                .unwrap()
                                .process(b"\x1b[?1049h");
                            ui.refresh();
                            assert!(
                                ui.session.terminals[&id]
                                    .buffer
                                    .lock()
                                    .unwrap()
                                    .parser
                                    .screen()
                                    .alternate_screen()
                            );
                            cx.notify();
                        })
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(150)).await;
                handle
                    .update(cx, |_, window, cx| {
                        let position = view.read(cx).bounds.center();
                        for delta in [3.0, -3.0] {
                            window.dispatch_event(
                                PlatformInput::Scroll(ScrollEvent {
                                    position,
                                    delta: ScrollDelta::Lines(point(0.0, delta)),
                                    ..Default::default()
                                }),
                                cx,
                            );
                        }
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(200)).await;
                *received.lock().unwrap() = fs::read(&receipt).unwrap_or_default();
                handle
                    .update(cx, |_, _, cx| {
                        view.update(cx, |ui, _| ui.session.shutdown())
                    })
                    .unwrap();
                finished.store(true, Ordering::Release);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    fs::remove_dir_all(root).unwrap();
    assert!(passed.load(Ordering::Acquire));
    let bytes = actual.lock().unwrap();
    assert!(
        bytes.is_empty(),
        "scrolling must not synthesize input: {bytes:?}"
    );
}
