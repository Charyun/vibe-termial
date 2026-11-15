use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};
use vibe_terminal::{
    model::{CliProfile, Tab},
    terminal::Terminal,
};

#[test]
fn real_pty_input_resize_and_exit() {
    let mut profile = CliProfile::defaults().remove(0);
    #[cfg(windows)]
    {
        profile.command = "powershell.exe".into();
        profile.name = "PowerShell".into();
    }
    let tab = Tab::new(profile, std::env::current_dir().unwrap());
    let wakes = Arc::new(AtomicUsize::new(0));
    let count = wakes.clone();
    let mut term = Terminal::start(
        &tab,
        &std::env::temp_dir(),
        24,
        80,
        Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    term.resize(30, 100).unwrap();
    #[cfg(windows)]
    term.send(
        "[Console]::WriteLine(('VIBE_' + 'PTY_OK'));[Console]::WriteLine(('中文' + '输入'))\r"
            .as_bytes()
            .to_vec(),
    )
    .unwrap();
    #[cfg(unix)]
    term.send(b"printf 'VIBE_%s\\n' 'PTY_OK'\r".to_vec())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let contents = term.buffer.lock().unwrap().parser.screen().contents();
        if contents.contains("VIBE_PTY_OK") && (!cfg!(windows) || contents.contains("中文输入"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "PTY did not return command output"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        wakes.load(Ordering::Relaxed) > 0,
        "First PTY output must wake an event-driven frontend, including after resize"
    );
    assert_eq!(
        term.buffer.lock().unwrap().parser.screen().size(),
        (30, 100)
    );
    term.stop();
    assert!(!term.running());
    assert!(term.ended.load(Ordering::Acquire));
    assert!(term.send(vec![3]).is_err());
    let _ = PathBuf::new();
}
