//! Shared conversation input exercises the real PTY, without any model requests.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;
use vibe_terminal::{
    conversation::ConversationReader,
    model::{AppData, CliKind, CliProfile},
    session::Session,
    store::Store,
};

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "PTY fixture did not reach expected state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn fixture() -> (PathBuf, Session, CliProfile) {
    let root = std::env::temp_dir().join(format!("vibe-content-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    let helper = root.join("agent.cjs");
    fs::write(&helper, include_str!("fixtures/conversation-agent.cjs")).unwrap();
    let store = Store { root: root.clone() };
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    let session = Session::new(store, data, None);
    let profile = CliProfile {
        id: "content-fixture".into(),
        name: "Pi fixture".into(),
        command: "node".into(),
        args: vec![helper.to_string_lossy().into_owned()],
        kind: CliKind::Pi,
    };
    (root, session, profile)
}
fn ready(session: &Session, id: Uuid) -> bool {
    session.terminals[&id]
        .buffer
        .lock()
        .unwrap()
        .parser
        .screen()
        .contents()
        .contains("CONTENT_FIXTURE_READY")
}
fn session_path(root: &Path, id: Uuid) -> PathBuf {
    root.join("sessions").join(format!("{id}.jsonl"))
}

#[test]
fn content_send_and_terminal_share_process_and_transcript_after_tab_switch() {
    let (root, mut session, profile) = fixture();
    session.new_tab(profile.clone());
    let first = session.selected().unwrap();
    wait_for(|| ready(&session, first));
    let original_buffer = session.terminals[&first].buffer.clone();
    let text = "你好\nsecond line";
    let ticket = session.begin_message(first, text).unwrap();
    let path = session_path(&root, first);
    wait_for(|| path.with_extension("jsonl.body").is_file());
    assert_eq!(
        fs::read_to_string(path.with_extension("jsonl.body")).unwrap(),
        text
    );
    assert!(!path.is_file(), "paste body alone must not submit a turn");
    session.new_tab(profile);
    let second = session.selected().unwrap();
    wait_for(|| ready(&session, second));
    session.finish_message(ticket).unwrap();
    wait_for(|| fs::read_to_string(&path).is_ok_and(|text| text.lines().count() == 2));
    assert!(Arc::ptr_eq(
        &original_buffer,
        &session.terminals[&first].buffer
    ));
    assert!(
        !session_path(&root, second).is_file(),
        "delayed Enter must never target the newly selected tab"
    );
    let mut reader = ConversationReader::default();
    let snapshot = reader
        .refresh(session.data.tab(first).unwrap(), &root)
        .unwrap();
    assert_eq!(snapshot.messages[0].text, text);
    assert_eq!(snapshot.messages[1].text, format!("Echo: {text}"));
    // A message entered through the terminal follows the same conversation file.
    session.select_tab(first);
    session.send(b"\x1b[200~from terminal\x1b[201~".to_vec());
    wait_for(|| {
        fs::read_to_string(path.with_extension("jsonl.body"))
            .is_ok_and(|text| text == "from terminal")
    });
    session.send(vec![b'\r']);
    wait_for(|| fs::read_to_string(&path).is_ok_and(|text| text.lines().count() == 4));
    let snapshot = reader
        .refresh(session.data.tab(first).unwrap(), &root)
        .unwrap();
    assert_eq!(snapshot.messages.len(), 4);
    assert_eq!(snapshot.messages[2].text, "from terminal");
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_submit_is_rejected_after_process_replacement() {
    let (root, mut session, profile) = fixture();
    session.new_tab(profile);
    let id = session.selected().unwrap();
    wait_for(|| ready(&session, id));
    let ticket = session
        .begin_message(id, "do not submit after restart")
        .unwrap();
    let path = session_path(&root, id);
    wait_for(|| path.with_extension("jsonl.body").is_file());
    session.restart(id);
    wait_for(|| ready(&session, id));
    assert!(session.finish_message(ticket).is_err());
    assert!(!path.is_file());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires installed Pi and an isolated PI_CODING_AGENT_DIR; no model request"]
fn real_pi_content_command_uses_the_existing_session() {
    let root = std::env::temp_dir().join(format!("vibe-pi-content-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(Store { root: root.clone() }, data, None);
    let profile = CliProfile {
        id: "pi-content".into(),
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
    let tab = vibe_terminal::model::Tab::new(profile, root.clone());
    let id = tab.id;
    fs::write(session_path(&root, id), format!("{}\n{}\n", serde_json::json!({"type":"session","version":3,"id":Uuid::new_v4(),"timestamp":"2026-11-16T00:00:00Z","cwd":root}), serde_json::json!({"type":"message","id":"seed","parentId":null,"timestamp":"2026-11-16T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"PI_CONTENT_FIXTURE"}],"timestamp":1794787200000u64}}))).unwrap();
    let workspace = session.data.workspace_mut().unwrap();
    workspace.tabs.push(tab);
    workspace.selected_tab = Some(id);
    session.start(id);
    wait_for(|| {
        session.terminals[&id]
            .buffer
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("clear/exit")
    });
    let original_buffer = session.terminals[&id].buffer.clone();
    session.send(b"/name stale draft".to_vec());
    wait_for(|| session.content_draft(id).as_deref() == Some("/name stale draft"));
    let ticket = session
        .begin_message(id, "/name Shared content fixture")
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    session.finish_message(ticket).unwrap();
    wait_for(|| {
        fs::read_to_string(session_path(&root, id)).is_ok_and(|text| {
            text.contains("Shared content fixture") && text.contains("session_info")
        })
    });
    assert!(Arc::ptr_eq(
        &original_buffer,
        &session.terminals[&id].buffer
    ));
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
