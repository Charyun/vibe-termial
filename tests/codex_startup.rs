//! Explicit local CLI smoke test: opens Codex without submitting a prompt.
use std::{fs, time::{Duration, Instant}};
use vibe_terminal::{activity::Indicator, model::{AppData, CliKind, CliProfile}, session::Session, store::Store};

#[test]
#[ignore = "launches the installed Codex CLI without submitting a request"]
fn installed_codex_new_session_is_idle_before_submission() {
    let root = std::env::temp_dir().join(format!("vibe-codex-startup-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let mut data = AppData::default();
    data.add_workspace(&std::env::current_dir().unwrap()).unwrap();
    let mut session = Session::new(Store { root: root.clone() }, data, None);
    session.new_tab(CliProfile {
        id: "codex-startup".into(), name: "Codex".into(), command: "codex".into(),
        args: vec![], kind: CliKind::Codex,
    });
    let id = session.selected().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut busy = false;
    let mut previous = None;
    while Instant::now() < deadline {
        let terminal = &session.terminals[&id];
        let buffer = terminal.buffer.lock().unwrap();
        let activity = &buffer.parser.callbacks().activity;
        let state = activity.indicator(terminal.running());
        if previous != Some(state) {
            eprintln!("Codex startup: indicator={state:?}, native={}", activity.native());
            previous = Some(state);
        }
        busy |= state == Indicator::Working;
        drop(buffer);
        std::thread::sleep(Duration::from_millis(50));
    }
    session.shutdown();
    fs::remove_dir_all(root).unwrap();
    assert!(!busy, "Codex initialization must not be shown as an active turn");
}
