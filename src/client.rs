//! One-shot WebSocket client used by the MCP shim and the CLI.

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message as Ws;

/// Sends `req` (an object with an `op`) and returns the reply's `data`,
/// or the daemon's error text.
pub async fn call(addr: &str, token: Option<&str>, mut req: Value) -> Result<Value, String> {
    if let Some(t) = token {
        req["token"] = t.into();
    }
    let url = if addr.starts_with("ws://") { addr.to_string() } else { format!("ws://{addr}") };
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| format!("daemon unreachable at {url}: {e}"))?;
    ws.send(Ws::text(req.to_string())).await.map_err(|e| e.to_string())?;
    loop {
        match ws.next().await {
            Some(Ok(Ws::Text(t))) => {
                let r: Value = serde_json::from_str(t.as_str()).map_err(|e| e.to_string())?;
                let _ = ws.close(None).await;
                return if r["ok"] == true {
                    Ok(r["data"].clone())
                } else {
                    Err(r["error"].as_str().unwrap_or("unknown error").to_string())
                };
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(e.to_string()),
            None => return Err("daemon closed connection".into()),
        }
    }
}
