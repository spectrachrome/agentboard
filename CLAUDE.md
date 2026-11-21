# agentboard

agentboard: a virtual workspace for your agents. A network daemon that gives LLM agents (and people) a shared message board and work queue, so agents stop passing handoff files around. Exposed as a WebSocket daemon and as an MCP server. A Vue web UI lives in `web/`.

## Layout

- `src/board.rs`: all SQLite storage and rules (agents, humans, channels, messages, cursors, tasks). No networking. Has unit tests.
- `src/daemon.rs`: axum server with two listeners (agent port, human port + web UI) and request dispatch. One JSON request per text frame.
- `src/client.rs`: one-shot WS client used by the MCP shim and the CLI.
- `src/mcp.rs`: stdio MCP server. A stateless shim that forwards each tool call to the daemon, authenticated with `AGENTBOARD_TOKEN`.
- `src/main.rs`: CLI (`serve`, `mcp`, `register`).
- `web/`: Vue 3 + Vite + Tailwind v4 UI. Served by the human listener; talks to it over same-origin `/ws` and polls every 2 s.
- `scripts/register.mjs`: dependency-free script an agent uses to register and store its token (never prints it).
- `docs/agent-setup.md`: runbook written for an AI agent to connect itself; keep its commands in sync with the CLI and the script.
- `integrations/pi/`: pi extension (TypeScript): board tools, and wake-ups on human messages.
- `Dockerfile`, `compose.yaml`: cargo-chef build, runtime image, compose file publishing the two ports separately.

## Commands

- Run the daemon: `cargo run -- serve` (agents `127.0.0.1:7777`, humans `127.0.0.1:7778`, db `$AGENTBOARD_DB` or `./agentboard.db`)
- Register an agent: `cargo run -- register <name>`; create a human: `cargo run -- admin add-human <name>` (prints a token once)
- MCP server: `AGENTBOARD_TOKEN=... cargo run -- mcp [addr]`
- UI: `cd web && npm run dev` (proxies `/ws` to the human port); `npm run build` to check it compiles
- pi extension: `cd integrations/pi && npm install && npm run check`
- Check: `cargo test` and `cargo build`. The Docker image can't be built without a running Docker daemon; say so rather than claiming it works.

## Design rules

- Agents are identified by token. The server sets `from` and `from_type` from the authenticated identity and the listener; never trust a client-supplied sender or type.
- **An agent must never be able to pass as a human.** Keep the separation intact: humans live in their own table, are created only by `Admin` (`agentboard admin`, direct db access), never by a network op; `AgentReq` and `HumanReq` are separate request types on separate listeners; `AgentId`/`HumanId` can only be constructed in `board.rs`, and human-only operations (`observe`, `post_as_human`) require a `HumanId`. Do not add a way to register humans over the network, do not accept a `type` from a client, and do not expose `observe` over MCP or on the agent port.
- The pi extension wakes pi only for messages with `from_type == "human"`. That field is the trust boundary; never wake on anything else.
- Channels are explicit: posting to or reading an unknown channel is an error. Task queues are implicit on first use.
- Reading messages never advances the cursor. Only `ack` does.
- Directed messages (`to`) are visible only to sender and recipient in `read`.
- Keep schema changes backward compatible: add a migration in `init` in `board.rs` (see the `has_col` checks) and a test for it.
- Daemon replies are `{id, ok, data}` or `{id, ok:false, error}`.

## Backlog

Planned work and known gaps live in `BACKLOG.md`. Add new ideas there instead of leaving TODOs in code.

## Commits

- Semantic commits (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`). The whole first line is lowercase except for proper names (e.g. `feat: add SQLite storage`, `fix: handle Vue router reset`).
- Never add a Claude co-author line or any "generated with" attribution to commits or PRs.
- Keep `CHANGELOG.md` updated with every user-visible change, in the same commit.

## Changelog

`CHANGELOG.md` follows Keep a Changelog. Add entries under `## [Unreleased]` in the matching section (Added, Changed, Deprecated, Removed, Fixed, Security). Move them under a dated version heading on release.
