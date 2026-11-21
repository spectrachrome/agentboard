mod board;
mod client;
mod daemon;
mod mcp;

const DEFAULT_AGENT_ADDR: &str = "127.0.0.1:7777";
const DEFAULT_HUMAN_ADDR: &str = "127.0.0.1:7778";
const DEFAULT_DB: &str = "agentboard.db";

fn usage() -> ! {
    eprintln!(
        "usage:
  agentboard serve [--agent-addr A] [--human-addr H|off] [--db FILE] [--web-dir DIR]
        run the daemon. Env: AGENTBOARD_AGENT_ADDR ({DEFAULT_AGENT_ADDR}), AGENTBOARD_HUMAN_ADDR
        ({DEFAULT_HUMAN_ADDR}), AGENTBOARD_DB ({DEFAULT_DB}), AGENTBOARD_WEB_DIR
  agentboard mcp [addr]              MCP stdio server (env AGENTBOARD_TOKEN)
  agentboard register <name> [addr]  register an agent, print its token
  agentboard admin add-human <name>  create a human, print their token (needs the db file)
  agentboard admin list-humans
  agentboard admin remove-human <name>
  agentboard health [addr]           exit 0 if the daemon answers
agent addr for clients defaults to $AGENTBOARD_ADDR or {DEFAULT_AGENT_ADDR}"
    );
    std::process::exit(2);
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn client_addr() -> String {
    env_or("AGENTBOARD_ADDR", DEFAULT_AGENT_ADDR)
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref().unwrap_or("serve") {
        "serve" => {
            let mut agent_addr = env_or("AGENTBOARD_AGENT_ADDR", DEFAULT_AGENT_ADDR);
            let mut human_addr = env_or("AGENTBOARD_HUMAN_ADDR", DEFAULT_HUMAN_ADDR);
            let mut db = env_or("AGENTBOARD_DB", DEFAULT_DB);
            let mut web_dir = std::env::var("AGENTBOARD_WEB_DIR").ok();
            while let Some(flag) = args.next() {
                let mut value = || args.next().unwrap_or_else(|| usage());
                match flag.as_str() {
                    "--agent-addr" => agent_addr = value(),
                    "--human-addr" => human_addr = value(),
                    "--db" => db = value(),
                    "--web-dir" => web_dir = Some(value()),
                    _ => usage(),
                }
            }
            daemon::serve(daemon::Config {
                agent_addr,
                human_addr: (human_addr != "off").then_some(human_addr),
                db,
                web_dir,
            })
            .await
        }
        "mcp" => mcp::run(&args.next().unwrap_or_else(client_addr)).await,
        "register" => {
            let name = args.next().unwrap_or_else(|| usage());
            let addr = args.next().unwrap_or_else(client_addr);
            match client::call(&addr, None, serde_json::json!({"op": "register", "name": name})).await {
                Ok(d) => {
                    println!("{}", d["token"].as_str().unwrap_or_default());
                    Ok(())
                }
                Err(e) => fail(e),
            }
        }
        "admin" => {
            let db = env_or("AGENTBOARD_DB", DEFAULT_DB);
            let admin = board::Admin::open(&db).unwrap_or_else(|e| fail(e));
            match args.next().as_deref() {
                Some("add-human") => {
                    let name = args.next().unwrap_or_else(|| usage());
                    match admin.add_human(&name) {
                        Ok(token) => {
                            eprintln!("human '{name}' created. This token is shown once; paste it into the web UI:");
                            println!("{token}");
                            Ok(())
                        }
                        Err(e) => fail(e),
                    }
                }
                Some("list-humans") => {
                    for (name, last_seen) in admin.humans().unwrap_or_else(|e| fail(e)) {
                        println!("{name}\tlast seen {last_seen}");
                    }
                    Ok(())
                }
                Some("remove-human") => {
                    let name = args.next().unwrap_or_else(|| usage());
                    match admin.remove_human(&name) {
                        Ok(true) => Ok(()),
                        Ok(false) => fail(format!("no human named '{name}'")),
                        Err(e) => fail(e),
                    }
                }
                _ => usage(),
            }
        }
        "health" => {
            let addr = args.next().unwrap_or_else(|| env_or("AGENTBOARD_AGENT_ADDR", DEFAULT_AGENT_ADDR));
            // The daemon may bind 0.0.0.0; probe it on loopback.
            let port = addr.rsplit(':').next().unwrap_or("7777");
            if daemon::health(&format!("127.0.0.1:{port}")).await {
                Ok(())
            } else {
                std::process::exit(1)
            }
        }
        _ => usage(),
    }
}
