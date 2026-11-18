use super::super::{Arc, Duration, Ordering, Session, Ui, Uuid};
use futures_util::FutureExt;
use gpui_kit::{AppContext, WindowOptions};
use std::{fs, sync::atomic::AtomicBool};

#[test]
#[ignore = "opens a native window to verify per-workspace panel visibility"]
fn native_right_panel_restores_each_workspaces_open_state() {
    let root = std::env::temp_dir().join(format!("vibe-workspace-panel-{}", Uuid::new_v4()));
    let a = root.join("a");
    let b = root.join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    let first = data.add_workspace(&a).unwrap();
    data.workspace_mut().unwrap().right_panel_open = true;
    let second = data.add_workspace(&b).unwrap();
    data.selected_workspace = Some(first);
    let store = vibe_terminal::store::Store { root: root.clone() };
    store.save(&data).unwrap();
    let mut session = Session::new(store, data, None);
    session.read_only = true;
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    let cleanup = root.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) =
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    window.set_window_title("Vibe workspace panel regression");
                    cx.new(|cx| Ui::new(session, window, cx))
                })
                .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    smol::Timer::after(Duration::from_millis(200)).await;
                    for _ in 0..3 {
                        handle
                            .update(cx, |_, window, cx| {
                                view.update(cx, |ui, cx| {
                                    assert!(ui.panel.open, "saved first workspace starts expanded");
                                    ui.request_workspace(second, window, cx);
                                    assert!(
                                        !ui.panel.open,
                                        "second workspace keeps its collapsed state"
                                    );
                                    cx.notify();
                                })
                            })
                            .unwrap();
                        smol::Timer::after(Duration::from_millis(100)).await;
                        handle
                            .update(cx, |_, window, cx| {
                                view.update(cx, |ui, cx| {
                                    ui.request_workspace(first, window, cx);
                                    assert!(
                                        ui.panel.open,
                                        "first workspace restores expanded state"
                                    );
                                    cx.notify();
                                })
                            })
                            .unwrap();
                        smol::Timer::after(Duration::from_millis(100)).await;
                    }
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                ui.set_project_panel_open(false);
                                ui.request_workspace(second, window, cx);
                                assert!(!ui.panel.open);
                                ui.set_project_panel_open(true);
                                ui.request_workspace(first, window, cx);
                                assert!(!ui.panel.open, "closing first must not close second");
                                ui.request_workspace(second, window, cx);
                                assert!(ui.panel.open);
                                ui.session.store.save(&ui.session.data).unwrap();
                                let restored = ui.session.store.load().unwrap();
                                assert!(super::ProjectPanel::for_workspace(&restored).open);
                                assert!(
                                    !restored
                                        .workspaces
                                        .iter()
                                        .find(|w| w.id == first)
                                        .unwrap()
                                        .right_panel_open
                                );
                                cx.notify();
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(100)).await;
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
