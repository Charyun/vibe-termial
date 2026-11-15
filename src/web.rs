//! 局域网及 Tailscale 共享终端；会话操作统一交回桌面线程执行。
use crate::{model::Tab, session::Session, terminal::Buffer};
use anyhow::{Context, Result, bail};
use axum::{
    Router,
    extract::{
        ConnectInfo, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    net::{IpAddr, SocketAddr, TcpListener, UdpSocket},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tokio::sync::{Semaphore, broadcast, oneshot, watch};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum Output {
    Data(Vec<u8>),
    Resize(u16, u16),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    List,
    Subscribe { id: Uuid },
    Input { id: Uuid, data: String },
    Paste { id: Uuid, data: String },
    Start { id: Uuid },
    Stop { id: Uuid },
    Restart { id: Uuid },
    New { workspace: Uuid, profile: String },
    Close { id: Uuid },
}

pub struct Stream {
    buffer: Arc<Mutex<Buffer>>,
    receiver: broadcast::Receiver<Output>,
    initial: Vec<u8>,
    rows: u16,
    cols: u16,
}
pub enum Reply {
    Json(Value),
    Stream(Stream),
}
pub struct Request {
    pub command: Command,
    pub reply: oneshot::Sender<std::result::Result<Reply, String>>,
}
pub type Dispatch = Arc<dyn Fn(Request) -> std::result::Result<(), String> + Send + Sync>;

impl Session {
    pub fn web_command(&mut self, command: Command) -> Result<Reply> {
        if self.read_only && !matches!(command, Command::List | Command::Subscribe { .. }) {
            bail!("当前实例为只读模式");
        }
        let value = match command {
            Command::List => {
                let workspaces: Vec<_> = self
                    .data
                    .workspaces
                    .iter()
                    .map(|w| {
                        let tabs: Vec<_> = w.tabs.iter().map(|t| json!({
                        "id": t.id, "title": t.title, "profile": t.profile.name,
                        "running": self.terminals.get(&t.id).is_some_and(|t| t.running()),
                        "instance": self.terminals.get(&t.id).map(|t| t.instance),
                    })).collect();
                        json!({"id": w.id, "name": w.name, "tabs": tabs})
                    })
                    .collect();
                json!({"workspaces": workspaces, "profiles": self.data.profiles,
                    "selected": self.selected(), "read_only": self.read_only})
            }
            Command::Subscribe { id } => {
                let tab = self.data.tab(id).context("会话不存在")?;
                let buffer = if let Some(t) = self.terminals.get(&id) {
                    t.buffer.clone()
                } else {
                    let mut b = Buffer::new(
                        tab.rows.clamp(2, 250),
                        tab.cols.clamp(10, 500),
                        tab.cwd.clone(),
                    );
                    b.replay(&self.store.history(id)?);
                    Arc::new(Mutex::new(b))
                };
                let mut b = buffer
                    .lock()
                    .map_err(|_| anyhow::anyhow!("终端缓冲区不可用"))?;
                // 在同一把锁中订阅和取快照，避免首屏与增量输出之间丢字或重复。
                let receiver = b.output.subscribe();
                let initial = b.snapshot();
                let (rows, cols) = b.parser.screen().size();
                drop(b);
                return Ok(Reply::Stream(Stream {
                    buffer,
                    receiver,
                    initial,
                    rows,
                    cols,
                }));
            }
            Command::Input { id, data } | Command::Paste { id, data } if data.len() > 65536 => {
                let _ = id;
                bail!("单次输入不能超过 64 KB");
            }
            Command::Input { id, data } => {
                self.terminals
                    .get(&id)
                    .context("会话未运行")?
                    .send(data.into_bytes())?;
                json!({})
            }
            Command::Paste { id, data } => {
                self.terminals
                    .get(&id)
                    .context("会话未运行")?
                    .paste(&data)?;
                json!({})
            }
            Command::Start { id } | Command::Restart { id } => {
                self.data.tab(id).context("会话不存在")?;
                self.error = None;
                if matches!(command, Command::Restart { .. }) {
                    self.restart(id);
                } else {
                    self.start(id);
                }
                if let Some(error) = self.error.take() {
                    bail!("{error}");
                }
                json!({"id": id})
            }
            Command::Stop { id } => {
                self.data.tab(id).context("会话不存在")?;
                if let Some(t) = self.terminals.get_mut(&id) {
                    t.stop();
                }
                self.checkpoint();
                json!({"id": id})
            }
            Command::Close { id } => {
                self.data.tab(id).context("会话不存在")?;
                self.close_tab(id);
                json!({"id": id})
            }
            Command::New { workspace, profile } => {
                let profile = self
                    .data
                    .profiles
                    .iter()
                    .find(|p| p.id == profile)
                    .context("CLI 配置不存在")?
                    .clone();
                let w = self
                    .data
                    .workspaces
                    .iter_mut()
                    .find(|w| w.id == workspace)
                    .context("工作区不存在")?;
                let mut tab = Tab::new(profile, w.directory.clone());
                let base = tab.title.clone();
                let mut ordinal = 2;
                while w.tabs.iter().any(|t| t.title == tab.title) {
                    tab.title = format!("{base} {ordinal}");
                    ordinal += 1;
                }
                let id = tab.id;
                w.tabs.push(tab);
                self.persist();
                self.error = None;
                self.start(id);
                if let Some(error) = self.error.take() {
                    bail!("{error}");
                }
                json!({"id": id})
            }
        };
        Ok(Reply::Json(value))
    }
}

#[derive(Clone)]
struct ServerState {
    token: String,
    dispatch: Dispatch,
    shutdown: watch::Receiver<bool>,
    slots: Arc<Semaphore>,
}
pub struct Server {
    pub address: SocketAddr,
    pub token: String,
    shutdown: watch::Sender<bool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(address: SocketAddr, dispatch: Dispatch) -> Result<Self> {
        let listener = TcpListener::bind(address).context("Web 端口监听失败")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let (shutdown, receiver) = watch::channel(false);
        let state = ServerState {
            token: token.clone(),
            dispatch,
            shutdown: receiver.clone(),
            slots: Arc::new(Semaphore::new(16)),
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let thread = thread::spawn(move || {
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => l,
                    Err(_) => return,
                };
                let app = Router::new()
                    .route(
                        "/",
                        get(|| async {
                            asset(
                                "text/html; charset=utf-8",
                                include_str!("../assets/web/index.html"),
                            )
                        }),
                    )
                    .route(
                        "/app.js",
                        get(|| async {
                            asset(
                                "text/javascript; charset=utf-8",
                                include_str!("../assets/web/app.js"),
                            )
                        }),
                    )
                    .route(
                        "/style.css",
                        get(|| async {
                            asset(
                                "text/css; charset=utf-8",
                                include_str!("../assets/web/style.css"),
                            )
                        }),
                    )
                    .route(
                        "/xterm.js",
                        get(|| async {
                            asset(
                                "text/javascript; charset=utf-8",
                                include_str!("../assets/web/vendor/xterm.js"),
                            )
                        }),
                    )
                    .route(
                        "/xterm.css",
                        get(|| async {
                            asset(
                                "text/css; charset=utf-8",
                                include_str!("../assets/web/vendor/xterm.css"),
                            )
                        }),
                    )
                    .route("/ws", get(upgrade))
                    .with_state(state);
                let mut receiver = receiver;
                let _ = axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(async move {
                    let _ = receiver.changed().await;
                })
                .await;
            })
        });
        Ok(Self {
            address,
            token,
            shutdown,
            thread: Some(thread),
        })
    }
    pub fn url(&self, host: &str) -> String {
        format!(
            "http://{host}:{}/#token={}",
            self.address.port(),
            self.token
        )
    }
    pub fn lan_url(&self) -> String {
        let ip = UdpSocket::bind("0.0.0.0:0")
            .and_then(|s| {
                s.connect("192.0.2.1:80")?;
                s.local_addr()
            })
            .map(|a| a.ip())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        self.url(&ip.to_string())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn asset(kind: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "no-store"),
        (header::REFERRER_POLICY, "no-referrer"),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::CONTENT_SECURITY_POLICY, "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'")], body).into_response()
}
fn valid_origin(headers: &HeaderMap, peer: SocketAddr) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| url::Url::parse(h).ok())
    else {
        return false;
    };
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    let authority = origin[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    let forwarded = peer
        .ip()
        .is_loopback()
        .then(|| {
            headers
                .get("x-forwarded-host")
                .and_then(|h| h.to_str().ok())
        })
        .flatten();
    host == Some(authority.as_str()) || forwarded == Some(authority.as_str())
}
async fn upgrade(
    State(state): State<ServerState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !valid_origin(&headers, peer) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(slot) = state.slots.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    ws.max_message_size(128 * 1024)
        .max_frame_size(128 * 1024)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            socket_loop(socket, state).await;
        })
}
async fn dispatch(state: &ServerState, command: Command) -> std::result::Result<Reply, String> {
    let (reply, receive) = oneshot::channel();
    (state.dispatch)(Request { command, reply })?;
    let mut shutdown = state.shutdown.clone();
    tokio::select! {
        _ = shutdown.changed() => Err("Web 服务已关闭".into()),
        result = tokio::time::timeout(Duration::from_secs(10), receive) => {
            result.map_err(|_| "桌面响应超时".to_owned())?.map_err(|_| "桌面连接已断开".to_owned())?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_proxy_origin_only_trusts_forwarded_host_from_loopback() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:8787".parse().unwrap());
        headers.insert(
            header::ORIGIN,
            "https://desktop.example.ts.net".parse().unwrap(),
        );
        headers.insert(
            "x-forwarded-host",
            "desktop.example.ts.net".parse().unwrap(),
        );
        assert!(valid_origin(&headers, "127.0.0.1:12345".parse().unwrap()));
        assert!(!valid_origin(
            &headers,
            "192.168.1.2:12345".parse().unwrap()
        ));
        headers.remove("x-forwarded-host");
        assert!(!valid_origin(&headers, "127.0.0.1:12345".parse().unwrap()));
        headers.insert(header::ORIGIN, "http://127.0.0.1:8787".parse().unwrap());
        assert!(valid_origin(&headers, "192.168.1.2:12345".parse().unwrap()));
    }
}
async fn send(socket: &mut WebSocket, value: Value) -> bool {
    matches!(
        tokio::time::timeout(
            Duration::from_secs(3),
            socket.send(Message::Text(value.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    )
}
async fn socket_loop(mut socket: WebSocket, mut state: ServerState) {
    let auth = tokio::select! {
        _ = state.shutdown.changed() => return,
        value = tokio::time::timeout(Duration::from_secs(5), socket.recv()) => value,
    };
    let valid = if let Ok(Some(Ok(Message::Text(text)))) = auth {
        serde_json::from_str::<Value>(&text)
            .ok()
            .is_some_and(|v| v["op"] == "auth" && v["token"].as_str() == Some(&state.token))
    } else {
        false
    };
    if !valid {
        let _ = send(
            &mut socket,
            json!({"type": "error", "error": "访问令牌无效"}),
        )
        .await;
        return;
    }
    if !send(&mut socket, json!({"type": "ready"})).await {
        return;
    }
    let mut stream: Option<Stream> = None;
    loop {
        tokio::select! {
            biased;
            _ = state.shutdown.changed() => break,
            message = socket.recv() => {
                let Some(Ok(Message::Text(text))) = message else { break; };
                let value = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
                let request_id = value.get("request").cloned().unwrap_or(Value::Null);
                let command = serde_json::from_value::<Command>(value.get("command").cloned().unwrap_or(Value::Null));
                let result = match command { Ok(c) => dispatch(&state, c).await, Err(_) => Err("请求格式无效".into()) };
                let response = match result {
                    Ok(Reply::Json(data)) => json!({"type": "reply", "request": request_id, "data": data}),
                    Ok(Reply::Stream(s)) => {
                        let response = json!({"type": "snapshot", "request": request_id, "rows": s.rows, "cols": s.cols, "bytes": s.initial});
                        stream = Some(s); response
                    }
                    Err(error) => json!({"type": "error", "request": request_id, "error": error}),
                };
                if !send(&mut socket, response).await { break; }
            }
            output = async { match &mut stream { Some(s) => s.receiver.recv().await, None => std::future::pending().await } } => {
                let value = match output {
                    Ok(Output::Data(bytes)) => json!({"type": "output", "bytes": bytes}),
                    Ok(Output::Resize(rows, cols)) => json!({"type": "resize", "rows": rows, "cols": cols}),
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let s = stream.as_mut().unwrap();
                        let recovered = s.buffer.lock().ok().map(|mut b| {
                            s.receiver = b.output.subscribe();
                            let bytes = b.snapshot();
                            let (rows, cols) = b.parser.screen().size();
                            json!({"type": "snapshot", "bytes": bytes, "rows": rows, "cols": cols})
                        });
                        match recovered { Some(value) => value, None => break }
                    }
                    Err(broadcast::error::RecvError::Closed) => { stream = None; continue; }
                };
                if !send(&mut socket, value).await { break; }
            }
        }
    }
}
