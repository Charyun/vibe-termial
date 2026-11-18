use super::{Arc, AtomicBool, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use gpui_kit::{
    AppContext, Bounds, ExternalPaths, FileDropEvent, Focusable, PlatformInput, WindowBounds,
    WindowOptions, point, px, size,
};
use std::{fs, sync::Mutex};

#[test]
#[ignore = "opens a native window, dispatches platform file-drop events and checks real PTY input"]
fn native_file_drops_reach_terminal_and_composer_without_submitting() {
    let root = std::env::temp_dir().join(format!("vibe-file-drop-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let helper = root.join("drop-agent.cjs");
    let receipt = root.join("input.bin");
    fs::write(&helper, "const fs=require('node:fs');process.stdin.setRawMode(true);process.stdin.on('data',b=>fs.appendFileSync(process.argv[2],b));console.log('DROP_READY');").unwrap();
    let paths = vec![root.join("中文 file.txt"), root.join("second file.png")];
    for path in &paths {
        fs::write(path, "fixture").unwrap();
    }
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    session.new_tab(CliProfile {
        id: "drop-fixture".into(),
        name: "Pi fixture".into(),
        command: "node".into(),
        args: vec![
            helper.to_string_lossy().into_owned(),
            receipt.to_string_lossy().into_owned(),
        ],
        kind: CliKind::Pi,
    });
    let id = session.selected().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let results = observed.clone();
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
                        size(px(1200.0), px(800.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe file drop regression");
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
                            .contains("DROP_READY")
                    }) {
                        break;
                    }
                    smol::Timer::after(Duration::from_millis(50)).await;
                }
                smol::Timer::after(Duration::from_millis(200)).await;
                let expected = cx.update(|cx| {
                    vibe_terminal::file_drop::path_input(
                        &view.read(cx).session.data.tab(id).unwrap().profile,
                        &paths,
                    )
                    .unwrap()
                });
                handle
                    .update(cx, |_, window, cx| {
                        let position = view.read(cx).bounds.center();
                        window.dispatch_event(
                            PlatformInput::FileDrop(FileDropEvent::Entered {
                                position,
                                paths: ExternalPaths(paths.clone().into_iter().collect()),
                            }),
                            cx,
                        );
                        window.dispatch_event(
                            PlatformInput::FileDrop(FileDropEvent::Submit { position }),
                            cx,
                        );
                        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Ended), cx);
                    })
                    .unwrap();
                for _ in 0..80 {
                    if fs::read(&receipt).unwrap_or_default() == expected.as_bytes() {
                        break;
                    }
                    smol::Timer::after(Duration::from_millis(50)).await;
                }
                results.lock().unwrap().push((
                    "terminal exact input without Enter",
                    fs::read(&receipt).unwrap_or_default() == expected.as_bytes(),
                ));
                // Exiting/cancelling an external drag must never insert its paths.
                handle
                    .update(cx, |_, window, cx| {
                        let position = view.read(cx).bounds.center();
                        window.dispatch_event(
                            PlatformInput::FileDrop(FileDropEvent::Entered {
                                position,
                                paths: ExternalPaths(paths.clone().into_iter().collect()),
                            }),
                            cx,
                        );
                        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Exited), cx);
                        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Ended), cx);
                    })
                    .unwrap();
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            ui.toggle_transcript(window, cx);
                            ui.conversations[&id].input.update(cx, |input, cx| {
                                input.set_value("old draft", window, cx);
                                input.select_all(window, cx);
                                input.replace("prefix ", window, cx);
                            });
                        })
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(200)).await;
                handle
                    .update(cx, |_, window, cx| {
                        let position = view.read(cx).conversations[&id]
                            .input
                            .read(cx)
                            .cursor_layout()
                            .unwrap()
                            .0
                            .center();
                        window.dispatch_event(
                            PlatformInput::FileDrop(FileDropEvent::Entered {
                                position,
                                paths: ExternalPaths(paths.clone().into_iter().collect()),
                            }),
                            cx,
                        );
                        window.dispatch_event(
                            PlatformInput::FileDrop(FileDropEvent::Submit { position }),
                            cx,
                        );
                        window.dispatch_event(PlatformInput::FileDrop(FileDropEvent::Ended), cx);
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(150)).await;
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            let input = &ui.conversations[&id].input;
                            results.lock().unwrap().push((
                                "composer inserts at caret and retains draft",
                                input.read(cx).value() == format!("prefix {expected}"),
                            ));
                            results.lock().unwrap().push((
                                "composer receives keyboard focus",
                                input.read(cx).focus_handle(cx).is_focused(window),
                            ));
                            results.lock().unwrap().push((
                                "composer drop is not sent to PTY",
                                fs::read(&receipt).unwrap_or_default() == expected.as_bytes(),
                            ));
                            results.lock().unwrap().push((
                                "drop never starts a turn",
                                ui.session.terminals[&id]
                                    .buffer
                                    .lock()
                                    .unwrap()
                                    .parser
                                    .callbacks()
                                    .activity
                                    .indicator(true)
                                    == vibe_terminal::activity::Indicator::None,
                            ));
                            ui.session.shutdown();
                        })
                    })
                    .unwrap();
                finished.store(true, Ordering::Release);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    fs::remove_dir_all(root).unwrap();
    assert!(passed.load(Ordering::Acquire));
    for (name, success) in observed.lock().unwrap().iter() {
        eprintln!("{name}: {success}");
    }
    assert!(observed.lock().unwrap().iter().all(|(_, success)| *success));
}
