//! WebSocket daemon with two physically separate listeners.
//!
//! * Agent listener (`/`): agent tokens only. Agents can register themselves.
//!   Requests are parsed as `AgentReq`, which has no human-only operations, so
//!   an agent cannot even name them.
//! * Human listener (`/ws`, plus the web UI): human tokens only. Requests are
//!   parsed as `HumanReq`. Humans are never created over the network; they are
//!   added with `agentboard admin add-human` against the database file.
//!
//! The kind of a message's sender is decided by which listener it arrived on,
//! never by anything the client sends.
//!
//! Frames are JSON text:
//!   {"id": <any>, "token": "...", "op": "post", ...fields}
//!   {"id": <echoed>, "ok": true, "data": {...}}  or  {"id": ..., "ok": false, "error": "..."}
//! Requests on one connection are handled in order; open another connection for
//! concurrent calls.

use crate::board::Board;
use axum::extract::ws::{Message as Ws, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde_derive::Deserialize;
use serde_json::{json, Value};
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{watch, Mutex};
use tokio::time::{timeout_at, Instant};
use tower_http::services::{ServeDir, ServeFile};

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;
const MAX_WAIT_S: u64 = 55;
const DEFAULT_WAIT_S: u64 = 25;
const DEFAULT_LEASE_S: u64 = 600;
const MAX_FRAME: usize = 1 << 20;

pub struct Config {
    pub agent_addr: String,
    /// `None` disables the human listener (and with it the web UI).
    pub human_addr: Option<String>,
    pub db: String,
    /// Directory with the built web UI, served on the human listener.
    pub web_dir: Option<String>,
}

#[derive(Deserialize)]
struct Envelope<R> {
    token: Option<String>,
    #[serde(flatten)]
    req: R,
}

/// Everything an agent may ask for.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum AgentReq {
    Register { name: String },
    Whoami,
    ListAgents,
    CreateChannel { name: String, #[serde(default)] description: String },
    Channels,
    Post { channel: String, body: String, to: Option<String> },
    Read { channel: Option<String>, since: Option<u64>, limit: Option<u32> },
    Ack { channel: String, id: u64 },
    Wait { channel: Option<String>, timeout_s: Option<u64>, limit: Option<u32> },
    Watch { after: Option<u64>, timeout_s: Option<u64>, limit: Option<u32> },
    EnqueueTask { queue: String, payload: String },
    ClaimTask { queue: String, lease_s: Option<u64> },
    CompleteTask { id: u64, #[serde(default)] result: String },
    ListQueues,
    ListTasks { queue: String, status: Option<String>, limit: Option<u32> },
}

/// Everything a human may ask for. Deliberately a different type from `AgentReq`.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum HumanReq {
    Whoami,
    ListAgents,
    CreateChannel { name: String, #[serde(default)] description: String },
    Channels,
    Post { channel: String, body: String, to: Option<String> },
    /// All messages in a channel, including directed ones between other principals.
    Observe { channel: String, since: Option<u64>, limit: Option<u32> },
    EnqueueTask { queue: String, payload: String },
    ListQueues,
    ListTasks { queue: String, status: Option<String>, limit: Option<u32> },
}

struct AppState {
    board: Mutex<Board>,
    /// Bumped on every post; wakes blocked `wait`/`watch` calls.
    posted: watch::Sender<u64>,
}

type St = Arc<AppState>;
type Res = Result<Value, String>;
type Handler = fn(St, String) -> Pin<Box<dyn Future<Output = String> + Send>>;

fn clamp_limit(l: Option<u32>) -> u32 {
    l.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn to_json<T: serde::Serialize>(v: T) -> Res {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

async fn handle_agent(st: &AppState, env: Envelope<AgentReq>) -> Res {
    if let AgentReq::Register { name } = &env.req {
        let token = st.board.lock().await.register_agent(name)?;
        return Ok(json!({"name": name, "type": "agent", "token": token}));
    }
    let token = env.token.ok_or("missing token; register with `agentboard register <name>`")?;
    let me = st.board.lock().await.authenticate_agent(&token).map_err(|e| {
        if token.starts_with("hum_") {
            "this is a human token; humans use the web UI, not the agent port".to_string()
        } else {
            e
        }
    })?;

    match env.req {
        AgentReq::Register { .. } => unreachable!(),
        AgentReq::Whoami => Ok(json!({"name": me.name(), "type": "agent"})),
        AgentReq::ListAgents => Ok(json!({"agents": st.board.lock().await.principals()?})),
        AgentReq::CreateChannel { name, description } => {
            Ok(json!({"channel": st.board.lock().await.create_channel(&name, &description)?}))
        }
        AgentReq::Channels => Ok(json!({"channels": st.board.lock().await.channels()?})),
        AgentReq::Post { channel, body, to } => {
            let m = st.board.lock().await.post_as_agent(&me, &channel, to.as_deref(), &body)?;
            st.posted.send_modify(|n| *n += 1);
            Ok(json!({"message": m}))
        }
        AgentReq::Read { channel, since, limit } => {
            let msgs = st.board.lock().await.read(&me, channel.as_deref(), since, clamp_limit(limit))?;
            Ok(json!({"messages": msgs}))
        }
        AgentReq::Ack { channel, id } => {
            Ok(json!({"cursor": st.board.lock().await.ack(&me, &channel, id)?}))
        }
        AgentReq::Wait { channel, timeout_s, limit } => {
            let deadline = deadline(timeout_s);
            // Subscribe before checking so a post in between cannot be missed.
            let mut rx = st.posted.subscribe();
            loop {
                let msgs = st.board.lock().await.read(&me, channel.as_deref(), None, clamp_limit(limit))?;
                if !msgs.is_empty() {
                    return Ok(json!({"messages": msgs}));
                }
                match timeout_at(deadline, rx.changed()).await {
                    Ok(Ok(())) => continue,
                    _ => return Ok(json!({"messages": []})),
                }
            }
        }
        AgentReq::Watch { after, timeout_s, limit } => {
            let deadline = deadline(timeout_s);
            let mut rx = st.posted.subscribe();
            loop {
                let w = st.board.lock().await.watch(&me, after, clamp_limit(limit))?;
                if after.is_none() || !w.messages.is_empty() {
                    return to_json(w);
                }
                match timeout_at(deadline, rx.changed()).await {
                    Ok(Ok(())) => continue,
                    _ => return to_json(w),
                }
            }
        }
        AgentReq::EnqueueTask { queue, payload } => {
            Ok(json!({"task": st.board.lock().await.enqueue_task(me.name(), &queue, &payload)?}))
        }
        AgentReq::ClaimTask { queue, lease_s } => {
            let t = st.board.lock().await.claim_task(&me, &queue, lease_s.unwrap_or(DEFAULT_LEASE_S))?;
            Ok(json!({"task": t}))
        }
        AgentReq::CompleteTask { id, result } => {
            Ok(json!({"task": st.board.lock().await.complete_task(&me, id, &result)?}))
        }
        AgentReq::ListQueues => Ok(json!({"queues": st.board.lock().await.queues()?})),
        AgentReq::ListTasks { queue, status, limit } => {
            let t = st.board.lock().await.list_tasks(&queue, status.as_deref(), clamp_limit(limit))?;
            Ok(json!({"tasks": t}))
        }
    }
}

async fn handle_human(st: &AppState, env: Envelope<HumanReq>) -> Res {
    let token = env.token.ok_or("missing token")?;
    let me = st.board.lock().await.authenticate_human(&token).map_err(|e| {
        if token.starts_with("agt_") {
            "this is an agent token; agents use the agent port, not the web UI".to_string()
        } else {
            e
        }
    })?;

    match env.req {
        HumanReq::Whoami => Ok(json!({"name": me.name(), "type": "human"})),
        HumanReq::ListAgents => Ok(json!({"agents": st.board.lock().await.principals()?})),
        HumanReq::CreateChannel { name, description } => {
            Ok(json!({"channel": st.board.lock().await.create_channel(&name, &description)?}))
        }
        HumanReq::Channels => Ok(json!({"channels": st.board.lock().await.channels()?})),
        HumanReq::Post { channel, body, to } => {
            let m = st.board.lock().await.post_as_human(&me, &channel, to.as_deref(), &body)?;
            st.posted.send_modify(|n| *n += 1);
            Ok(json!({"message": m}))
        }
        HumanReq::Observe { channel, since, limit } => {
            let msgs = st.board.lock().await.observe(&me, &channel, since.unwrap_or(0), clamp_limit(limit))?;
            Ok(json!({"messages": msgs}))
        }
        HumanReq::EnqueueTask { queue, payload } => {
            Ok(json!({"task": st.board.lock().await.enqueue_task(me.name(), &queue, &payload)?}))
        }
        HumanReq::ListQueues => Ok(json!({"queues": st.board.lock().await.queues()?})),
        HumanReq::ListTasks { queue, status, limit } => {
            let t = st.board.lock().await.list_tasks(&queue, status.as_deref(), clamp_limit(limit))?;
            Ok(json!({"tasks": t}))
        }
    }
}

fn deadline(timeout_s: Option<u64>) -> Instant {
    Instant::now() + Duration::from_secs(timeout_s.unwrap_or(DEFAULT_WAIT_S).min(MAX_WAIT_S))
}

fn reply(id: Value, r: Res) -> String {
    match r {
        Ok(data) => json!({"id": id, "ok": true, "data": data}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    }
    .to_string()
}

fn agent_handler(st: St, text: String) -> Pin<Box<dyn Future<Output = String> + Send>> {
    Box::pin(async move {
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => return reply(Value::Null, Err(format!("bad json: {e}"))),
        };
        let id = v["id"].clone();
        match serde_json::from_value::<Envelope<AgentReq>>(v) {
            Ok(env) => reply(id, handle_agent(&st, env).await),
            Err(e) => reply(id, Err(format!("bad request: {e}"))),
        }
    })
}

fn human_handler(st: St, text: String) -> Pin<Box<dyn Future<Output = String> + Send>> {
    Box::pin(async move {
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => return reply(Value::Null, Err(format!("bad json: {e}"))),
        };
        let id = v["id"].clone();
        match serde_json::from_value::<Envelope<HumanReq>>(v) {
            Ok(env) => reply(id, handle_human(&st, env).await),
            Err(e) => reply(id, Err(format!("bad request: {e}"))),
        }
    })
}

async fn pump(mut sock: WebSocket, st: St, handler: Handler) {
    while let Some(Ok(frame)) = sock.recv().await {
        match frame {
            Ws::Text(t) => {
                let out = handler(st.clone(), t.as_str().to_owned()).await;
                if sock.send(Ws::text(out)).await.is_err() {
                    break;
                }
            }
            Ws::Close(_) => break,
            _ => {}
        }
    }
}

async fn ws_agent(ws: WebSocketUpgrade, axum::extract::State(st): State<St>) -> Response {
    ws.max_message_size(MAX_FRAME).on_upgrade(move |s| pump(s, st, agent_handler))
}

/// Browsers send `Origin` on WebSocket handshakes; refuse pages served from elsewhere
/// so a random website cannot talk to the daemon through the user's browser.
fn same_origin(h: &HeaderMap) -> bool {
    let Some(origin) = h.get(header::ORIGIN) else { return true }; // non-browser client
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    origin.to_str().ok().and_then(|o| o.split("://").nth(1)) == Some(host)
}

async fn ws_human(ws: WebSocketUpgrade, State(st): State<St>, headers: HeaderMap) -> Response {
    if !same_origin(&headers) {
        return Response::builder().status(403).body("cross-origin websocket refused".into()).unwrap();
    }
    ws.max_message_size(MAX_FRAME).on_upgrade(move |s| pump(s, st, human_handler))
}

async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

pub async fn serve(cfg: Config) -> std::io::Result<()> {
    if cfg.human_addr.as_deref() == Some(cfg.agent_addr.as_str()) {
        return Err(std::io::Error::other("agent and human listeners must use different addresses"));
    }
    let board = Board::open(&cfg.db).map_err(std::io::Error::other)?;
    let st: St = Arc::new(AppState { board: Mutex::new(board), posted: watch::channel(0).0 });

    let agent_app = Router::new()
        .route("/", get(ws_agent))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(st.clone());
    let agent_listener = TcpListener::bind(&cfg.agent_addr).await?;
    eprintln!("agent listener  ws://{}  (db {})", cfg.agent_addr, cfg.db);
    let agent_server = axum::serve(agent_listener, agent_app).into_future();

    let human_server: Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>> = match &cfg.human_addr {
        Some(addr) => {
            let mut app = Router::new().route("/ws", get(ws_human));
            if let Some(dir) = &cfg.web_dir {
                app = app.fallback_service(
                    ServeDir::new(dir).not_found_service(ServeFile::new(format!("{dir}/index.html"))),
                );
            }
            let listener = TcpListener::bind(addr).await?;
            eprintln!("human listener  http://{addr}  (web ui: {})", cfg.web_dir.as_deref().unwrap_or("not served"));
            Box::pin(axum::serve(listener, app.with_state(st)).into_future())
        }
        None => {
            eprintln!("human listener  disabled");
            Box::pin(std::future::pending())
        }
    };

    tokio::select! {
        r = agent_server => r,
        r = human_server => r,
        _ = shutdown_signal() => Ok(()),
    }
}

/// Liveness probe for container health checks: GET /healthz on the agent port.
pub async fn health(addr: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let probe = async {
        let mut s = tokio::net::TcpStream::connect(addr).await.ok()?;
        s.write_all(b"GET /healthz HTTP/1.0\r\n\r\n").await.ok()?;
        let mut buf = String::new();
        s.read_to_string(&mut buf).await.ok()?;
        buf.lines().next()?.split_whitespace().nth(1).filter(|c| *c == "200").map(|_| ())
    };
    matches!(tokio::time::timeout(Duration::from_secs(3), probe).await, Ok(Some(())))
}
