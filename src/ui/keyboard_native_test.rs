use super::{Arc, AtomicBool, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use gpui_kit::{AppContext, KeyDownEvent, Keystroke, PlatformInput, WindowOptions};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, WM_KEYDOWN, WM_KEYUP,
};

#[test]
#[ignore = "opens an isolated native window and a raw-mode Node PTY"]
fn native_terminal_keys_bypass_gui_shortcuts() {
    let root = std::env::temp_dir().join(format!("vibe-keyboard-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let helper = root.join("keys.cjs");
    let output = root.join("keys.bin");
    let ready = root.join("ready");
    std::fs::write(&helper, r#"
const fs = require('node:fs');
process.stdin.setRawMode(true);
process.stdin.on('data', bytes => fs.appendFileSync(process.argv[2], bytes));
fs.writeFileSync(process.argv[3], 'ready');
"#).unwrap();
    let mut data = vibe_terminal::model::AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(
        vibe_terminal::store::Store { root: root.clone() }, data, None,
    );
    session.new_tab(CliProfile {
        id: "keyboard-fixture".into(), name: "Keyboard fixture".into(),
        command: "node".into(), kind: CliKind::Shell,
        args: vec![helper, output.clone(), ready.clone()].iter()
            .map(|path| path.to_string_lossy().into_owned()).collect(),
    });
    let id = session.selected().unwrap();
    let passed = Arc::new(AtomicBool::new(false));
    let finished = passed.clone();
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let (handle, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            window.set_window_title("Vibe terminal keyboard regression");
            cx.new(|cx| { let mut ui = Ui::new(session, window, cx); ui.demo = true; ui })
        }).unwrap();
        cx.spawn(async move |cx| {
            for _ in 0..100 {
                if ready.exists() { break; }
                smol::Timer::after(Duration::from_millis(50)).await;
            }
            assert!(ready.exists(), "raw PTY fixture must start");
            smol::Timer::after(Duration::from_millis(300)).await;
            // Native Windows key messages exercise GPUI's actual key pump.
            unsafe {
                let title: Vec<u16> = "Vibe terminal keyboard regression\0".encode_utf16().collect();
                let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                assert!(!hwnd.is_null());
                for key in [9, 27, 112, 38] { // Tab, Esc, F1, Up
                    PostMessageW(hwnd, WM_KEYDOWN, key, 0);
                    PostMessageW(hwnd, WM_KEYUP, key, 0);
                }
            }
            smol::Timer::after(Duration::from_millis(300)).await;
            // Dispatch modifier combinations through the same GPUI key binding
            // resolver without changing the user's global keyboard state.
            handle.update(cx, |_, window, cx| {
                assert!(view.read(cx).terminal_has_keyboard_focus(window));
                for key in ["ctrl-w", "ctrl-t", "ctrl-c", "ctrl-v", "ctrl-tab", "shift-tab", "ctrl-/"] {
                    window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent {
                        keystroke: Keystroke::parse(key).unwrap(), is_held: false,
                        prefer_character_input: false,
                    }), cx);
                }
            }).unwrap();
            let expected = b"\t\x1b\x1bOP\x1b[A\x17\x14\x03\x16\t\x1b[Z\x1f";
            for _ in 0..100 {
                if std::fs::read(&output).unwrap_or_default().len() >= expected.len() { break; }
                smol::Timer::after(Duration::from_millis(50)).await;
            }
            let actual = std::fs::read(&output).unwrap_or_default();
            handle.update(cx, |_, window, cx| view.update(cx, |ui, _| {
                assert_eq!(ui.session.selected(), Some(id));
                assert_eq!(ui.session.data.workspace().unwrap().tabs.len(), 1);
                assert!(ui.terminal_has_keyboard_focus(window));
                assert!(!ui.settings && ui.dialog.is_none());
                ui.session.shutdown();
            })).unwrap();
            assert_eq!(actual, expected, "every key must reach the PTY exactly once");
            finished.store(true, Ordering::Release);
            cx.update(|cx| cx.quit());
        }).detach();
    });
    assert!(passed.load(Ordering::Acquire));
    std::fs::remove_dir_all(root).unwrap();
}
