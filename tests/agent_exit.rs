#![cfg(windows)]

use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use vibe_terminal::{
    model::{AppData, CliKind, CliProfile},
    session::Session,
    store::Store,
};

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "terminal transition timed out");
        thread::sleep(Duration::from_millis(20));
    }
}
fn contents(session: &Session, id: Uuid) -> String {
    session.terminals[&id]
        .buffer
        .lock()
        .unwrap()
        .parser
        .screen()
        .contents()
}
fn fixture() -> (PathBuf, Session, Uuid) {
    let root = std::env::temp_dir().join(format!("vibe-agent-exit-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let helper = root.join("agent.cjs");
    fs::write(&helper, include_str!("fixtures/activity-agent.cjs")).unwrap();
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(Store { root: root.clone() }, data, None);
    session.new_tab(CliProfile {
        id: "exit-fixture".into(),
        name: "Pi fixture".into(),
        command: "node".into(),
        args: vec![helper.to_string_lossy().into_owned()],
        kind: CliKind::Pi,
    });
    let id = session.selected().unwrap();
    wait(|| contents(&session, id).contains("ACTIVITY_FIXTURE_READY"));
    (root, session, id)
}

#[test]
fn interrupted_agent_only_returns_to_cmd_after_exit_and_preserves_its_binding() {
    let (root, mut session, id) = fixture();
    let original = session.terminals[&id].buffer.clone();
    let runtime = original.lock().unwrap().runtime_id;
    session.data.tab_mut(id).unwrap().rename("My Agent".into());
    let saved_session = Uuid::new_v4().to_string();
    session.data.tab_mut(id).unwrap().session_id = Some(saved_session.clone());
    session.send(b"\x03".to_vec());
    wait(|| contents(&session, id).contains("AGENT_INTERRUPTED"));
    assert!(session.return_exited_agents_to_shell().is_empty());
    assert!(Arc::ptr_eq(&original, &session.terminals[&id].buffer));
    assert_eq!(session.data.tab(id).unwrap().profile.kind, CliKind::Pi);
    session.send(b"\x03".to_vec());
    wait(|| !session.return_exited_agents_to_shell().is_empty());
    let tab = session.data.tab(id).unwrap();
    assert_eq!(session.selected(), Some(id));
    assert_eq!(tab.profile.kind, CliKind::Shell);
    assert_eq!(tab.display_title(), "CMD");
    assert_eq!(tab.profile.command, "cmd.exe");
    assert!(tab.session_id.is_none());
    assert!(!tab.title_is_manual);
    assert_eq!(tab.conversation_generation, 1);
    assert!(!Arc::ptr_eq(&original, &session.terminals[&id].buffer));
    let archive = root
        .join("sessions/exited-agents")
        .join(format!("{id}-{runtime}"));
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(archive.with_extension("json")).unwrap()).unwrap();
    assert_eq!(saved["session_id"], saved_session);
    assert_eq!(saved["title"], "My Agent");
    assert_eq!(saved["profile"]["kind"], "pi");
    assert!(
        fs::read(archive.with_extension("ansi"))
            .unwrap()
            .windows(17)
            .any(|w| w == b"AGENT_INTERRUPTED")
    );
    assert!(root.join("sessions").join(format!("{id}.jsonl")).exists());
    session.send(b"set VIBE_RESIDENT=RETURNED_\recho %VIBE_RESIDENT%CMD_OK\r".to_vec());
    wait(|| contents(&session, id).contains("RETURNED_CMD_OK"));
    let shell = session.terminals[&id].buffer.clone();
    session.new_tab(CliProfile::default_shell());
    session.select_tab(id);
    session.restore(id);
    assert!(Arc::ptr_eq(&shell, &session.terminals[&id].buffer));
    session.send(b"echo %VIBE_RESIDENT%STILL_HERE\r".to_vec());
    wait(|| contents(&session, id).contains("RETURNED_STILL_HERE"));
    assert_eq!(
        session.store.load().unwrap().tab(id).unwrap().profile.kind,
        CliKind::Shell
    );
    session.shutdown();
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn opened_agent_and_exited_runtime_are_not_reloaded_when_switching() {
    let (root, mut session, id) = fixture();
    let original = session.terminals[&id].buffer.clone();
    {
        let mut buffer = original.lock().unwrap();
        for line in 0..70 {
            buffer.process(format!("resident line {line}\r\n").as_bytes());
        }
        buffer.scroll_lines(12);
    }
    let scroll = original.lock().unwrap().parser.screen().scrollback();
    assert!(scroll > 0);
    session.new_tab(CliProfile::default_shell());
    let other_workspace = session.data.add_workspace(&root.join("sessions")).unwrap();
    let workspace = session
        .data
        .workspaces
        .iter()
        .find(|w| w.tabs.iter().any(|t| t.id == id))
        .unwrap()
        .id;
    session.select_workspace(other_workspace);
    session.select_workspace(workspace);
    session.select_tab(id);
    session.restore(id);
    assert!(Arc::ptr_eq(&original, &session.terminals[&id].buffer));
    assert_eq!(
        original.lock().unwrap().parser.screen().scrollback(),
        scroll
    );
    session.send(b"Q".to_vec());
    wait(|| session.terminals.get_mut(&id).unwrap().exited());
    assert!(session.return_exited_agents_to_shell().is_empty());
    session.new_tab(CliProfile::default_shell());
    session.select_tab(id);
    session.restore(id);
    assert!(Arc::ptr_eq(&original, &session.terminals[&id].buffer));
    assert!(session.terminals.get_mut(&id).unwrap().exited());
    session.restart(id);
    assert!(!Arc::ptr_eq(&original, &session.terminals[&id].buffer));
    wait(|| contents(&session, id).contains("ACTIVITY_FIXTURE_READY"));
    session.shutdown();
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_archive_keeps_original_agent_metadata_and_does_not_start_cmd() {
    let (root, mut session, id) = fixture();
    let original = session.terminals[&id].buffer.clone();
    fs::write(root.join("sessions/exited-agents"), b"blocked directory").unwrap();
    session.send(b"\x03".to_vec());
    wait(|| contents(&session, id).contains("AGENT_INTERRUPTED"));
    session.send(b"\x03".to_vec());
    wait(|| session.terminals.get_mut(&id).unwrap().exited());
    assert!(session.return_exited_agents_to_shell().is_empty());
    assert!(session.error.is_some());
    assert_eq!(session.data.tab(id).unwrap().profile.kind, CliKind::Pi);
    assert!(Arc::ptr_eq(&original, &session.terminals[&id].buffer));
    session.shutdown();
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
