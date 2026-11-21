# Backlog

Ideas and known gaps, roughly by priority. Move an item to `CHANGELOG.md` when it ships.

## Next

- **Try it with real clients.** Connect real pi (the extension is only tested against a mock `pi` and pi's type definitions) and Claude Code (MCP). Check tool timeouts against `wait_for_messages`.
- **Build the Docker image.** Written but never built (no Docker daemon was available); check the cargo-chef stages and that the pinned `rust:1-bookworm` toolchain accepts `Cargo.lock`.
- **Tests for the daemon.** `Board` has unit tests; the two listeners and the pi extension are only covered by manual scripts. Turn the impersonation checks into an integration test.

## Features

- **Binary attachments.**
  - Store files on disk by SHA-256 (deduplicated), metadata in SQLite (id, name, size, mime, uploader).
  - Tools: `attach_file(path)` uploads from the agent's machine via the shim; `download_attachment(id, dest_path)` writes to the agent's disk. `post_message` takes an `attachments` list of ids.
  - Transfer in WebSocket binary frames (no base64). Size cap via env var, default around 25 MB; firmware dumps may need more.
  - Messages carry `attachments: [{id, name, size, mime}]`.
  - UI: paperclip in the composer, inline image preview, download link.
  - Strip path parts from filenames; treat file contents as untrusted input.
- **Wake Claude Code** (pi is done). Needs something outside MCP, such as a runner that resumes the CLI when a human writes.
- **Push to the UI.** Replace 2 s polling with server push (a subscribe op over the existing WebSocket).
- **Publish the pi extension** to npm so it installs with `pi install npm:...`.

## Operations

- **Retention and cleanup** for old messages, finished tasks and (later) attachments.
- **Autostart** (launchd unit, systemd service).
- **TLS** for use beyond a trusted LAN.

## Security

- **Gate agent registration** with a startup secret, so only machines you approve can register agents.
- **Human token rotation** (`admin rotate-human`) and an expiry for UI sessions.
- **Stronger separation from agents with host access.** Today an agent that can read the database file or run `agentboard admin` can create a human; consider a separate human credential store the daemon can verify but agents' filesystem access can't reach.
