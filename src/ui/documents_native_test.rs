use super::{
    Arc, CliProfile, Dialog, DocumentContent, Duration, FileLink, Ordering, PreviewAction, Session,
    Ui, Uuid,
};
use gpui_kit::{AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
use std::{fs, sync::atomic::AtomicBool};
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW, WM_CHAR};
fn link(path: &std::path::Path) -> FileLink {
    FileLink {
        start: 0,
        end: 0,
        path: path.to_path_buf(),
        line: None,
        column: None,
    }
}
#[test]
#[ignore = "opens an isolated native file-tab and split-preview window"]
fn native_document_tabs_preserve_editors_and_terminal_split_ownership() {
    let root = std::env::temp_dir().join(format!("vibe-documents-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let a = root.join("a.rs");
    let b = root.join("b.txt");
    let picture = root.join("image.png");
    fs::write(&a, b"\xef\xbb\xbffn main() {}\r\n").unwrap();
    fs::write(&b, "second file\n").unwrap();
    image::DynamicImage::new_rgb8(4, 3).save(&picture).unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    let workspace = data.add_workspace(&root).unwrap();
    let tab = vibe_terminal::model::Tab::new(CliProfile::default_shell(), root.clone());
    let terminal = tab.id;
    data.workspace_mut().unwrap().tabs.push(tab);
    data.workspace_mut().unwrap().selected_tab = Some(terminal);
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    session.read_only = true;
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    let cleanup_root = root.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(100.0), px(100.0)),
                        size(px(1280.0), px(800.0)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("Vibe file tabs regression");
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
                smol::Timer::after(Duration::from_millis(400)).await;
                let (first, second) = handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            ui.open_document(link(&a), window, cx);
                            let first = ui.active_document.unwrap();
                            ui.open_document(link(&b), window, cx);
                            (first, ui.active_document.unwrap())
                        })
                    })
                    .unwrap();
                let mut loaded = false;
                for _ in 0..60 {
                    smol::Timer::after(Duration::from_millis(50)).await;
                    loaded = cx.update(|cx| {
                        view.read(cx)
                            .documents
                            .iter()
                            .all(|tab| tab.editor.is_some())
                    });
                    if loaded {
                        break;
                    }
                }
                assert!(
                    loaded,
                    "both asynchronous file loads must populate their own editor"
                );
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            assert_eq!(ui.documents.len(), 2);
                            assert_eq!(ui.active_document, Some(second));
                            assert_eq!(ui.session.selected(), Some(terminal));
                            assert_eq!(ui.documents[0].title, "a.rs");
                            assert_eq!(ui.documents[1].title, "b.txt");
                            ui.open_document(link(&root.join(".").join("a.rs")), window, cx);
                            assert_eq!(ui.documents.len(), 2);
                            assert_eq!(ui.active_document, Some(first));
                            ui.documents[0].editing = true;
                            ui.focus_current_view(window, cx);
                            cx.notify();
                        })
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(200)).await;
                unsafe {
                    let title: Vec<_> = "Vibe file tabs regression\0".encode_utf16().collect();
                    let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                    assert!(!hwnd.is_null());
                    for unit in "中文".encode_utf16() {
                        SendMessageW(hwnd, WM_CHAR, unit as usize, 0);
                    }
                }
                smol::Timer::after(Duration::from_millis(200)).await;
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            assert!(ui.documents[0].dirty);
                            assert!(!ui.preview_dirty);
                            assert!(
                                ui.documents[0]
                                    .editor
                                    .as_ref()
                                    .unwrap()
                                    .read(cx)
                                    .value()
                                    .contains("中文")
                            );
                            ui.activate_document(second, window, cx);
                            assert!(ui.dialog.is_none());
                            assert!(ui.documents[0].dirty);
                            assert!(!ui.confirm_window_close(window, cx));
                            assert_eq!(ui.pending_document, Some(first));
                            ui.close_dialog(window, cx);
                            ui.activate_document(second, window, cx);
                            ui.request_close_document(first, window, cx);
                            assert!(matches!(ui.dialog, Some(Dialog::UnsavedPreview)));
                            assert_eq!(ui.pending_document, Some(first));
                            ui.close_dialog(window, cx);
                            assert_eq!(ui.documents.len(), 2);
                            assert!(ui.documents[0].dirty);
                            fs::write(&a, "external change").unwrap();
                            assert!(!ui.save_document(first, cx));
                            assert!(ui.documents[0].dirty);
                            fs::write(&a, b"\xef\xbb\xbffn main() {}\r\n").unwrap();
                            ui.request_close_document(first, window, cx);
                            ui.finish_pending_preview(true, window, cx);
                            assert_eq!(ui.documents.len(), 1);
                            assert_eq!(ui.active_document, Some(second));
                            let saved = fs::read(&a).unwrap();
                            assert!(saved.starts_with(b"\xef\xbb\xbf"));
                            assert!(String::from_utf8_lossy(&saved).contains("中文"));
                            assert!(saved.ends_with(b"\r\n"));
                            ui.select_tab(terminal, window, cx);
                            ui.request_preview_action(PreviewAction::File(link(&a)), window, cx);
                        })
                    })
                    .unwrap();
                let mut split = false;
                for _ in 0..60 {
                    smol::Timer::after(Duration::from_millis(50)).await;
                    split = cx.update(|cx| {
                        let ui = view.read(cx);
                        ui.preview_editor.is_some() && ui.terminal_split_visible()
                    });
                    if split {
                        break;
                    }
                }
                assert!(
                    split,
                    "terminal file links must open a terminal-owned split"
                );
                smol::Timer::after(Duration::from_millis(150)).await;
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            assert_eq!(ui.split_preview_owner, Some(terminal));
                            assert!(
                                f32::from(ui.bounds.size.width) < 500.0,
                                "terminal must shrink inside the main content area"
                            );
                            ui.activate_document(second, window, cx);
                            assert!(!ui.terminal_split_visible());
                            assert_eq!(
                                ui.documents[0]
                                    .editor
                                    .as_ref()
                                    .unwrap()
                                    .read(cx)
                                    .value()
                                    .as_ref(),
                                "second file\n"
                            );
                            ui.select_tab(terminal, window, cx);
                            assert!(ui.terminal_split_visible());
                            ui.perform_preview_action(PreviewAction::Close, window, cx);
                            assert!(ui.preview.is_none());
                            assert_eq!(ui.documents.len(), 1);
                            ui.open_document(link(&picture), window, cx);
                        })
                    })
                    .unwrap();
                smol::Timer::after(Duration::from_millis(300)).await;
                handle
                    .update(cx, |_, window, cx| {
                        view.update(cx, |ui, cx| {
                            assert!(matches!(
                                &ui.documents.last().unwrap().content,
                                Some(DocumentContent::Image(_))
                            ));
                            assert_eq!(ui.main_tabs().len(), 3);
                            assert!(ui.documents.iter().all(|tab| tab.workspace == workspace));
                            ui.close_document(second, window, cx);
                            assert_eq!(ui.session.data.workspace().unwrap().tabs.len(), 1);
                            ui.session.shutdown();
                        })
                    })
                    .unwrap();
                finished.store(true, Ordering::Release);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(passed.load(Ordering::Acquire));
    fs::remove_dir_all(cleanup_root).unwrap();
}
