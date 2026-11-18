use super::{Arc, AtomicBool, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use gpui_kit::{AppContext, Bounds, ClipboardItem, WindowBounds, WindowOptions, point, px, size};
use std::{fs, sync::Mutex, thread, time::Instant};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetMenuItemCount, PostMessageW, SendMessageW, WM_CANCELMODE,
    WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
};

// Native menus run a Win32 tracking loop. Drive them from a separate thread so
// cancellation and menu selection do not depend on the blocked GPUI executor.
fn probe_menu(x: i32, y: i32, expect_menu: bool) -> thread::JoinHandle<usize> {
    thread::spawn(move || unsafe {
        let title: Vec<u16> = "Vibe right click regression\0".encode_utf16().collect();
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        let location = ((y as u32) << 16 | x as u32) as isize;
        PostMessageW(hwnd, WM_MOUSEMOVE, 0, location);
        thread::sleep(Duration::from_millis(40));
        PostMessageW(hwnd, WM_RBUTTONDOWN, 2, location);
        PostMessageW(hwnd, WM_RBUTTONUP, 0, location);
        let class: Vec<u16> = "#32768\0".encode_utf16().collect();
        let deadline = Instant::now() + Duration::from_millis(if expect_menu { 1800 } else { 450 });
        while Instant::now() < deadline {
            let popup = FindWindowW(class.as_ptr(), std::ptr::null());
            if !popup.is_null() {
                thread::sleep(Duration::from_millis(100));
                let menu = SendMessageW(popup, 0x01e1, 0, 0); // MN_GETHMENU
                let count = GetMenuItemCount(menu as _) as usize;
                PostMessageW(hwnd, WM_CANCELMODE, 0, 0);
                return count;
            }
            thread::sleep(Duration::from_millis(15));
        }
        PostMessageW(hwnd, WM_CANCELMODE, 0, 0);
        0
    })
}

#[test]
#[ignore = "opens a native terminal/Agent composer and operates Windows context menus"]
fn native_right_click_setting_controls_terminal_and_agent_input() {
    let root = std::env::temp_dir().join(format!("vibe-right-click-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let helper = root.join("keys.cjs");
    let receipt = root.join("input.bin");
    fs::write(&helper, "const fs=require('node:fs');process.stdin.setRawMode(true);process.stdin.on('data',b=>fs.appendFileSync(process.argv[2],b));").unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(vibe_terminal::store::Store { root: root.clone() }, data, None);
    session.new_tab(CliProfile {
        id: "right-click-fixture".into(), name: "Pi fixture".into(), command: "node".into(),
        args: vec![helper.to_string_lossy().into_owned(), receipt.to_string_lossy().into_owned()],
        kind: CliKind::Pi,
    });
    let id = session.selected().unwrap();
    let results = Arc::new(Mutex::new(Vec::new()));
    let observations = results.clone();
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let clipboard_backup = cx.read_from_clipboard();
        let (handle, view) = gpui_kit::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(80.0), px(80.0)), size(px(1200.0), px(800.0))))),
            ..Default::default()
        }, cx, |window, cx| {
            window.set_window_title("Vibe right click regression");
            cx.new(|cx| { let mut ui = Ui::new(session, window, cx); ui.demo = true; ui })
        }).unwrap();
        cx.spawn(async move |cx| {
            smol::Timer::after(Duration::from_millis(800)).await;
            for composer in [false, true] {
                if composer {
                    handle.update(cx, |_, window, cx| view.update(cx, |ui, cx| {
                        ui.toggle_transcript(window, cx);
                        ui.conversations[&id].input.update(cx, |input, cx| {
                            input.set_value("draft", window, cx);
                            input.select_all(window, cx);
                        });
                    })).unwrap();
                    smol::Timer::after(Duration::from_millis(300)).await;
                }
                for enabled in [false, true, false] {
                    handle.update(cx, |_, window, cx| view.update(cx, |ui, cx| {
                        ui.set_right_click_menu(enabled, cx);
                        ui.focus_current_view(window, cx);
                        cx.write_to_clipboard(ClipboardItem::new_string("right-click-text".into()));
                    })).unwrap();
                    smol::Timer::after(Duration::from_millis(150)).await;
                    let (x, y) = handle.update(cx, |_, window, cx| {
                        let ui = view.read(cx);
                        let position = if composer {
                            ui.conversations[&id].input.read(cx).cursor_layout().unwrap().0.center()
                        } else {
                            ui.bounds.center()
                        };
                        let scale = window.scale_factor();
                        ((f32::from(position.x) * scale) as i32, (f32::from(position.y) * scale) as i32)
                    }).unwrap();
                    let probe = probe_menu(x, y, enabled);
                    smol::Timer::after(Duration::from_millis(2300)).await;
                    let count = probe.join().unwrap();
                    observations.lock().unwrap().push((format!("menu composer={composer} enabled={enabled}"), (count > 0) == enabled));
                    if enabled {
                        // Exercise the exact actions offered by the native menus,
                        // without redirecting the user's global keyboard focus.
                        handle.update(cx, |_, window, cx| {
                            if composer {
                                window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx);
                            } else {
                                window.dispatch_action(Box::new(super::super::context_menu::PasteTerminal), cx);
                            }
                        }).unwrap();
                        smol::Timer::after(Duration::from_millis(200)).await;
                        let pasted = if composer {
                            cx.update(|cx| view.read(cx).conversations[&id].input.read(cx).value().contains("right-click-text"))
                        } else {
                            fs::read(&receipt).unwrap_or_default().windows(16).any(|bytes| bytes == b"right-click-text")
                        };
                        observations.lock().unwrap().push((format!("menu Paste action composer={composer}"), pasted));
                    }
                }
            }
            handle.update(cx, |_, _, cx| view.update(cx, |ui, _| ui.session.shutdown())).unwrap();
            cx.update(|cx| {
                if let Some(backup) = clipboard_backup { cx.write_to_clipboard(backup); }
                cx.quit();
            });
            finished.store(true, Ordering::Release);
        }).detach();
    });
    assert!(passed.load(Ordering::Acquire));
    let observations = results.lock().unwrap();
    fs::remove_dir_all(root).unwrap();
    for (label, success) in observations.iter() {
        eprintln!("{label}: {success}");
    }
    assert!(observations.iter().all(|(_, success)| *success));
}
