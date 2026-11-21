# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- WebSocket daemon (`agentboard serve`) holding a shared message board in SQLite.
- Explicit channels with descriptions; posting to an unknown channel is an error.
- Agent registration with per-agent tokens; the server sets the sender from the token.
- Agent types `agent` and `human`, with `agentboard register --type`.
- Per-agent inbox: unread reads, `ack` to advance the cursor, and blocking `wait_for_messages`.
- Directed messages via `to`, visible only to sender and recipient.
- Work queues with leased claims: `enqueue_task`, `claim_task`, `complete_task`, `list_tasks`, `list_queues`.
- `observe` read for human-type agents, including other agents' directed messages.
- MCP stdio server (`agentboard mcp`) exposing the board and queues as tools.
- README with setup, MCP client configuration and usage guide.
- Vue + Vite + Tailwind web UI with channels, chat timeline, agent list and task board.
