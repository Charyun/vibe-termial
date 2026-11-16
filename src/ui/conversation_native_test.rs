use super::{Arc, CliKind, CliProfile, Duration, Ordering, Session, Ui, Uuid};
use gpui_kit::{AppContext, Bounds, WindowBounds, WindowOptions, point, px, size};
#[cfg(all(test, windows))]
mod native_tests {
    use super::*;
    use std::{fs, sync::atomic::AtomicBool};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, PostMessageW, SendMessageW, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
    };

    #[test]
    #[ignore = "opens an isolated native conversation with a local echo Agent; no model requests"]
    fn native_content_input_reaches_same_pty_and_refreshes() {
        let root = std::env::temp_dir().join(format!("vibe-content-ui-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join("sessions")).unwrap();
        let helper = root.join("agent.cjs");
        fs::write(
            &helper,
            include_str!("../../tests/fixtures/conversation-agent.cjs"),
        )
        .unwrap();
        let mut data = vibe_terminal::model::AppData::default();
        data.add_workspace(&root).unwrap();
        let mut session = Session::new(
            vibe_terminal::store::Store { root: root.clone() },
            data,
            None,
        );
        let profile = CliProfile {
            id: "content-native".into(),
            name: "Pi fixture".into(),
            command: "node".into(),
            kind: CliKind::Pi,
            args: vec![helper.to_string_lossy().into_owned()],
        };
        session.new_tab(profile.clone());
        let tab_id = session.selected().unwrap();
        let original = session.terminals[&tab_id].buffer.clone();
        let passed = Arc::new(AtomicBool::new(false));
        let finished = passed.clone();
        gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
            gpui_kit::init(cx);
            let (handle, view) = gpui_kit::open_window(WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(100.0), px(100.0)), size(px(1200.0), px(800.0))))),
                ..Default::default()
            }, cx, |window,cx| {
                window.set_window_title("Vibe content input regression");
                cx.new(|cx| { let mut ui=Ui::new(session,window,cx); ui.demo=true; ui })
            }).unwrap();
            cx.spawn(async move |cx| {
                smol::Timer::after(Duration::from_millis(800)).await;
                handle.update(cx, |_,window,cx| view.update(cx, |ui,cx| ui.toggle_transcript(window,cx))).unwrap();
                smol::Timer::after(Duration::from_millis(250)).await;
                unsafe {
                    let title:Vec<u16>="Vibe content input regression\0".encode_utf16().collect();
                    let hwnd=FindWindowW(std::ptr::null(),title.as_ptr()); assert!(!hwnd.is_null());
                    for unit in "内容视图输入测试".encode_utf16() { SendMessageW(hwnd,WM_CHAR,unit as usize,0); }
                    PostMessageW(hwnd,WM_KEYDOWN,13,0); PostMessageW(hwnd,WM_KEYUP,13,0);
                }
                let mut received=false;
                for _ in 0..80 {
                    smol::Timer::after(Duration::from_millis(100)).await;
                    received=cx.update(|cx| {
                        let ui=view.read(cx); let content=&ui.conversations[&tab_id];
                        content.messages.len()==2 && content.messages[0].text=="内容视图输入测试"
                            && content.messages[1].text=="Echo: 内容视图输入测试" && content.pending.is_empty()
                            && content.input.read(cx).value().is_empty()
                    });
                    if received {break}
                }
                assert!(received,"native composer Enter must reach the existing PTY and refresh the conversation");
                let ticket=handle.update(cx, |_,window,cx| view.update(cx, |ui,cx| {
                    assert!(Arc::ptr_eq(&original,&ui.session.terminals[&tab_id].buffer));
                    ui.toggle_transcript(window,cx);
                    assert!(!ui.content_visible());
                    ui.session.begin_message(tab_id,"continued from terminal").unwrap()
                })).unwrap();
                smol::Timer::after(Duration::from_millis(180)).await;
                handle.update(cx, |_,window,cx| view.update(cx, |ui,cx| {
                    ui.session.finish_message(ticket).unwrap(); ui.toggle_transcript(window,cx);
                })).unwrap();
                let mut synchronized=false;
                for _ in 0..80 {
                    smol::Timer::after(Duration::from_millis(100)).await;
                    synchronized=cx.update(|cx| view.read(cx).conversations[&tab_id].messages.len()==4);
                    if synchronized {break}
                }
                assert!(synchronized,"switching forms must retain and refresh the same conversation");
                handle.update(cx, |_,window,cx| view.update(cx, |ui,cx| {
                    let first_input=ui.conversations[&tab_id].input.clone();
                    first_input.update(cx, |input,cx| input.set_value("draft one",window,cx));
                    ui.new_terminal_with(profile,window,cx);
                    let second=ui.session.selected().unwrap(); ui.toggle_transcript(window,cx);
                    assert!(ui.conversations[&second].input.read(cx).value().is_empty());
                    ui.select_tab(tab_id,window,cx);
                    assert!(ui.content_visible());
                    assert_eq!(ui.conversations[&tab_id].input.read(cx).value().as_ref(),"draft one");
                    assert!(Arc::ptr_eq(&original,&ui.session.terminals[&tab_id].buffer));
                    ui.session.shutdown();
                })).unwrap();
                finished.store(true,Ordering::Release);
                cx.update(|cx|cx.quit());
            }).detach();
        });
        assert!(passed.load(Ordering::Acquire));
        fs::remove_dir_all(root).unwrap();
    }
}
