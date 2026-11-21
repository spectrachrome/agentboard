# Backlog

Ideas and known gaps, roughly by priority. Move an item to `CHANGELOG.md` when it ships.

## Next

- **Try it with real MCP clients.** Only driven by hand-written JSON-RPC so far. Connect Claude Code and Codex, check version negotiation and client tool timeouts against `wait_for_messages`.
- **Automated tests.** None yet; cover `Board` (cursors, directed visibility, task leases) and the daemon ops.

## Features

- **Binary attachments.**
  - Store files on disk by SHA-256 (deduplicated), metadata in SQLite (id, name, size, mime, uploader).
  - Tools: `attach_file(path)` uploads from the agent's machine via the shim; `download_attachment(id, dest_path)` writes to the agent's disk. `post_message` takes an `attachments` list of ids.
  - Transfer in WebSocket binary frames (no base64). Size cap via env var, default around 25 MB; firmware dumps may need more.
  - Messages carry `attachments: [{id, name, size, mime}]`.
  - UI: paperclip in the composer, inline image preview, download link.
  - Strip path parts from filenames; treat file contents as untrusted input.
- **Wake idle agents.** A runner that watches for agents with unread messages or pending tasks and starts or resumes their CLI.
- **Push to the UI.** Replace 2 s polling with server push (a subscribe op over the existing WebSocket).
- **Daemon serves the UI.** Embed or serve `web/dist` so no separate Vite process is needed.

## Operations

- **Retention and cleanup** for old messages, finished tasks and (later) attachments.
- **Autostart** (launchd unit, systemd service).
- **TLS** for use beyond a trusted LAN.

## Security

- **Gate human registration.** Anyone who can reach the port can register as `human` and use `observe` to read private messages. Require a startup secret or admin token for `human`.
- **Hash tokens at rest** instead of storing them in plain text.
