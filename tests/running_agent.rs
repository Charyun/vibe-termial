#![cfg(windows)]
use std::{
    fs,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use vibe_terminal::{
    model::{AppData, CliKind, CliProfile},
    running_agent,
    session::Session,
    store::Store,
};

fn poll(session: &mut Session) {
    let requests = session.shell_processes();
    let roots = requests.iter().map(|(_, _, pid)| *pid).collect::<Vec<_>>();
    let results = running_agent::detect(&roots);
    for (id, runtime, root) in requests {
        if let Some((_, agent)) = results.iter().find(|(pid, _)| *pid == root) {
            session.apply_running_agent(id, runtime, root, agent.clone());
        }
    }
}
fn wait(session: &mut Session, mut ready: impl FnMut(&Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        poll(session);
        if ready(session) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Agent identity transition timed out"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn shell_child_changes_identity_and_returns_without_replacing_the_pty() {
    let root = std::env::temp_dir().join(format!("vibe-shell-agent-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("history")).unwrap();
    let mut data = AppData::default();
    data.add_workspace(&root).unwrap();
    let mut session = Session::new(Store { root: root.clone() }, data, None);
    session.new_tab(CliProfile::default_shell());
    let id = session.selected().unwrap();
    let original = session.terminals[&id].buffer.clone();
    session.new_tab(CliProfile::default_shell());
    let other = session.selected().unwrap();
    session.select_tab(id);
    for (package, name, expected) in [
        ("@earendil-works/pi-coding-agent/dist", "cli.js", "pi"),
        ("@openai/codex/bin", "codex.js", "codex"),
    ] {
        let script = root.join("node_modules").join(package).join(name);
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        let receipt = root.join(format!("{expected}-mouse.bin"));
        fs::write(
            &script,
            r#"const fs = require('node:fs');
process.stdin.setRawMode(true);
process.stdin.resume();
process.stdin.on('data', b => {
  if (b.includes(113)) { process.stdout.write('\x1b[?1003l\x1b[?1006l'); process.exit(0); }
  fs.appendFileSync(process.argv[2], b);
});
process.stdout.write('\x1b[?1003h\x1b[?1006hREADY_' + process.argv[3]);
"#,
        )
        .unwrap();
        session.send(
            format!(
                "node \"{}\" \"{}\" {expected}\r",
                script.display(),
                receipt.display()
            )
            .into_bytes(),
        );
        wait(&mut session, |s| {
            s.data.tab(id).unwrap().profile.id == expected
        });
        let tab = session.data.tab(id).unwrap();
        assert_eq!(
            tab.display_title(),
            if expected == "pi" { "Pi" } else { "Codex" }
        );
        assert_eq!(
            session.data.tab(other).unwrap().profile.kind,
            CliKind::Shell
        );
        assert!(Arc::ptr_eq(&original, &session.terminals[&id].buffer));
        wait(&mut session, |s| {
            s.terminals[&id]
                .buffer
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents()
                .contains(&format!("READY_{expected}"))
        });
        let (up, down) = {
            let buffer = session.terminals[&id].buffer.lock().unwrap();
            let screen = buffer.parser.screen();
            let known_agent = matches!(
                session.data.tab(id).unwrap().profile.kind,
                CliKind::Pi | CliKind::Codex
            );
            (
                vibe_terminal::scroll::mouse_wheel_sequence(
                    screen,
                    true,
                    10,
                    10,
                    false,
                    false,
                    known_agent,
                )
                .unwrap(),
                vibe_terminal::scroll::mouse_wheel_sequence(
                    screen,
                    false,
                    10,
                    10,
                    false,
                    false,
                    known_agent,
                )
                .unwrap(),
            )
        };
        assert_eq!(up, b"\x1b[<64;11;11M");
        assert_eq!(down, b"\x1b[<65;11;11M");
        let mut expected_input = up.repeat(3);
        expected_input.extend(down.repeat(2));
        session.send(expected_input.clone());
        wait(&mut session, |_| {
            fs::read(&receipt).unwrap_or_default() == expected_input
        });
        assert_eq!(
            fs::read(&receipt).unwrap(),
            expected_input,
            "{expected} launched through CMD must receive mouse-wheel events, not arrow keys"
        );
        session.persist();
        assert_eq!(
            session.store.load().unwrap().tab(id).unwrap().profile.kind,
            CliKind::Shell
        );
        session.send(b"q".to_vec());
        wait(&mut session, |s| {
            s.data.tab(id).unwrap().profile.kind == CliKind::Shell
        });
        assert_eq!(session.data.tab(id).unwrap().display_title(), "CMD");
        assert!(Arc::ptr_eq(&original, &session.terminals[&id].buffer));
    }
    // Text mentioning an Agent isn't a running Agent.
    session.send(b"echo pi codex\r".to_vec());
    thread::sleep(Duration::from_millis(100));
    poll(&mut session);
    assert_eq!(session.data.tab(id).unwrap().profile.kind, CliKind::Shell);
    session.shutdown();
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
