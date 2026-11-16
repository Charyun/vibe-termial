use super::{Arc, CliKind, CliProfile, Duration, Ordering, PathBuf, Session, Ui, Uuid};
use futures_util::FutureExt;
use gpui_kit::{
    AppContext, Bounds, ClipboardItem, Image, ImageFormat, WindowBounds, WindowOptions, point, px,
    size,
};
use std::{fs, sync::atomic::AtomicBool};
fn native_control_v() {
    use windows_sys::Win32::System::Threading::AttachThreadInput;
    use windows_sys::Win32::UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL,
            VK_V,
        },
        WindowsAndMessaging::{
            FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow,
        },
    };
    unsafe {
        let title: Vec<u16> = "Vibe clipboard regression\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        assert!(!hwnd.is_null());
        let current_thread = windows_sys::Win32::System::Threading::GetCurrentThreadId();
        let foreground_thread =
            GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let attached = current_thread != foreground_thread
            && AttachThreadInput(current_thread, foreground_thread, 1) != 0;
        SetForegroundWindow(hwnd);
        if attached {
            AttachThreadInput(current_thread, foreground_thread, 0);
        }
        assert_eq!(
            GetForegroundWindow(),
            hwnd,
            "test window must receive the keyboard input"
        );
        let input: Vec<INPUT> = [
            (VK_CONTROL, 0),
            (VK_V, 0),
            (VK_V, KEYEVENTF_KEYUP),
            (VK_CONTROL, KEYEVENTF_KEYUP),
        ]
        .into_iter()
        .map(|(key, flags)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        })
        .collect();
        assert_eq!(
            SendInput(
                input.len() as u32,
                input.as_ptr(),
                std::mem::size_of::<INPUT>() as i32
            ),
            input.len() as u32
        );
    }
}
#[test]
#[ignore = "uses a native window, real system image clipboard and local PTY fixture; no model requests"]
fn native_image_clipboard_reaches_agent_or_stages_a_readable_path() {
    let root = std::env::temp_dir().join(format!("vibe-clipboard-ui-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let helper = root.join("agent.cjs");
    fs::write(
        &helper,
        include_str!("../../tests/fixtures/clipboard-agent.cjs"),
    )
    .unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() },
        data,
        None,
    );
    let receipt = root.join("native.bin");
    let profile = CliProfile {
        id: "clipboard-native".into(),
        name: "Clipboard fixture".into(),
        command: "node".into(),
        args: vec![
            helper.to_string_lossy().into_owned(),
            receipt.to_string_lossy().into_owned(),
        ],
        kind: CliKind::Custom,
    };
    session.new_tab(profile.clone());
    let first = session.selected().unwrap();
    session.data.tab_mut(first).unwrap().profile.kind = CliKind::Codex;
    let mut image_bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 3)
        .write_to(&mut image_bytes, image::ImageFormat::Bmp)
        .unwrap();
    let clipboard = ClipboardItem::new_image(&Image::from_bytes(
        ImageFormat::Bmp,
        image_bytes.into_inner(),
    ));
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    let cleanup_root = root.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let backup = cx.read_from_clipboard();
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
                    window.set_window_title("Vibe clipboard regression");
                    cx.new(|cx| {
                        let mut ui = Ui::new(session, window, cx);
                        ui.demo = true;
                        ui
                    })
                },
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let outcome = std::panic::AssertUnwindSafe(async {
                    smol::Timer::after(Duration::from_millis(600)).await;
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                cx.write_to_clipboard(clipboard.clone());
                                ui.focus_current_view(window, cx);
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(100)).await;
                    native_control_v();
                    let mut native = false;
                    for _ in 0..60 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        native = fs::read(&receipt).is_ok_and(|bytes| bytes == b"\x16");
                        if native {
                            break;
                        }
                    }
                    assert!(
                        native,
                        "image Ctrl+V must reach Codex as a raw native shortcut, without Enter"
                    );
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                ui.toggle_transcript(window, cx);
                                assert!(ui.content_visible());
                                assert!(ui.conversations[&first].input.read(cx).value().is_empty());
                                assert!(ui.native_images.contains_key(&first));
                                ui.send_content_message(first, window, cx);
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(350)).await;
                    assert_eq!(
                        fs::read(&receipt).unwrap(),
                        b"\x16\r",
                        "image-only message submits only after the explicit Send action"
                    );
                    let second_receipt = root.join("fallback.bin");
                    let mut fallback_profile = profile.clone();
                    fallback_profile.args[1] = second_receipt.to_string_lossy().into_owned();
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                ui.new_terminal_with(fallback_profile, window, cx)
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(500)).await;
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                cx.write_to_clipboard(clipboard.clone());
                                ui.focus_current_view(window, cx);
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(100)).await;
                    native_control_v();
                    let mut pasted = Vec::new();
                    for _ in 0..60 {
                        smol::Timer::after(Duration::from_millis(50)).await;
                        pasted = fs::read(&second_receipt).unwrap_or_default();
                        if pasted.ends_with(b"\x1b[201~") {
                            break;
                        }
                    }
                    let text = String::from_utf8(pasted).unwrap();
                    assert!(
                        text.starts_with("\x1b[200~\"") && text.ends_with("\" \x1b[201~"),
                        "fallback must paste a quoted path, not a native key: {text:?}"
                    );
                    assert!(!text.contains(['\r', '\n']));
                    let path = PathBuf::from(
                        text.trim_start_matches("\x1b[200~\"")
                            .trim_end_matches("\" \x1b[201~"),
                    );
                    assert_eq!(
                        (
                            image::open(&path).unwrap().width(),
                            image::open(&path).unwrap().height()
                        ),
                        (4, 3)
                    );
                    fs::remove_file(&path).unwrap();
                    // The conversation composer takes the same native image path.
                    handle
                        .update(cx, |_, window, cx| {
                            view.update(cx, |ui, cx| {
                                ui.select_tab(first, window, cx);
                                assert!(ui.content_visible());
                                cx.write_to_clipboard(clipboard.clone());
                                assert!(ui.image_composer_target(window, cx).is_some());
                            })
                        })
                        .unwrap();
                    smol::Timer::after(Duration::from_millis(150)).await;
                    native_control_v();
                    smol::Timer::after(Duration::from_millis(150)).await;
                    cx.update(|cx| {
                        let ui = view.read(cx);
                        assert!(ui.conversations[&first].input.read(cx).value().is_empty(), "composer contains {} characters; native image registered: {}; PTY received {} bytes", ui.conversations[&first].input.read(cx).value().chars().count(), ui.native_images.contains_key(&first), fs::metadata(&receipt).unwrap().len());
                        assert!(ui.native_images.contains_key(&first));
                    });
                    assert_eq!(fs::read(&receipt).unwrap(), b"\x16\r\x16");
                })
                .catch_unwind()
                .await;
                let _ = handle.update(cx, |_, _, cx| {
                    view.update(cx, |ui, _| ui.session.shutdown())
                });
                cx.update(|cx| {
                    cx.write_to_clipboard(
                        backup.unwrap_or_else(|| ClipboardItem::new_string(String::new())),
                    );
                });
                finished.store(outcome.is_ok(), Ordering::Release);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    assert!(passed.load(Ordering::Acquire));
    fs::remove_dir_all(cleanup_root).unwrap();
}
