use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use uuid::Uuid;
use vibe_terminal::{
    model::AppData,
    session::Session,
    ssh::{SshHost, SshProject},
    store::Store,
};

fn host() -> SshHost {
    SshHost {
        id: Uuid::new_v4(),
        name: "Development".into(),
        host: "dev-alias".into(),
        user: "developer".into(),
        port: Some(2222),
        identity_file: None,
        config_file: None,
    }
}
#[test]
fn remote_workspace_persists_without_becoming_a_local_directory() {
    let mut data = AppData::default();
    let machine = host();
    let id = machine.id;
    data.ssh_hosts.push(machine);
    let workspace = data
        .add_remote_workspace(
            id,
            "/srv/project with spaces".into(),
            "Server project".into(),
        )
        .unwrap();
    assert_eq!(
        data.add_remote_workspace(id, "/srv/project with spaces".into(), "ignored".into())
            .unwrap(),
        workspace
    );
    assert_eq!(data.workspaces.len(), 1);
    assert_eq!(
        data.workspace().unwrap().remote.as_ref().unwrap().host_id,
        id
    );
    let json = serde_json::to_string(&data).unwrap();
    let restored: AppData = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.ssh_hosts[0].port, Some(2222));
    assert_eq!(
        restored.workspace().unwrap().remote.as_ref().unwrap().path,
        "/srv/project with spaces"
    );
    let root = std::env::temp_dir().join(format!("vibe-ssh-model-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let store = Store { root: root.clone() };
    store.save(&restored).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(
        loaded.workspace().unwrap().remote,
        restored.workspace().unwrap().remote
    );
    let mut session = Session::new(store, loaded, None);
    session.read_only = true;
    session
        .add_remote_workspace(id, "/srv/another".into(), String::new())
        .unwrap();
    let tab = session.selected().unwrap();
    assert!(session.data.is_remote_tab(tab));
    assert_eq!(session.data.tab(tab).unwrap().profile.name, "SSH Shell");
    session.shutdown();
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn legacy_state_defaults_to_local_projects_and_no_ssh_hosts() {
    let data:AppData=serde_json::from_value(serde_json::json!({"version":1,"selected_workspace":null,"profiles":[],"workspaces":[{"id":Uuid::new_v4(),"name":"old","directory":"old","tabs":[],"selected_tab":null}]})).unwrap();
    assert!(data.ssh_hosts.is_empty());
    assert!(data.workspaces[0].remote.is_none());
}

struct Fixture {
    child: std::process::Child,
    root: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::write(self.root.join("stop"), b"stop");
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
#[test]
#[ignore = "uses system OpenSSH, Python paramiko and POSIX sh against a disposable loopback SSH server"]
fn real_ssh_transport_runs_in_remote_project_and_preserves_metadata() {
    use std::process::{Command, Stdio};
    let root = std::env::temp_dir().join(format!("vibe-ssh-live-{}", Uuid::new_v4()));
    std::fs::create_dir_all(root.join("data/sessions")).unwrap();
    std::fs::create_dir_all(root.join("data/history")).unwrap();
    let stderr = std::fs::File::create(root.join("server.log")).unwrap();
    let mut fixture = Fixture {
        root: root.clone(),
        child: Command::new("python")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ssh_server.py"))
            .arg(&root)
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .unwrap(),
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    while !root.join("ready.json").exists() {
        assert!(
            fixture.child.try_wait().unwrap().is_none(),
            "SSH fixture exited: {}",
            std::fs::read_to_string(root.join("server.log")).unwrap()
        );
        assert!(Instant::now() < deadline, "SSH fixture startup timeout");
        std::thread::sleep(Duration::from_millis(50));
    }
    let ready: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("ready.json")).unwrap()).unwrap();
    let machine = SshHost {
        id: Uuid::new_v4(),
        name: "Loopback fixture".into(),
        host: "127.0.0.1".into(),
        user: "fixture".into(),
        port: Some(ready["port"].as_u64().unwrap() as u16),
        identity_file: Some(ready["identity"].as_str().unwrap().into()),
        config_file: Some(ready["config"].as_str().unwrap().into()),
    };
    let status = vibe_terminal::ssh::probe(&machine).unwrap();
    assert!(status.starts_with("已连接"));
    let project = ready["project"].as_str().unwrap().to_owned();
    let mut data = AppData::default();
    let host_id = machine.id;
    data.ssh_hosts.push(machine);
    let mut session = Session::new(
        Store {
            root: root.join("data"),
        },
        data,
        None,
    );
    let workspace = session
        .add_remote_workspace(host_id, project.clone(), "Loopback project".into())
        .unwrap();
    assert!(session.error.is_none(), "{:?}", session.error);
    let id = session.selected().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !session.terminals[&id]
        .buffer
        .lock()
        .unwrap()
        .parser
        .screen()
        .contents()
        .contains("__VIBE_SSH_READY__")
    {
        assert!(
            Instant::now() < deadline,
            "SSH terminal did not execute remote command: {}",
            std::fs::read_to_string(root.join("server.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    session.terminals[&id]
        .send(b"printf '__VIBE_REMOTE_EXEC__\\n'; pwd; exit\r".to_vec())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let output = std::fs::read_to_string(root.join("output.log")).unwrap_or_default();
        if output
            .lines()
            .any(|line| line.trim() == "__VIBE_REMOTE_EXEC__")
            && output.contains("remote project ' quoted")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "remote shell did not execute expected command: {output}; terminal={}; received={}",
            session.terminals[&id]
                .buffer
                .lock()
                .unwrap()
                .parser
                .screen()
                .contents(),
            std::fs::read_to_string(root.join("input.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        session.terminals[&id]
            .buffer
            .lock()
            .unwrap()
            .parser
            .callbacks()
            .remote
    );
    session.checkpoint();
    assert_eq!(session.data.workspace().unwrap().id, workspace);
    assert_eq!(
        session.data.workspace().unwrap().remote,
        Some(SshProject {
            host_id,
            path: project
        })
    );
    let mut password_host = session.data.ssh_hosts[0].clone();
    password_host.id = Uuid::new_v4();
    password_host.name = "Password fixture".into();
    password_host.identity_file = None;
    let password_id = password_host.id;
    session.data.ssh_hosts.push(password_host);
    session
        .add_remote_workspace(
            password_id,
            session
                .data
                .workspace()
                .unwrap()
                .remote
                .as_ref()
                .unwrap()
                .path
                .clone(),
            "Password project".into(),
        )
        .unwrap();
    let password_tab = session.selected().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !session.terminals[&password_tab]
        .buffer
        .lock()
        .unwrap()
        .parser
        .screen()
        .contents()
        .to_lowercase()
        .contains("password")
    {
        assert!(
            Instant::now() < deadline,
            "interactive SSH did not display a password prompt"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    session.terminals[&password_tab]
        .send(b"fixture-pass\r".to_vec())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !session.terminals[&password_tab]
        .buffer
        .lock()
        .unwrap()
        .parser
        .screen()
        .contents()
        .contains("__VIBE_SSH_READY__")
    {
        assert!(
            Instant::now() < deadline,
            "SSH password authentication did not reach the remote shell"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    session.terminals[&password_tab]
        .send(b"printf '__VIBE_PASSWORD_OK__\\n'; exit\r".to_vec())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(root.join("output.log"))
        .unwrap_or_default()
        .lines()
        .any(|line| line.trim() == "__VIBE_PASSWORD_OK__")
    {
        assert!(
            Instant::now() < deadline,
            "password-authenticated remote shell did not execute command"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !serde_json::to_string(&session.data)
            .unwrap()
            .contains("fixture-pass")
    );
    session.shutdown();
    drop(fixture);
    std::fs::remove_dir_all(root).unwrap();
}
