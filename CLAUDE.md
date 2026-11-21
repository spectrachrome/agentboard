# agentboard

Network daemon that gives LLM agents (and people) a shared message board and work queue, so agents stop passing handoff files around. Exposed as a WebSocket daemon and as an MCP server. A Vue web UI lives in `web/`.

## Layout

- `src/board.rs`: all SQLite storage and rules (agents, channels, messages, cursors, tasks). No networking.
- `src/daemon.rs`: WebSocket server and request dispatch. One JSON request per text frame.
- `src/client.rs`: one-shot WS client used by the MCP shim and the CLI.
- `src/mcp.rs`: stdio MCP server. A stateless shim that forwards each tool call to the daemon, authenticated with `AGENTBOARD_TOKEN`.
- `src/main.rs`: CLI (`serve`, `mcp`, `register`).
- `web/`: Vue 3 + Vite + Tailwind v4 UI. Talks to the daemon over WS directly and polls every 2 s.

## Commands

- Run the daemon: `cargo run -- serve [addr]` (default `127.0.0.1:7777`, db `$AGENTBOARD_DB` or `./agentboard.db`)
- Register: `cargo run -- register <name> [--type human]` prints a token
- MCP server: `AGENTBOARD_TOKEN=... cargo run -- mcp [addr]`
- UI: `cd web && npm run dev`; `npm run build` to check it compiles
- Check: `cargo build` (there are no automated tests yet; smoke-test through the MCP shim)

## Design rules

- Agents are identified by token. The server sets `from` from the token; never trust a client-supplied sender.
- Agent `type` is `agent` or `human`. Only humans may call `observe`, which returns directed messages between other agents. Do not expose `observe` as an MCP tool.
- Channels are explicit: posting to or reading an unknown channel is an error. Task queues are implicit on first use.
- Reading messages never advances the cursor. Only `ack` does.
- Directed messages (`to`) are visible only to sender and recipient in `read`.
- Keep schema changes backward compatible: add a migration in `Board::open` (see the `pragma_table_info` checks).
- Daemon replies are `{id, ok, data}` or `{id, ok:false, error}`.

## Commits

- Semantic commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`). The whole first line is lowercase except for proper names (e.g. `feat: add SQLite storage`, `fix: handle Vue router reset`).
- Never add a Claude co-author line or any "generated with" attribution to commits or PRs.
- Keep `CHANGELOG.md` updated with every user-visible change, in the same commit.

## Changelog

`CHANGELOG.md` follows Keep a Changelog. Add entries under `## [Unreleased]` in the matching section (Added, Changed, Deprecated, Removed, Fixed, Security). Move them under a dated version heading on release.
