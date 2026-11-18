use super::{Arc, AtomicBool, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use gpui_kit::{AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use std::{fs, sync::Mutex};
use vibe_terminal::activity::Indicator;

#[test]
#[ignore = "opens the installed Codex in a native window without submitting a request"]
fn native_codex_startup_has_no_task_spinner() {
    let root = std::env::temp_dir().join(format!("vibe-codex-startup-ui-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&std::env::current_dir().unwrap()).unwrap();
    let mut session = Session::new(vibe_terminal::store::Store { root: root.clone() }, data, None);
    session.new_tab(CliProfile {
        id: "codex-startup-ui".into(), name: "Codex".into(), command: "codex".into(),
        args: vec![], kind: CliKind::Codex,
    });
    let id = session.selected().unwrap();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let readings = observed.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let done = finished.clone();
    gpui_kit::application().with_assets(crate::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let (handle, view) = gpui_kit::open_window(WindowOptions {
            titlebar: Some(gpui_kit::component::TitleBar::title_bar_options()),
            app_owns_titlebar_drag: true,
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(100.0), px(100.0)), size(px(1280.0), px(800.0))))),
            ..Default::default()
        }, cx, |window, cx| {
            window.set_window_title("Vibe Codex startup regression");
            cx.new(|cx| Ui::new(session, window, cx))
        }).unwrap();
        cx.spawn(async move |cx| {
            for _ in 0..150 {
                smol::Timer::after(Duration::from_millis(100)).await;
                handle.update(cx, |_, _, cx| {
                    let ui = view.read(cx);
                    let state = ui.agent_activity.get(&id).copied().unwrap_or_default();
                    readings.lock().unwrap().push(state);
                }).unwrap();
            }
            handle.update(cx, |_, _, cx| view.update(cx, |ui, _| {
                let buffer = ui.session.terminals[&id].buffer.lock().unwrap();
                eprintln!("native status={}; session bound={}", buffer.parser.callbacks().activity.native(), ui.session.data.tab(id).unwrap().session_id.is_some());
                drop(buffer);
                ui.session.shutdown();
            })).unwrap();
            done.store(true, Ordering::Release);
            cx.update(|cx| cx.quit());
        }).detach();
    });
    fs::remove_dir_all(root).unwrap();
    assert!(finished.load(Ordering::Acquire));
    let readings = observed.lock().unwrap();
    assert!(!readings.is_empty());
    assert!(readings.iter().all(|state| *state == Indicator::None), "unexpected startup states: {readings:?}");
}
