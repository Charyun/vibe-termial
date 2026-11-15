#![cfg(feature = "web")]
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    fs,
    net::SocketAddr,
    sync::{Arc, mpsc},
    time::Duration,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use uuid::Uuid;
use vibe_terminal::{
    model::{AppData, CliProfile, Tab},
    session::Session,
    store::Store,
    web::{Command, Reply, Request, Server},
};

fn fixture() -> (std::path::PathBuf, Session, Uuid, Uuid) {
    let root = std::env::temp_dir().join(format!("vibe-web-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("history")).unwrap();
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    let mut data = AppData::default();
    let first = data.add_workspace(&root).unwrap();
    let local = Tab::new(CliProfile::default_shell(), root.clone());
    let local_id = local.id;
    data.workspace_mut().unwrap().tabs.push(local);
    data.workspace_mut().unwrap().selected_tab = Some(local_id);
    data.add_workspace(&root.join("other")).unwrap();
    let mut profile = CliProfile::default_shell();
    #[cfg(windows)]
    {
        profile.command = "powershell.exe".into();
    }
    let remote = Tab::new(profile, root.join("other"));
    let remote_id = remote.id;
    data.workspace_mut().unwrap().tabs.push(remote);
    data.workspace_mut().unwrap().selected_tab = Some(remote_id);
    data.selected_workspace = Some(first);
    let session = Session::new(Store { root: root.clone() }, data, None);
    (root, session, local_id, remote_id)
}

#[test]
fn remote_close_targets_its_workspace_and_keeps_desktop_selection() {
    let (root, mut session, local, remote) = fixture();
    let selected = session.data.selected_workspace;
    session.web_command(Command::Close { id: remote }).unwrap();
    assert_eq!(session.data.selected_workspace, selected);
    assert_eq!(session.selected(), Some(local));
    assert!(session.data.tab(local).is_some());
    assert!(session.data.tab(remote).is_none());
    assert!(session.web_command(Command::Start { id: remote }).is_err());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_history_is_read_only_and_readonly_instance_rejects_control() {
    let (root, mut session, local, remote) = fixture();
    session
        .store
        .save_history(remote, b"history-only-marker")
        .unwrap();
    assert!(matches!(
        session
            .web_command(Command::Subscribe { id: remote })
            .unwrap(),
        Reply::Stream(_)
    ));
    assert!(session.terminals.is_empty());
    assert_eq!(session.selected(), Some(local));
    session.read_only = true;
    for command in [
        Command::Start { id: remote },
        Command::Stop { id: remote },
        Command::Close { id: remote },
        Command::Input {
            id: remote,
            data: "x".into(),
        },
    ] {
        assert!(session.web_command(command).is_err());
    }
    assert!(session.web_command(Command::List).is_ok());
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn remote_new_and_close_keep_the_local_workspace_and_tab() {
    let (root, mut session, local, _) = fixture();
    let background = session.data.workspaces[1].id;
    let selected = session.data.selected_workspace;
    let reply = session
        .web_command(Command::New {
            workspace: background,
            profile: "shell".into(),
        })
        .unwrap();
    let Reply::Json(value) = reply else {
        panic!("新建会话应返回会话 ID")
    };
    let id: Uuid = serde_json::from_value(value["id"].clone()).unwrap();
    assert!(session.terminals[&id].running());
    assert_eq!(session.data.selected_workspace, selected);
    assert_eq!(session.selected(), Some(local));
    assert_eq!(session.data.workspaces[1].tabs.len(), 2);
    assert_ne!(
        session.data.workspaces[1].tabs[0].title,
        session.data.workspaces[1].tabs[1].title
    );
    session.web_command(Command::Close { id }).unwrap();
    assert_eq!(session.data.workspaces[1].tabs.len(), 1);
    assert_eq!(session.selected(), Some(local));
    drop(session);
    fs::remove_dir_all(root).unwrap();
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn open(
    server: &Server,
    origin: &str,
) -> Result<Socket, tokio_tungstenite::tungstenite::Error> {
    let mut request = format!("ws://{}/ws", server.address)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    connect_async(request).await.map(|(socket, _)| socket)
}
async fn receive(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(15), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
async fn command(socket: &mut Socket, request: u64, command: Value) -> Value {
    socket
        .send(Message::Text(
            json!({"request": request, "command": command})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    loop {
        let value = receive(socket).await;
        if value["request"] == request {
            return value;
        }
    }
}

#[tokio::test]
async fn websocket_auth_origin_shared_pty_and_shutdown() {
    let (root, session, local, remote) = fixture();
    let store = session.store.clone();
    let data = session.data.clone();
    drop(session);
    let (sender, receiver) = mpsc::sync_channel::<Request>(64);
    let worker = std::thread::spawn(move || {
        let mut session = Session::new(store, data, None);
        while let Ok(request) = receiver.recv() {
            if request.reply.is_closed() {
                continue;
            }
            let result = session
                .web_command(request.command)
                .map_err(|e| e.to_string());
            assert_eq!(session.selected(), Some(local));
            let _ = request.reply.send(result);
        }
    });
    let server = Server::start(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        Arc::new(move |r| sender.try_send(r).map_err(|e| e.to_string())),
    )
    .unwrap();
    let origin = format!("http://{}", server.address);
    assert!(open(&server, "http://untrusted.example").await.is_err());
    let mut rejected = open(&server, &origin).await.unwrap();
    rejected
        .send(Message::Text(
            json!({"op":"auth","token":"wrong"}).to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(receive(&mut rejected).await["type"], "error");
    let mut socket = open(&server, &origin).await.unwrap();
    socket
        .send(Message::Text(
            json!({"op":"auth","token":server.token}).to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(receive(&mut socket).await["type"], "ready");
    let list = command(&mut socket, 1, json!({"op":"list"})).await;
    assert_eq!(list["data"]["workspaces"].as_array().unwrap().len(), 2);
    assert_eq!(
        command(&mut socket, 2, json!({"op":"start","id":remote})).await["type"],
        "reply"
    );
    let snapshot = command(&mut socket, 3, json!({"op":"subscribe","id":remote})).await;
    assert_eq!(snapshot["type"], "snapshot");
    let mut parser = vt100::Parser::new(30, 100, 2000);
    parser.process(
        &snapshot["bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>(),
    );
    #[cfg(windows)]
    let input = "[Console]::WriteLine(('WEB_' + 'PTY_中文'))\r";
    #[cfg(not(windows))]
    let input = "printf 'WEB_%s\\n' 'PTY_中文'\r";
    assert_eq!(
        command(
            &mut socket,
            4,
            json!({"op":"input","id":remote,"data":input})
        )
        .await["type"],
        "reply"
    );
    loop {
        let value = receive(&mut socket).await;
        if value["type"] == "output" {
            parser.process(
                &value["bytes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>(),
            );
            if parser.screen().contents().contains("WEB_PTY_中文") {
                break;
            }
        }
    }
    let mut viewer = open(&server, &origin).await.unwrap();
    viewer
        .send(Message::Text(
            json!({"op":"auth","token":server.token}).to_string().into(),
        ))
        .await
        .unwrap();
    assert_eq!(receive(&mut viewer).await["type"], "ready");
    let snapshot = command(&mut viewer, 5, json!({"op":"subscribe","id":remote})).await;
    let mut replay = vt100::Parser::new(30, 100, 2000);
    replay.process(
        &snapshot["bytes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect::<Vec<_>>(),
    );
    assert!(replay.screen().contents().contains("WEB_PTY_中文"));
    assert_eq!(
        command(&mut socket, 6, json!({"op":"stop","id":remote})).await["type"],
        "reply"
    );
    let list = command(&mut socket, 7, json!({"op":"list"})).await;
    assert_eq!(list["data"]["workspaces"][1]["tabs"][0]["running"], false);
    drop(socket);
    drop(viewer);
    drop(rejected);
    let port = server.address;
    drop(server);
    worker.join().unwrap();
    assert!(std::net::TcpStream::connect(port).is_err());
    fs::remove_dir_all(root).unwrap();
}

/// 为浏览器验收启动隔离的数据目录，不读取或操作用户的真实会话。
#[test]
#[ignore = "需显式指定 VIBE_WEB_FIXTURE_FILE，配合浏览器交互验收"]
fn browser_fixture() {
    let endpoint = std::path::PathBuf::from(
        std::env::var_os("VIBE_WEB_FIXTURE_FILE").expect("缺少浏览器验收端点文件路径"),
    );
    let (root, mut session, _, remote) = fixture();
    session.start(remote);
    let (sender, receiver) = mpsc::sync_channel::<Request>(64);
    let server = Server::start(
        "0.0.0.0:0".parse().unwrap(),
        Arc::new(move |r| sender.try_send(r).map_err(|e| e.to_string())),
    )
    .unwrap();
    fs::write(&endpoint, json!({"url": server.url("127.0.0.1"), "port": server.address.port(), "root": root, "id": remote}).to_string()).unwrap();
    while !endpoint.with_extension("stop").exists() {
        if let Ok(request) = receiver.recv_timeout(Duration::from_millis(100))
            && !request.reply.is_closed()
        {
            let _ = request.reply.send(
                session
                    .web_command(request.command)
                    .map_err(|e| e.to_string()),
            );
        }
    }
    drop(server);
    drop(session);
    fs::remove_dir_all(root).unwrap();
}
