//! Use native program status where the PTY preserves it, and real session logs otherwise.
use std::{
    fs,
    time::{Duration, Instant},
};
use uuid::Uuid;
use vibe_terminal::{
    activity::{ActivityReader, Indicator, Phase},
    model::{AppData, CliKind, CliProfile},
    session::Session,
    store::Store,
};
#[track_caller]
fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Agent status did not reach expected state"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn indicator(session: &Session, id: Uuid) -> Indicator {
    let terminal = &session.terminals[&id];
    terminal
        .buffer
        .lock()
        .unwrap()
        .parser
        .callbacks()
        .activity
        .indicator(terminal.running())
}
fn poll(session: &Session, reader: &mut ActivityReader) {
    let tabs = session
        .data
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.tabs)
        .cloned()
        .collect::<Vec<_>>();
    for update in reader.refresh(&tabs, &session.store.root) {
        let mut buffer = session.terminals[&update.key.tab_id].buffer.lock().unwrap();
        if update.worked && !update.seed {
            buffer
                .parser
                .callbacks_mut()
                .activity
                .fallback(Phase::Working, false);
        }
        buffer
            .parser
            .callbacks_mut()
            .activity
            .fallback(update.phase, update.seed);
    }
}
#[test]
fn background_completion_and_click_acknowledgement_through_real_pty() {
    let root = std::env::temp_dir().join(format!("vibe-activity-pty-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    let helper = root.join("agent.cjs");
    fs::write(&helper, include_str!("fixtures/activity-agent.cjs")).unwrap();
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(Store { root: root.clone() }, data, None);
    let profile = CliProfile {
        id: "activity-fixture".into(),
        name: "Status fixture".into(),
        command: "node".into(),
        args: vec![helper.to_string_lossy().into_owned()],
        kind: CliKind::Pi,
    };
    let mut reader = ActivityReader::default();
    session.new_tab(profile.clone());
    let first = session.selected().unwrap();
    wait_for(|| {
        session.terminals[&first]
            .buffer
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("ACTIVITY_FIXTURE_READY")
    });
    poll(&session, &mut reader);
    assert_eq!(indicator(&session, first), Indicator::None);
    session.send(b"W".to_vec());
    wait_for(|| {
        poll(&session, &mut reader);
        indicator(&session, first) == Indicator::Working
    });
    session.select_tab(first);
    assert_eq!(indicator(&session, first), Indicator::Working);
    session.new_tab(profile);
    let second = session.selected().unwrap();
    wait_for(|| {
        session.terminals[&second]
            .buffer
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("ACTIVITY_FIXTURE_READY")
    });
    session.terminals[&first].send(b"D".to_vec()).unwrap();
    wait_for(|| {
        poll(&session, &mut reader);
        indicator(&session, first) == Indicator::Completed
    });
    assert_eq!(indicator(&session, second), Indicator::None);
    session.select_tab(first);
    assert_eq!(indicator(&session, first), Indicator::None);
    session.send(b"D".to_vec());
    std::thread::sleep(Duration::from_millis(80));
    poll(&session, &mut reader);
    assert_eq!(
        indicator(&session, first),
        Indicator::None,
        "repeat completion must not resurrect the green dot"
    );
    let native = session.terminals[&first]
        .buffer
        .lock()
        .unwrap()
        .parser
        .callbacks()
        .activity
        .native();
    session.send(b"WB".to_vec());
    wait_for(|| {
        poll(&session, &mut reader);
        indicator(&session, first)
            == if native {
                Indicator::Blocked
            } else {
                Indicator::Working
            }
    });
    session.send(b"E".to_vec());
    wait_for(|| {
        poll(&session, &mut reader);
        indicator(&session, first) == Indicator::Failed
    });
    session.select_tab(first);
    assert_eq!(indicator(&session, first), Indicator::None);
    session.restart(first);
    wait_for(|| {
        session.terminals[&first]
            .buffer
            .lock()
            .unwrap()
            .parser
            .screen()
            .contents()
            .contains("ACTIVITY_FIXTURE_READY")
    });
    poll(&session, &mut reader);
    assert_eq!(indicator(&session, first), Indicator::None);
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
