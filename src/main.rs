mod board;
mod client;
mod daemon;
mod mcp;

const DEFAULT_ADDR: &str = "127.0.0.1:7777";

fn usage() -> ! {
    eprintln!(
        "usage:\n  agentboard serve [addr]            run the daemon (env AGENTBOARD_DB, default agentboard.db)\n  \
         agentboard mcp [addr]              MCP stdio server (env AGENTBOARD_TOKEN)\n  \
         agentboard register <name> [--type human] [addr]
                                    register an agent, print its token (type: agent|human)\n\
         addr defaults to $AGENTBOARD_ADDR or {DEFAULT_ADDR}"
    );
    std::process::exit(2);
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "serve".into());
    let default_addr =
        || std::env::var("AGENTBOARD_ADDR").unwrap_or_else(|_| DEFAULT_ADDR.into());
    match mode.as_str() {
        "serve" => {
            let addr = args.next().unwrap_or_else(default_addr);
            let db = std::env::var("AGENTBOARD_DB").unwrap_or_else(|_| "agentboard.db".into());
            daemon::serve(&addr, &db).await
        }
        "mcp" => mcp::run(&args.next().unwrap_or_else(default_addr)).await,
        "register" => {
            let name = args.next().unwrap_or_else(|| usage());
            let mut kind = "agent".to_string();
            let mut addr = None;
            while let Some(a) = args.next() {
                if a == "--type" {
                    kind = args.next().unwrap_or_else(|| usage());
                } else {
                    addr = Some(a);
                }
            }
            let addr = addr.unwrap_or_else(default_addr);
            let req = serde_json::json!({"op": "register", "name": name, "type": kind});
            match client::call(&addr, None, req).await {
                Ok(d) => {
                    println!("{}", d["token"].as_str().unwrap_or_default());
                    Ok(())
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => usage(),
    }
}
