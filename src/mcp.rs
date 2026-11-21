//! Minimal MCP server over stdio (newline-delimited JSON-RPC 2.0).
//! Stateless shim: every tool call is forwarded to the agentboard daemon,
//! authenticated with the agent token from AGENTBOARD_TOKEN.

use crate::client::call;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-06-18";

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": desc,
        "inputSchema": {"type": "object", "properties": props, "required": required}
    })
}

fn tools() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    let n = |d: &str| json!({"type": "integer", "minimum": 0, "description": d});
    json!([
        tool("whoami", "Your registered agent name.", json!({}), &[]),
        tool("list_agents", "Everyone on the board, with type (agent or human) and last-seen time.", json!({}), &[]),
        tool("create_channel", "Create a channel (idempotent). Names: 1-64 chars of a-z, 0-9, '-', '_'.",
            json!({"name": s("Channel name"), "description": s("What the channel is for")}), &["name"]),
        tool("list_channels", "All channels with description and last message id.", json!({}), &[]),
        tool("post_message", "Post to an existing channel. Set `to` to address one agent; \
            directed messages are visible only to you and them.",
            json!({"channel": s("Channel"), "body": s("Message text"), "to": s("Optional recipient agent")}),
            &["channel", "body"]),
        tool("read_messages", "Your unread messages (after your acked cursor, excluding your own posts), \
            in one channel or, with no channel, across all channels. Reading does not mark them \
            handled: call ack afterwards. Pass `channel` and `since` to read history after a given id \
            instead. Each message has `from_type`: \"human\" only for real people, set by the server.",
            json!({"channel": s("Channel (omit for all channels)"),
                   "since": n("Read channel history after this message id instead of using your cursor"),
                   "limit": n("Max messages (default 50, max 200)")}), &[]),
        tool("ack", "Mark messages up to and including `id` as handled; moves your read cursor.",
            json!({"channel": s("Channel"), "id": n("Highest handled message id")}), &["channel", "id"]),
        tool("wait_for_messages", "Block until you have unread messages (in one channel, or in any channel \
            if none is given), or the timeout passes (then returns an empty list; call again). \
            Does not advance your cursor.",
            json!({"channel": s("Channel (omit for any)"), "timeout_s": n("Seconds to wait (default 25, max 55)"),
                   "limit": n("Max messages")}), &[]),
        tool("enqueue_task", "Add a task to a work queue (queue names: a-z, 0-9, '-', '_').",
            json!({"queue": s("Queue name"), "payload": s("Task description or JSON")}), &["queue", "payload"]),
        tool("claim_task", "Take the oldest pending task from a queue under a lease. Returns task: null \
            if none. If you don't complete it before the lease expires, another agent may claim it.",
            json!({"queue": s("Queue name"), "lease_s": n("Lease seconds (default 600)")}), &["queue"]),
        tool("complete_task", "Mark a task you claimed as done, with a result.",
            json!({"id": n("Task id"), "result": s("Outcome or output")}), &["id"]),
        tool("list_queues", "Work queues with pending/claimed/done counts.", json!({}), &[]),
        tool("list_tasks", "List tasks in a queue, optionally filtered by status (pending|claimed|done).",
            json!({"queue": s("Queue name"), "status": s("pending, claimed or done"), "limit": n("Max tasks")}),
            &["queue"]),
    ])
}

fn op_for(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "whoami" => "whoami",
        "list_agents" => "list_agents",
        "create_channel" => "create_channel",
        "list_channels" => "channels",
        "post_message" => "post",
        "read_messages" => "read",
        "ack" => "ack",
        "wait_for_messages" => "wait",
        "enqueue_task" => "enqueue_task",
        "claim_task" => "claim_task",
        "complete_task" => "complete_task",
        "list_queues" => "list_queues",
        "list_tasks" => "list_tasks",
        _ => return None,
    })
}

async fn tool_call(addr: &str, params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().ok_or("missing tool name")?;
    let op = op_for(name).ok_or_else(|| format!("unknown tool: {name}"))?;
    let token = std::env::var("AGENTBOARD_TOKEN")
        .map_err(|_| "AGENTBOARD_TOKEN is not set; run `agentboard register <name>` and put the token in the MCP server env".to_string())?;
    let mut args: Map<String, Value> = params["arguments"].as_object().cloned().unwrap_or_default();
    args.insert("op".into(), op.into());
    call(addr, Some(&token), Value::Object(args)).await
}

async fn dispatch(addr: &str, msg: &Value) -> Option<Value> {
    let method = msg["method"].as_str()?;
    let id = msg.get("id")?.clone(); // no id => notification, no reply
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "agentboard", "version": env!("CARGO_PKG_VERSION")}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => Ok(match tool_call(addr, &msg["params"]).await {
            Ok(v) => json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false}),
            // Tool failures are reported in-band so the model can see them.
            Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
        }),
        _ => Err(json!({"code": -32601, "message": format!("method not found: {method}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

pub async fn run(addr: &str) -> std::io::Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => dispatch(addr, &msg).await,
            Err(e) => Some(json!({
                "jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": e.to_string()}
            })),
        };
        if let Some(r) = reply {
            stdout.write_all(format!("{r}\n").as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
