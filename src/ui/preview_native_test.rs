use super::super::{
    Arc, CliProfile, Duration, FileLink, Ordering, PreviewAction, Session, Ui, Uuid,
};
use futures_util::FutureExt;
use gpui_kit::{AppContext, Bounds, Focusable, WindowBounds, WindowOptions, point, px, size};
use std::{fs, sync::atomic::AtomicBool};

#[test]
#[ignore = "opens a native Markdown preview and verifies editing, saving and conflict handling"]
fn native_markdown_preview_is_rendered_until_edit_and_refreshes_after_save() {
    let root = std::env::temp_dir().join(format!("vibe-markdown-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("README.MD");
    fs::write(&path, "# Preview title\n\n**Bold text**\n\n- List item\n").unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let tab = vibe_terminal::model::Tab::new(CliProfile::default_shell(), root.clone());
    let id = tab.id;
    data.workspace_mut().unwrap().tabs.push(tab);
    data.workspace_mut().unwrap().selected_tab = Some(id);
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    session.read_only = true;
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    let cleanup = root.clone();
    gpui_kit::application()
        .with_assets(crate::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.), px(100.)),
                        size(px(1280.), px(800.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe Markdown preview regression");
                    cx.new(|cx| {
                        let mut ui = Ui::new(session, window, cx);
                        ui.demo = true;
                        ui.panel.open = true;
                        ui
                    })
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                ui.request_preview_action(
                                    PreviewAction::File(FileLink {
                                        start: 0,
                                        end: 0,
                                        path: path.clone(),
                                        line: None,
                                        column: None,
                                    }),
                                    window,
                                    cx,
                                );
                                cx.notify();
                            })
                        })
                        .unwrap();
                    let mut loaded = false;
                    for _ in 0..100 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        loaded = cx.update(|cx| view.read(cx).markdown_preview().is_some());
                        if loaded {
                            break;
                        }
                    }
                    assert!(
                        loaded,
                        "path activation must open Markdown in rendered preview mode"
                    );
                    // Allow an actual Markdown frame to be laid out and painted.
                    smol::Timer::after(Duration::from_millis(200)).await;
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                assert!(!ui.preview_editing);
                                assert_eq!(ui.split_preview_owner, Some(id));
                                assert!(
                                    !ui.preview_editor
                                        .as_ref()
                                        .unwrap()
                                        .read(cx)
                                        .focus_handle(cx)
                                        .is_focused(window)
                                );
                                ui.toggle_preview_editing(window, cx);
                                assert!(ui.preview_editing && ui.markdown_preview().is_none());
                                ui.preview_editor
                                    .as_ref()
                                    .unwrap()
                                    .update(cx, |editor, cx| {
                                        editor.replace_all("# Updated title\n\n`code`", window, cx)
                                    });
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(150)).await;
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                assert!(ui.preview_dirty);
                                ui.toggle_preview_editing(window, cx);
                                assert!(!ui.preview_dirty && !ui.preview_editing);
                                assert_eq!(
                                    ui.markdown_preview().unwrap().file.content,
                                    "# Updated title\n\n`code`"
                                );
                                assert_eq!(
                                    fs::read_to_string(&path).unwrap(),
                                    "# Updated title\n\n`code`"
                                );
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(200)).await;
                    // External changes must still prevent finishing an unsaved edit.
                    fs::write(&path, "external change").unwrap();
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                ui.toggle_preview_editing(window, cx);
                                ui.preview_editor
                                    .as_ref()
                                    .unwrap()
                                    .update(cx, |editor, cx| {
                                        editor.replace_all("# Unsaved", window, cx)
                                    });
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(150)).await;
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                assert!(ui.preview_dirty);
                                ui.toggle_preview_editing(window, cx);
                                assert!(ui.preview_editing && ui.preview_dirty);
                                assert!(ui.markdown_preview().is_none());
                                assert!(ui.session.error.is_some());
                                assert_eq!(fs::read_to_string(&path).unwrap(), "external change");
                            })
                        })
                        .unwrap();
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
