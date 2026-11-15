use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use vibe_terminal::{
    model::{CliProfile, Tab},
    terminal::Terminal,
};

#[cfg(windows)]
#[test]
fn default_cmd_starts_and_runs_commands() {
    let profile = CliProfile::default_shell();
    assert_eq!(profile.command, "cmd.exe");
    let tab = Tab::new(profile, std::env::current_dir().unwrap());
    let mut term = Terminal::start(&tab, &std::env::temp_dir(), 24, 100, Arc::new(|| {})).unwrap();
    term.send(b"set VIBE_MARKER=VIBE_CMD_\r\necho %VIBE_MARKER%OK\r\n".to_vec())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !term
        .buffer
        .lock()
        .unwrap()
        .parser
        .screen()
        .contents()
        .contains("VIBE_CMD_OK")
    {
        assert!(Instant::now() < deadline, "CMD did not execute its command");
        std::thread::sleep(Duration::from_millis(20));
    }
    term.stop();
    assert!(!term.running());
}
#[test]
fn poisoned_parser_does_not_prevent_process_and_writer_cleanup() {
    let tab = Tab::new(
        CliProfile::default_shell(),
        std::env::current_dir().unwrap(),
    );
    let mut term = Terminal::start(&tab, &std::env::temp_dir(), 24, 80, Arc::new(|| {})).unwrap();
    let buffer = term.buffer.clone();
    let crashed = std::thread::spawn(move || {
        let _guard = buffer.lock().unwrap();
        panic!("simulate parser panic while holding the buffer");
    });
    assert!(crashed.join().is_err());
    let started = Instant::now();
    term.stop();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!term.running());
}
