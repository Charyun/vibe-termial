//! Explicit integration check against the installed Pi CLI. No model request is made.
use serde_json::json;
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use vibe_terminal::{
    model::{CliKind, CliProfile, Tab},
    scroll::mouse_wheel_sequence,
    terminal::Terminal,
};

#[test]
#[ignore = "requires the Pi CLI; run with an isolated PI_CODING_AGENT_DIR"]
fn real_pi_renders_seeded_history_and_receives_forwarded_wheel() {
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
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        term.buffer
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("PI_SCROLL_"),
        "Pi should keep showing the conversation after forwarded wheel input"
    );
    term.stop();
    drop(term);
    fs::remove_dir_all(root).unwrap();
}
