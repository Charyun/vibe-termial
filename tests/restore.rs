//! Failed restores are skipped without a replacement terminal or deleted history.
use std::{fs, path::PathBuf};
use uuid::Uuid;
use vibe_terminal::{
    model::{AppData, CliKind, CliProfile, Tab},
    session::Session,
    store::Store,
};
fn fixture(kind: CliKind, launched: bool) -> (PathBuf, Session, Uuid) {
    let root = std::env::temp_dir().join(format!("vibe-failed-restore-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("history")).unwrap();
    fs::create_dir_all(root.join("sessions")).unwrap();
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    let profile = CliProfile {
        id: "unavailable".into(),
        name: "Unavailable agent".into(),
        command: root.join("missing-cli.exe").to_string_lossy().into_owned(),
        args: vec![],
        kind,
    };
    let mut tab = Tab::new(profile, root.clone());
    tab.launched = launched;
    tab.session_id = None;
    let id = tab.id;
    data.workspace_mut().unwrap().tabs.push(tab);
    data.workspace_mut().unwrap().selected_tab = Some(id);
    let store = Store { root: root.clone() };
    store.save_history(id, b"saved terminal history").unwrap();
    store.save(&data).unwrap();
    (root, Session::new(store, data, None), id)
}
#[test]
fn explicit_restore_skips_unbound_codex_even_if_it_never_launched() {
    let (root, mut session, id) = fixture(CliKind::Codex, false);
    session.restore(id);
    assert!(session.error.is_none());
    assert!(session.data.tab(id).is_none());
    assert!(session.terminals.is_empty());
    assert_eq!(
        session.store.history(id).unwrap(),
        b"saved terminal history"
    );
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn archive_failure_does_not_discard_saved_session_metadata() {
    let (root, mut session, id) = fixture(CliKind::Codex, true);
    fs::write(root.join("failed-restores"), b"not a directory").unwrap();
    session.start(id);
    assert!(session.error.is_some());
    assert!(session.data.tab(id).is_some());
    assert!(session.store.load().unwrap().tab(id).is_some());
    assert_eq!(
        session.store.history(id).unwrap(),
        b"saved terminal history"
    );
    assert!(session.terminals.is_empty());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_without_session_id_is_not_opened_and_does_not_raise_a_restore_prompt() {
    let (root, mut session, id) = fixture(CliKind::Codex, true);
    session.ensure_history();
    assert!(session.terminals.contains_key(&id));
    session.start(id);
    assert!(session.error.is_none());
    assert!(session.data.tab(id).is_none());
    assert!(session.terminals.is_empty());
    assert_eq!(session.selected(), None);
    assert!(
        session.data.workspace().unwrap().tabs.is_empty(),
        "no CMD or other replacement tab may be created"
    );
    assert_eq!(
        session.store.history(id).unwrap(),
        b"saved terminal history"
    );
    let archived: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("failed-restores").join(format!("{id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(archived["tab"]["id"], id.to_string());
    assert_eq!(archived["tab"]["profile"]["kind"], "codex");
    assert!(session.store.load().unwrap().tab(id).is_none());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn other_failed_restores_are_skipped_but_new_launch_errors_still_report() {
    for launched in [true, false] {
        let (root, mut session, id) = fixture(CliKind::Custom, launched);
        session.start(id);
        if launched {
            assert!(session.error.is_none());
            assert!(session.data.tab(id).is_none());
            assert!(session.terminals.is_empty());
        } else {
            assert!(session.error.is_some());
            assert!(session.data.tab(id).is_some());
            assert!(!root.join("failed-restores").exists());
        }
        drop(session);
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn skipping_one_restore_preserves_the_other_tabs_and_selection() {
    let (root, mut session, id) = fixture(CliKind::Codex, true);
    let other = Tab::new(CliProfile::default_shell(), root.clone());
    let other_id = other.id;
    session.data.workspace_mut().unwrap().tabs.push(other);
    session.start(id);
    assert_eq!(session.data.workspace().unwrap().tabs.len(), 1);
    assert_eq!(session.selected(), Some(other_id));
    assert!(
        session.terminals.is_empty(),
        "skip must not automatically start a different terminal"
    );
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
