//! Explicit integration check against the installed Pi CLI. No model request is made.
use serde_json::json;
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use vibe_terminal::{
    model::{CliKind, CliProfile, Tab},
    scroll::{MouseButtonEvent, mouse_button_sequence, mouse_wheel_sequence},
    terminal::Terminal,
};

#[test]
#[ignore = "requires the Pi CLI; run with an isolated PI_CODING_AGENT_DIR"]
fn real_pi_renders_seeded_history_and_receives_forwarded_mouse() {
    let root = std::env::temp_dir().join(format!("vibe-pi-scroll-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let session = root.join("session.jsonl");
    let now = "2026-11-14T08:00:00.000Z";
    let mut entries = vec![
        json!({"type":"session","version":3,"id":uuid::Uuid::new_v4().to_string(),"timestamp":now,"cwd":root}),
    ];
    let mut parent: Option<String> = None;
    for turn in 0..12 {
        let user = format!("user{turn:04}");
        entries.push(json!({"type":"message","id":user,"parentId":parent,"timestamp":now,"message":{"role":"user","content":[{"type":"text","text":format!("PI test prompt {turn:03}")}],"timestamp":1794643200000u64}}));
        let assistant = format!("resp{turn:04}");
        let text = (0..30)
            .map(|line| format!("PI_SCROLL_{turn:03}_{line:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        entries.push(json!({"type":"message","id":assistant,"parentId":user,"timestamp":now,"message":{"role":"assistant","content":[{"type":"text","text":text}],"api":"openai-completions","provider":"openai","model":"gpt-4o-mini","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":1794643200000u64}}));
        parent = Some(assistant);
    }
    fs::write(
        &session,
        entries
            .into_iter()
            .map(|entry| serde_json::to_string(&entry).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();
    let profile = CliProfile {
        id: "pi-test".into(),
        name: "Pi".into(),
        command: "pi".into(),
        kind: CliKind::Custom,
        args: vec![
            "--session".into(),
            session.to_string_lossy().into_owned(),
            "--tui-mode".into(),
            "fullscreen".into(),
            "--no-extensions".into(),
            "--no-skills".into(),
            "--no-prompt-templates".into(),
        ],
    };
    let tab = Tab::new(profile, root.clone());
    let mut term = Terminal::start(&tab, &root, 30, 110, Arc::new(|| {})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let contents = term.buffer.lock().unwrap().parser.screen().contents();
        if contents.contains("PI_SCROLL_") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Pi did not render the seeded conversation: {contents}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let sequence = {
        let b = term.buffer.lock().unwrap();
        // Pi 1.1.0 在 ConPTY 下内联绘制：备用屏与鼠标模式都看不到，滚轮仍必须转发。
        mouse_wheel_sequence(b.parser.screen(), true, 10, 10, false, false, true)
            .expect("fullscreen CLI wheel must be forwarded")
    };
    assert_eq!(sequence, b"\x1b[<64;11;11M");
    term.send(sequence.repeat(12)).unwrap();
    wait_for_screen(&term, |text| text.contains("Jump to latest message"));

    // Find Pi's actual scrollbar thumb, then drag it to both ends.
    let (thumb_row, thumb_col) = {
        let buffer = term.buffer.lock().unwrap();
        let screen = buffer.parser.screen();
        let (rows, cols) = screen.size();
        (0..cols)
            .rev()
            .find_map(|col| {
                (0..rows).find_map(|row| {
                    screen
                        .cell(row, col)
                        .filter(|cell| matches!(cell.contents(), "┃" | "█"))
                        .map(|_| (row, col))
                })
            })
            .expect("Pi must render a scrollbar thumb after scrolling")
    };
    // Auto-hidden scrollbar must reappear on pointer hover and use the thick
    // active thumb without requiring either a wheel event or a button press.
    wait_for_screen(&term, |text| !text.contains('┃') && !text.contains('█'));
    send_mouse(&term, MouseButtonEvent::Move, thumb_row, thumb_col);
    wait_for_screen(&term, |text| text.contains('█'));
    std::thread::sleep(Duration::from_millis(1250));
    assert!(
        term.buffer
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains('█'),
        "hover must keep the scrollbar visible beyond the auto-hide delay"
    );
    send_mouse(&term, MouseButtonEvent::Move, 30, 110);
    wait_for_screen(&term, |text| !text.contains('┃') && !text.contains('█'));
    send_mouse(&term, MouseButtonEvent::Move, thumb_row, thumb_col);
    wait_for_screen(&term, |text| text.contains('█'));
    send_mouse(&term, MouseButtonEvent::Press, thumb_row, thumb_col);
    send_mouse(&term, MouseButtonEvent::Drag, 0, thumb_col);
    wait_for_screen(&term, |text| text.contains("PI_SCROLL_000_000"));
    send_mouse(&term, MouseButtonEvent::Drag, 29, thumb_col);
    send_mouse(&term, MouseButtonEvent::Release, 29, thumb_col);
    wait_for_screen(&term, |text| {
        text.contains("PI_SCROLL_011_029") && !text.contains("Jump to latest message")
    });

    term.send(sequence.repeat(12)).unwrap();
    wait_for_screen(&term, |text| text.contains("Jump to latest message"));
    let (row, col) = {
        let buffer = term.buffer.lock().unwrap();
        let screen = buffer.parser.screen();
        screen
            .rows(0, screen.size().1)
            .enumerate()
            .find_map(|(row, line)| {
                line.find("Jump to latest message").map(|offset| {
                    (
                        row as u16,
                        unicode_width::UnicodeWidthStr::width(&line[..offset]) as u16,
                    )
                })
            })
            .expect("jump button must be visible")
    };
    send_mouse(&term, MouseButtonEvent::Press, row, col);
    send_mouse(&term, MouseButtonEvent::Release, row, col);
    wait_for_screen(&term, |text| {
        text.contains("PI_SCROLL_011_029") && !text.contains("Jump to latest message")
    });
    term.stop();
    drop(term);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires Pi; verifies real ConPTY cursor movement without model requests"]
fn real_pi_caret_tracks_middle_of_input() {
    let root = std::env::temp_dir().join(format!("vibe-pi-caret-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let session = root.join("session.jsonl");
    fs::write(&session, format!("{}\n{}\n", json!({"type":"session","version":3,"id":uuid::Uuid::new_v4(),"timestamp":"2026-11-16T00:00:00Z","cwd":root}), json!({"type":"message","id":"seed","parentId":null,"timestamp":"2026-11-16T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"PI_CARET_FIXTURE"}],"timestamp":1794787200000u64}}))).unwrap();
    let profile = CliProfile {
        id: "pi-caret-test".into(),
        name: "Pi".into(),
        command: "pi".into(),
        kind: CliKind::Pi,
        args: vec![
            "--tui-mode".into(),
            "fullscreen".into(),
            "--no-extensions".into(),
            "--no-skills".into(),
            "--no-prompt-templates".into(),
        ],
    };
    let tab = Tab::new(profile, root.clone());
    fs::rename(&session, root.join(format!("{}.jsonl", tab.id))).unwrap();
    let mut term = Terminal::start(&tab, &root, 30, 110, Arc::new(|| {})).unwrap();
    wait_for_screen(&term, |text| text.contains("PI_CARET_FIXTURE"));
    term.send(b"CARET_abcdefghij".to_vec()).unwrap();
    wait_for_screen(&term, |text| text.contains("CARET_abcdefghij"));
    let (row, start) = {
        let buffer = term.buffer.lock().unwrap();
        let screen = buffer.parser.screen();
        screen
            .rows(0, 110)
            .enumerate()
            .find_map(|(row, line)| {
                line.find("CARET_abcdefghij")
                    .map(|col| (row as u16, col as u16))
            })
            .unwrap()
    };
    term.send(b"\x1b[D".repeat(5)).unwrap();
    let expected = (row, start + 11);
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let buffer = term.buffer.lock().unwrap();
        let screen = buffer.parser.screen();
        let ready = screen.cursor_position() == expected
            || screen
                .cell(expected.0, expected.1)
                .is_some_and(|cell| cell.inverse());
        drop(buffer);
        if ready || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let actual = {
        let buffer = term.buffer.lock().unwrap();
        eprintln!(
            "Pi edit cursor expected={expected:?}, ConPTY cursor={:?}, hidden={}",
            buffer.parser.screen().cursor_position(),
            buffer.parser.screen().hide_cursor()
        );
        buffer.parser.screen().cursor_position()
    };
    term.stop();
    drop(term);
    fs::remove_dir_all(root).unwrap();
    assert_eq!(
        actual, expected,
        "ConPTY must preserve the edit cursor after moving left inside existing text"
    );
}

fn send_mouse(term: &Terminal, event: MouseButtonEvent, row: u16, col: u16) {
    let sequence = {
        let buffer = term.buffer.lock().unwrap();
        mouse_button_sequence(buffer.parser.screen(), event, row, col, false, false, true)
            .expect("Pi must receive button input through ConPTY")
    };
    term.send(sequence).unwrap();
}

fn wait_for_screen(term: &Terminal, predicate: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = term.buffer.lock().unwrap().parser.screen().contents();
        if predicate(&text) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Pi screen did not reach expected state:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
