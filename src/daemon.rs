//! WebSocket daemon holding the board.
//!
//! Each text frame is one JSON request:
//!   {"id": <any>, "token": "...", "op": "post", ...fields}
//! and gets one reply frame:
//!   {"id": <echoed>, "ok": true, "data": {...}}  or  {"id": ..., "ok": false, "error": "..."}
//! Only `register` works without a token. Requests on one connection are
//! handled in order; open another connection for concurrent calls.

use crate::board::Board;
use futures_util::{SinkExt, StreamExt};
use serde_derive::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Mutex};
use tokio::time::{timeout_at, Instant};
use tokio_tungstenite::tungstenite::Message as Ws;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;
const MAX_WAIT_S: u64 = 55;
const DEFAULT_WAIT_S: u64 = 25;
const DEFAULT_LEASE_S: u64 = 600;

#[derive(Deserialize)]
struct Envelope {
    token: Option<String>,
    #[serde(flatten)]
    req: Request,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Register {
        name: String,
        #[serde(rename = "type", default = "default_kind")]
        kind: String,
    },
    Whoami,
    ListAgents,
    CreateChannel { name: String, #[serde(default)] description: String },
    Channels,
    Post { channel: String, body: String, to: Option<String> },
    Read { channel: String, since: Option<u64>, limit: Option<u32> },
    /// Human-type agents only: all messages, including other agents' directed ones.
    Observe { channel: String, since: Option<u64>, limit: Option<u32> },
    Ack { channel: String, id: u64 },
    Wait { channel: String, timeout_s: Option<u64>, limit: Option<u32> },
    EnqueueTask { queue: String, payload: String },
    ClaimTask { queue: String, lease_s: Option<u64> },
    CompleteTask { id: u64, #[serde(default)] result: String },
    ListQueues,
    ListTasks { queue: String, status: Option<String>, limit: Option<u32> },
}

fn default_kind() -> String {
    "agent".into()
}

struct State {
    board: Mutex<Board>,
    /// Bumped on every post; wakes blocked `wait` calls.
    posted: watch::Sender<u64>,
}

type Res = Result<Value, String>;

fn clamp_limit(l: Option<u32>) -> u32 {
    l.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

async fn handle(st: &State, env: Envelope) -> Res {
    if let Request::Register { name, kind } = &env.req {
        let token = st.board.lock().await.register(name, kind)?;
        return Ok(json!({"name": name, "type": kind, "token": token}));
    }
    let token = env.token.ok_or("missing token; register with `agentboard register <name>`")?;
    let (me, kind) = st.board.lock().await.authenticate(&token)?;

    match env.req {
        Request::Register { .. } => unreachable!(),
        Request::Whoami => Ok(json!({"name": me, "type": kind})),
        Request::ListAgents => Ok(json!({"agents": st.board.lock().await.agents()?})),
        Request::CreateChannel { name, description } => {
            Ok(json!({"channel": st.board.lock().await.create_channel(&name, &description)?}))
        }
        Request::Channels => Ok(json!({"channels": st.board.lock().await.channels()?})),
        Request::Post { channel, body, to } => {
            let m = st.board.lock().await.post(&me, &channel, to.as_deref(), &body)?;
            st.posted.send_modify(|n| *n += 1);
            Ok(json!({"message": m}))
        }
        Request::Read { channel, since, limit } => {
            let msgs = st.board.lock().await.read(&me, &channel, since, clamp_limit(limit))?;
            Ok(json!({"messages": msgs}))
        }
        Request::Observe { channel, since, limit } => {
            if kind != "human" {
                return Err("observe is only available to human-type agents".into());
            }
            let msgs = st.board.lock().await.observe(&channel, since.unwrap_or(0), clamp_limit(limit))?;
            Ok(json!({"messages": msgs}))
        }
        Request::Ack { channel, id } => {
            Ok(json!({"cursor": st.board.lock().await.ack(&me, &channel, id)?}))
        }
        Request::Wait { channel, timeout_s, limit } => {
            let secs = timeout_s.unwrap_or(DEFAULT_WAIT_S).min(MAX_WAIT_S);
            let deadline = Instant::now() + Duration::from_secs(secs);
            // Subscribe before checking so a post in between cannot be missed.
            let mut rx = st.posted.subscribe();
            loop {
                let msgs = st.board.lock().await.read(&me, &channel, None, clamp_limit(limit))?;
                if !msgs.is_empty() {
                    return Ok(json!({"messages": msgs}));
                }
                match timeout_at(deadline, rx.changed()).await {
                    Ok(Ok(())) => continue,
                    _ => return Ok(json!({"messages": []})),
                }
            }
        }
        Request::EnqueueTask { queue, payload } => {
            Ok(json!({"task": st.board.lock().await.enqueue_task(&me, &queue, &payload)?}))
        }
        Request::ClaimTask { queue, lease_s } => {
            let t = st.board.lock().await.claim_task(&me, &queue, lease_s.unwrap_or(DEFAULT_LEASE_S))?;
            Ok(json!({"task": t}))
        }
        Request::CompleteTask { id, result } => {
            Ok(json!({"task": st.board.lock().await.complete_task(&me, id, &result)?}))
        }
        Request::ListQueues => Ok(json!({"queues": st.board.lock().await.queues()?})),
        Request::ListTasks { queue, status, limit } => {
            let t = st.board.lock().await.list_tasks(&queue, status.as_deref(), clamp_limit(limit))?;
            Ok(json!({"tasks": t}))
        }
    }
}

fn reply(id: Value, r: Res) -> String {
    match r {
        Ok(data) => json!({"id": id, "ok": true, "data": data}),
        Err(error) => json!({"id": id, "ok": false, "error": error}),
    }
    .to_string()
}

async fn process(st: &State, text: &str) -> String {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return reply(Value::Null, Err(format!("bad json: {e}"))),
    };
    let id = v["id"].clone();
    match serde_json::from_value::<Envelope>(v) {
        Ok(env) => reply(id, handle(st, env).await),
        Err(e) => reply(id, Err(format!("bad request: {e}"))),
    }
}

async fn connection(st: Arc<State>, sock: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    let ws = tokio_tungstenite::accept_async(sock).await?;
    let (mut tx, mut rx) = ws.split();
    while let Some(frame) = rx.next().await {
        match frame? {
            Ws::Text(t) => {
                let out = process(&st, t.as_str()).await;
                tx.send(Ws::text(out)).await?;
            }
            Ws::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

pub async fn serve(addr: &str, db_path: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let board = Board::open(db_path).map_err(std::io::Error::other)?;
    eprintln!("agentboard listening on ws://{addr}, db {db_path}");
    let st = Arc::new(State { board: Mutex::new(board), posted: watch::channel(0).0 });
    loop {
        let (sock, _) = listener.accept().await?;
        let st = st.clone();
        tokio::spawn(async move {
            if let Err(e) = connection(st, sock).await {
                eprintln!("connection error: {e}");
            }
        });
    }
}
