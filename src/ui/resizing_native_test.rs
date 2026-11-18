use super::super::{Arc, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use futures_util::FutureExt;
use gpui_kit::{
    AppContext, Bounds, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput,
    WindowBounds, WindowOptions, point, px, size,
};
use std::{fs, sync::atomic::AtomicBool};

#[test]
#[ignore = "opens a native window and drags both side-pane dividers"]
fn native_side_panes_drag_persist_and_preserve_center_on_resize() {
    let root = std::env::temp_dir().join(format!("vibe-pane-widths-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("history")).unwrap();
    fs::create_dir_all(root.join("sessions")).unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let tab = vibe_terminal::model::Tab::new(CliProfile::default_shell(), root.clone());
    let id = tab.id;
    data.workspace_mut().unwrap().tabs.push(tab);
    data.workspace_mut().unwrap().selected_tab = Some(id);
    data.workspace_mut().unwrap().right_panel_open = true;
    let store = vibe_terminal::store::Store { root: root.clone() };
    store
        .save_history(id, b"Drag the left and right pane dividers\r\n")
        .unwrap();
    let mut session = Session::new(store, data, None);
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
                    window.set_window_title("Vibe pane resize regression");
                    cx.new(|cx| {
                        let mut ui = Ui::new(session, window, cx);
                        ui.demo = true;
                        ui.session.read_only = false;
                        ui
                    })
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    smol::Timer::after(Duration::from_millis(250)).await;
                    // GPUI platform events exercise hit testing and the actual
                    // divider/capture handlers; no direct resize method calls.
                    for right in [false, true] {
                        handle
                            .update(cx, |_, window, cx| {
                                let start = {
                                    let ui = view.read(cx);
                                    if right {
                                        f32::from(window.viewport_size().width)
                                            - ui.preview_width
                                            - 2.
                                    } else {
                                        ui.sidebar_width + 2.
                                    }
                                };
                                let end = start + if right { -80. } else { 80. };
                                window.dispatch_event(
                                    PlatformInput::MouseMove(MouseMoveEvent {
                                        position: point(px(start), px(200.)),
                                        ..Default::default()
                                    }),
                                    cx,
                                );
                                window.dispatch_event(
                                    PlatformInput::MouseDown(MouseDownEvent {
                                        button: MouseButton::Left,
                                        position: point(px(start), px(200.)),
                                        click_count: 1,
                                        ..Default::default()
                                    }),
                                    cx,
                                );
                                assert!(
                                    view.read(cx).pane_resize.is_some(),
                                    "divider must receive mouse down"
                                );
                                window.dispatch_event(
                                    PlatformInput::MouseMove(MouseMoveEvent {
                                        position: point(px(end), px(200.)),
                                        pressed_button: Some(MouseButton::Left),
                                        ..Default::default()
                                    }),
                                    cx,
                                );
                                window.dispatch_event(
                                    PlatformInput::MouseUp(MouseUpEvent {
                                        button: MouseButton::Left,
                                        position: point(px(end), px(200.)),
                                        click_count: 1,
                                        ..Default::default()
                                    }),
                                    cx,
                                );
                                assert!(
                                    view.read(cx).pane_resize.is_none(),
                                    "release must finish dragging"
                                );
                            })
                            .unwrap();
                        smol::Timer::after(Duration::from_millis(150)).await;
                    }
                    handle
                        .update(cx, |_, window, cx| {
                            let ui = view.read(cx);
                            assert_eq!(ui.sidebar_width, 304.);
                            assert_eq!(ui.preview_width, 520.);
                            let stored = ui.session.store.load().unwrap();
                            assert_eq!(
                                (stored.sidebar_width, stored.right_panel_width),
                                (304., 520.)
                            );
                            assert!(f32::from(ui.bounds.size.width) >= 320.);
                            window.resize(size(px(900.), px(600.)));
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(250)).await;
                    handle
                        .update(cx, |_, window, cx| {
                            let ui = view.read(cx);
                            assert!(
                                ui.sidebar_width + ui.preview_width + 328.
                                    <= f32::from(window.viewport_size().width)
                            );
                            assert_eq!(ui.session.data.sidebar_width, 304.);
                            assert_eq!(ui.session.data.right_panel_width, 520.);
                            window.resize(size(px(1280.), px(800.)));
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(250)).await;
                    handle
                        .update(cx, |_, _, cx| {
                            let ui = view.read(cx);
                            assert_eq!((ui.sidebar_width, ui.preview_width), (304., 520.));
                            assert!(f32::from(ui.bounds.size.width) >= 320.);
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
