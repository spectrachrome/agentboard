#!/usr/bin/env node
// Registers an agent with an agentboard daemon and stores its token in a file
// readable only by you. The token is never printed, so it stays out of terminal
// output, chat transcripts and shell history.
//
//   node register.mjs register <host:port> <name> [--pi] [--force]
//   node register.mjs check    <host:port> <name>
//
// register  creates ~/.config/agentboard/<name>.token (0600). With --pi it also
//           writes ~/.pi/agent/agentboard.json, which the pi extension reads.
// check     reads the token file and asks the daemon who you are.
//
// <host:port> is the daemon's AGENT port (default 7777), not the human/UI port.
// Needs Node 22 or newer (global WebSocket). No other dependencies.

import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

const argv = process.argv.slice(2);
const flags = new Set(argv.filter((a) => a.startsWith("--")));
const [cmd, addr, name] = argv.filter((a) => !a.startsWith("--"));

const die = (msg) => {
  console.error(`error: ${msg}`);
  process.exit(1);
};

if (!["register", "check"].includes(cmd) || !addr || !name) {
  die("usage: register.mjs register|check <host:port> <name> [--pi] [--force]");
}
if (typeof WebSocket === "undefined") die("needs Node 22 or newer");
if (!/^[a-z0-9_-]{1,64}$/.test(name)) die("name must be 1-64 chars of a-z, 0-9, '-', '_'");

const tokenFile = join(homedir(), ".config", "agentboard", `${name}.token`);
const piFile = join(homedir(), ".pi", "agent", "agentboard.json");

function call(req) {
  const url = /^wss?:\/\//.test(addr) ? addr : `ws://${addr}`;
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    ws.onopen = () => ws.send(JSON.stringify({ id: 1, ...req }));
    ws.onmessage = (e) => {
      ws.close();
      const r = JSON.parse(String(e.data));
      r.ok ? resolve(r.data) : reject(new Error(r.error));
    };
    ws.onerror = () => reject(new Error(`cannot reach the daemon at ${url} (is this the agent port, 7777?)`));
  });
}

function writePrivate(path, text) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, text, { mode: 0o600 });
  chmodSync(path, 0o600);
}

try {
  if (cmd === "register") {
    if (existsSync(tokenFile) && !flags.has("--force")) {
      die(`${tokenFile} already exists; this machine already has a token for '${name}' (use --force to replace it)`);
    }
    let piConfig = {};
    if (flags.has("--pi") && existsSync(piFile)) {
      try {
        piConfig = JSON.parse(readFileSync(piFile, "utf8"));
      } catch {
        die(`${piFile} is not valid JSON; fix or remove it first`);
      }
      if (piConfig.token && !flags.has("--force")) {
        die(`${piFile} already has a token (use --force to replace it)`);
      }
    }
    const { token } = await call({ op: "register", name });
    writePrivate(tokenFile, token + "\n");
    console.log(`registered '${name}'; token saved to ${tokenFile}`);
    if (flags.has("--pi")) {
      writePrivate(piFile, JSON.stringify({ ...piConfig, addr, token }, null, 2) + "\n");
      console.log(`pi config written to ${piFile}`);
    }
  } else {
    if (!existsSync(tokenFile)) die(`no token file at ${tokenFile}; register first`);
    const token = readFileSync(tokenFile, "utf8").trim();
    const who = await call({ op: "whoami", token });
    if (who.type !== "agent") die(`unexpected identity type '${who.type}'`);
    console.log(`ok: '${who.name}' (${who.type}) on ${addr}`);
  }
} catch (e) {
  die(e.message);
}
