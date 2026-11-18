use super::{Arc, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use futures_util::FutureExt;
use gpui_kit::{
    AppContext, Bounds, PlatformInput, ScrollDelta, ScrollEvent, WindowBounds, WindowOptions,
    point, px, size,
};
use std::{fs, sync::atomic::AtomicBool};

#[test]
#[ignore = "opens a native window and launches Pi from CMD"]
fn native_cmd_detects_pi_and_updates_shared_tab_identity() {
    let root = std::env::temp_dir().join(format!("vibe-shell-ui-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let pi_session = root.join("pi-scroll-session.jsonl");
    let mut records = vec![serde_json::json!({
        "type":"session", "version":3, "id":Uuid::new_v4().to_string(),
        "timestamp":"2026-11-17T00:00:00.000Z", "cwd":root,
    })];
    let mut parent = None::<String>;
    for turn in 0..12 {
        let user = format!("user{turn:04}");
        let assistant = format!("resp{turn:04}");
        records.push(serde_json::json!({"type":"message","id":user,"parentId":parent,
            "timestamp":"2026-11-17T00:00:00.000Z","message":{"role":"user",
            "content":[{"type":"text","text":format!("PI test prompt {turn:03}")}],"timestamp":1794873600000u64}}));
        let text = (0..30)
            .map(|line| format!("PI_SCROLL_{turn:03}_{line:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        records.push(serde_json::json!({"type":"message","id":assistant,"parentId":user,
            "timestamp":"2026-11-17T00:00:00.000Z","message":{"role":"assistant",
            "content":[{"type":"text","text":text}],"api":"openai-completions","provider":"openai",
            "model":"gpt-4o-mini","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
            "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":1794873600000u64}}));
        parent = Some(assistant);
    }
    fs::write(
        &pi_session,
        records
            .into_iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let agent_config = root.join("agent-config");
    fs::create_dir_all(&agent_config).unwrap();
    let pi = vibe_terminal::agents::resolve("pi")
        .expect("Pi must be installed for this acceptance test");
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    session.new_tab(CliProfile::default_shell());
    let id = session.selected().unwrap();
    let original = session.terminals[&id].buffer.clone();
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(gpui_kit::component::TitleBar::title_bar_options()),
                    app_owns_titlebar_drag: true,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.), px(100.)),
                        size(px(1280.), px(800.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe CMD Agent identity regression");
                    cx.new(|cx| Ui::new(session, window, cx))
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    handle
                        .update(cx, |_, _, cx| {
                            view.update(cx, |ui, _| {
                                ui.session
                                    .send(format!("set \"PI_CODING_AGENT_DIR={}\" && set \"PI_OFFLINE=1\" && \"{}\" --session \"{}\" --tui-mode fullscreen --no-extensions --no-skills --no-prompt-templates\r", agent_config.display(), pi.display(), pi_session.display()).into_bytes());
                            })
                        })
                        .unwrap();
                    let mut detected = false;
                    for _ in 0..200 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        detected = cx.update(|cx| {
                            view.read(cx).session.data.tab(id).unwrap().profile.kind == CliKind::Pi
                        });
                        if detected {
                            break;
                        }
                    }
                    assert!(
                        detected,
                        "actual Pi launched within CMD must be detected by the UI worker"
                    );
                    cx.update(|cx| {
                        let ui = view.read(cx);
                        let tab = ui.session.data.tab(id).unwrap();
                        // Both chrome and navigation consume this profile/title.
                        assert_eq!(tab.profile.id, "pi");
                        assert_eq!(tab.display_title(), "Pi");
                        assert!(Arc::ptr_eq(&original, &ui.session.terminals[&id].buffer));
                        assert!(ui.session.error.is_none());
                    });
                    let screen = |cx: &mut gpui_kit::App| {
                        view.read(cx).session.terminals[&id].buffer.lock().unwrap().parser.screen().contents()
                    };
                    let mut ready = false;
                    for _ in 0..200 {
                        ready = cx.update(|cx| screen(cx).contains("PI_SCROLL_011_029"));
                        if ready { break; }
                        smol::Timer::after(Duration::from_millis(50)).await;
                    }
                    assert!(ready, "CMD-launched Pi must render the seeded latest message: {}", cx.update(screen));
                    handle.update(cx, |_, window, cx| {
                        let position = view.read(cx).bounds.center();
                        window.dispatch_event(PlatformInput::Scroll(ScrollEvent {
                            position, delta: ScrollDelta::Lines(point(0.0, 12.0)), ..Default::default()
                        }), cx);
                    }).unwrap();
                    let mut scrolled = false;
                    for _ in 0..100 {
                        scrolled = cx.update(|cx| screen(cx).contains("Jump to latest message"));
                        if scrolled { break; }
                        smol::Timer::after(Duration::from_millis(50)).await;
                    }
                    assert!(scrolled, "native wheel must scroll Pi history instead of navigating prompt history: {}", cx.update(screen));
                    handle.update(cx, |_, window, cx| {
                        let position = view.read(cx).bounds.center();
                        window.dispatch_event(PlatformInput::Scroll(ScrollEvent {
                            position, delta: ScrollDelta::Lines(point(0.0, -100.0)), ..Default::default()
                        }), cx);
                    }).unwrap();
                    let mut latest = false;
                    for _ in 0..100 {
                        latest = cx.update(|cx| { let text = screen(cx); text.contains("PI_SCROLL_011_029") && !text.contains("Jump to latest message") });
                        if latest { break; }
                        smol::Timer::after(Duration::from_millis(50)).await;
                    }
                    assert!(latest, "wheel down must return to the latest message without arrow keys: {}", cx.update(screen));
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
    fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "opens a native window and launches Pi from CMD to observe live status"]
fn native_cmd_launched_pi_publishes_tab_status() {
    let root = std::env::temp_dir().join(format!("vibe-cmd-status-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let pi = vibe_terminal::agents::resolve("pi").expect("Pi must be installed");
    let session_file = root.join("pi-status.jsonl");
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    session.new_tab(CliProfile::default_shell());
    let id = session.selected().unwrap();
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(gpui_kit::component::TitleBar::title_bar_options()),
                    app_owns_titlebar_drag: true,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.), px(100.)),
                        size(px(1280.), px(800.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe CMD Agent status regression");
                    cx.new(|cx| Ui::new(session, window, cx))
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    handle
                        .update(cx, |_, _, cx| {
                            view.update(cx, |ui, _| {
                                ui.session.send(format!("set \"PI_OFFLINE=1\" && \"{}\" --session \"{}\" --tui-mode fullscreen --no-extensions --no-skills --no-prompt-templates\r", pi.display(), session_file.display()).into_bytes());
                            })
                        })
                        .unwrap();
                    let mut detected = false;
                    for _ in 0..300 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        detected = cx.update(|cx| {
                            view.read(cx).session.data.tab(id).unwrap().profile.kind == CliKind::Pi
                        });
                        if detected { break; }
                    }
                    assert!(detected, "Pi launched from CMD must be detected");
                    // Pi ignores prompts while its startup is still in progress.
                    let mut ready = false;
                    for _ in 0..300 {
                        smol::Timer::after(Duration::from_millis(100)).await;
                        ready = cx.update(|cx| {
                            let ui = view.read(cx);
                            let buffer = ui.session.terminals[&id].buffer.lock().unwrap();
                            let text = buffer.parser.screen().contents();
                            text.contains("Ask it how to use")
                                && !text.contains("Startup is still in progress")
                        });
                        if ready { break; }
                    }
                    assert!(ready, "Pi startup must finish before the prompt is submitted");
                    handle
                        .update(cx, |_, _, cx| {
                            view.update(cx, |ui, _| {
                                ui.session.send(b"say hi\r".to_vec());
                            })
                        })
                        .unwrap();
                    let mut seen = Vec::new();
                    for _ in 0..400 {
                        smol::Timer::after(Duration::from_millis(100)).await;
                        let current = cx.update(|cx| view.read(cx).agent_activity.get(&id).copied());
                        if let Some(current) = current
                            && seen.last() != Some(&current)
                        {
                            seen.push(current);
                        }
                        if seen.iter().any(|indicator| {
                            !matches!(indicator, vibe_terminal::activity::Indicator::None)
                        }) {
                            break;
                        }
                    }
                    let screen = cx.update(|cx| {
                        let ui = view.read(cx);
                        let buffer = ui.session.terminals[&id].buffer.lock().unwrap();
                        buffer.parser.screen().contents()
                    });
                    println!("seen={seen:?}\nscreen:\n{screen}");
                    assert!(
                        seen.iter().any(|indicator| !matches!(indicator, vibe_terminal::activity::Indicator::None)),
                        "CMD-launched Pi must publish a tab status while working: {seen:?}"
                    );
                    let rendered = cx.update(|cx| view.read(cx).agent_activity_indicator(id, "tab").is_some());
                    assert!(rendered, "tab strip must render the status slot: {seen:?}");
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
    fs::remove_dir_all(root).unwrap();
}
