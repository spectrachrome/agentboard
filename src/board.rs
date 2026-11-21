//! All storage and rules. No networking.
//!
//! Humans and agents are separate kinds of principal with separate credential
//! tables. The separation is enforced three ways:
//!
//! * Types: `AgentId` and `HumanId` can only be produced by this module, from a
//!   valid token of the matching kind. Human-only operations (`observe`,
//!   `post_as_human`) take a `&HumanId`, so code holding only an agent identity
//!   cannot call them.
//! * Capability: `Board`, used by the daemon, has no way to create a human.
//!   Humans are added only through `Admin`, which needs direct access to the
//!   database file (the `agentboard admin` CLI).
//! * Database: triggers keep names unique across both kinds, and each message
//!   records the kind of its sender at insert time; messages are immutable.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_derive::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;

pub type Result<T> = std::result::Result<T, String>;

const MAX_BODY: usize = 64 * 1024;

fn db_err(e: rusqlite::Error) -> String {
    e.to_string()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Agent,
    Human,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Agent => "agent",
            Kind::Human => "human",
        }
    }
    fn parse(s: &str) -> Kind {
        if s == "human" { Kind::Human } else { Kind::Agent }
    }
}

/// An authenticated agent. Cannot be constructed outside this module.
#[derive(Clone, Debug)]
pub struct AgentId(String);

/// An authenticated human. Cannot be constructed outside this module.
#[derive(Clone, Debug)]
pub struct HumanId(String);

impl AgentId {
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl HumanId {
    pub fn name(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: u64,
    pub channel: String,
    pub from: String,
    /// Kind of the sender, stamped by the server from the connection it arrived on.
    pub from_type: Kind,
    /// Set for directed messages: only `from` and `to` can see them (humans see all).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub body: String,
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Channel {
    pub name: String,
    pub description: String,
    pub created_at: u64,
    /// Highest message id in the channel, 0 if empty.
    pub last_id: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Principal {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub created_at: u64,
    pub last_seen: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Task {
    pub id: u64,
    pub queue: String,
    pub payload: String,
    /// pending | claimed | done
    pub status: String,
    pub created_by: String,
    pub claimed_by: Option<String>,
    pub lease_until: Option<u64>,
    pub result: Option<String>,
    pub created_at: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueueStat {
    pub queue: String,
    pub pending: u64,
    pub claimed: u64,
    pub done: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Watch {
    /// Messages for the agent with id > `after`, from other senders.
    pub messages: Vec<Message>,
    /// Pass as `after` next time.
    pub latest: u64,
    /// Total unread (un-acked) messages for the agent across all channels.
    pub unread: u64,
}

const MSG_COLS: &str = "id, channel, sender, sender_kind, recipient, body, ts";
const MSG_COLS_M: &str = "m.id, m.channel, m.sender, m.sender_kind, m.recipient, m.body, m.ts";
const TASK_COLS: &str =
    "id, queue, payload, status, created_by, claimed_by, lease_until, result, created_at";
const CHANNEL_SELECT: &str = "SELECT c.name, c.description, c.created_at, COALESCE(MAX(m.id), 0)
     FROM channels c LEFT JOIN messages m ON m.channel = c.name";

pub struct Board {
    db: Connection,
}

/// Database administration, available only to whoever can open the file.
/// The daemon never constructs this.
pub struct Admin {
    db: Connection,
}

fn open_db(path: &Path) -> Result<Connection> {
    let db = Connection::open(path).map_err(db_err)?;
    db.busy_timeout(Duration::from_secs(5)).map_err(db_err)?;
    init(&db)?;
    Ok(db)
}

fn has_col(db: &Connection, table: &str, col: &str) -> Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, col],
        |r| r.get(0),
    )
    .map_err(db_err)
}

fn init(db: &Connection) -> Result<()> {
    db.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS agents (
             name       TEXT PRIMARY KEY,
             token_hash TEXT NOT NULL UNIQUE,
             created_at INTEGER NOT NULL DEFAULT (unixepoch()),
             last_seen  INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE TABLE IF NOT EXISTS humans (
             name       TEXT PRIMARY KEY,
             token_hash TEXT NOT NULL UNIQUE,
             created_at INTEGER NOT NULL DEFAULT (unixepoch()),
             last_seen  INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE TABLE IF NOT EXISTS channels (
             name        TEXT PRIMARY KEY,
             description TEXT NOT NULL DEFAULT '',
             created_at  INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE TABLE IF NOT EXISTS messages (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             channel     TEXT NOT NULL REFERENCES channels(name),
             sender      TEXT NOT NULL,
             sender_kind TEXT NOT NULL DEFAULT 'agent' CHECK (sender_kind IN ('agent', 'human')),
             recipient   TEXT,
             body        TEXT NOT NULL,
             ts          INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE INDEX IF NOT EXISTS messages_channel_id ON messages (channel, id);
         CREATE TABLE IF NOT EXISTS cursors (
             agent   TEXT NOT NULL,
             channel TEXT NOT NULL,
             acked   INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (agent, channel)
         );
         CREATE TABLE IF NOT EXISTS tasks (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             queue       TEXT NOT NULL,
             payload     TEXT NOT NULL,
             status      TEXT NOT NULL DEFAULT 'pending',
             created_by  TEXT NOT NULL,
             claimed_by  TEXT,
             lease_until INTEGER,
             result      TEXT,
             created_at  INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE INDEX IF NOT EXISTS tasks_queue_status ON tasks (queue, status, id);",
    )
    .map_err(db_err)?;

    // Migrations from earlier layouts. Each step checks before acting.
    if has_col(db, "agents", "token")? {
        // Tokens used to be stored in plain text; store only their hash.
        db.execute_batch("ALTER TABLE agents RENAME COLUMN token TO token_hash;").map_err(db_err)?;
        let rows: Vec<(String, String)> = {
            let mut stmt = db.prepare("SELECT name, token_hash FROM agents").map_err(db_err)?;
            let it = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(db_err)?;
            it.collect::<rusqlite::Result<_>>().map_err(db_err)?
        };
        for (name, token) in rows {
            db.execute("UPDATE agents SET token_hash = ?1 WHERE name = ?2", params![hash_token(&token), name])
                .map_err(db_err)?;
        }
    }
    if has_col(db, "agents", "type")? {
        // Humans used to self-register over the open port, so none of them can be trusted.
        db.execute_batch("DELETE FROM agents WHERE type = 'human'; ALTER TABLE agents DROP COLUMN type;")
            .map_err(db_err)?;
    }
    if !has_col(db, "messages", "recipient")? {
        db.execute_batch("ALTER TABLE messages ADD COLUMN recipient TEXT;").map_err(db_err)?;
    }
    if !has_col(db, "messages", "sender_kind")? {
        db.execute_batch(
            "ALTER TABLE messages ADD COLUMN sender_kind TEXT NOT NULL DEFAULT 'agent'
                 CHECK (sender_kind IN ('agent', 'human'));",
        )
        .map_err(db_err)?;
    }
    // Databases from the implicit-channel version: register their channels.
    db.execute_batch("INSERT OR IGNORE INTO channels (name) SELECT DISTINCT channel FROM messages;")
        .map_err(db_err)?;

    db.execute_batch(
        "-- A name is either an agent or a human, never both, and a name that has
         -- ever spoken as a human can never be registered as an agent.
         CREATE TRIGGER IF NOT EXISTS agents_name_free BEFORE INSERT ON agents
         WHEN EXISTS (SELECT 1 FROM humans WHERE name = NEW.name)
           OR EXISTS (SELECT 1 FROM messages WHERE sender = NEW.name AND sender_kind = 'human')
         BEGIN SELECT RAISE(ABORT, 'name is reserved'); END;
         CREATE TRIGGER IF NOT EXISTS humans_name_free BEFORE INSERT ON humans
         WHEN EXISTS (SELECT 1 FROM agents WHERE name = NEW.name)
         BEGIN SELECT RAISE(ABORT, 'name is taken by an agent'); END;
         CREATE TRIGGER IF NOT EXISTS messages_immutable BEFORE UPDATE ON messages
         BEGIN SELECT RAISE(ABORT, 'messages are immutable'); END;",
    )
    .map_err(db_err)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn new_token(prefix: &str, bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
    Ok(format!("{prefix}{}", hex(&buf)))
}

fn validate_name(kind: &str, name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!("{kind} name must be 1-64 chars of a-z, 0-9, '-', '_'"))
    }
}

fn is_constraint(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation)
}

impl Admin {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self { db: open_db(path.as_ref())? })
    }

    /// Creates a human and returns their token. The token is shown once and
    /// only its hash is stored.
    pub fn add_human(&self, name: &str) -> Result<String> {
        validate_name("human", name)?;
        let token = new_token("hum_", 32)?;
        match self.db.execute(
            "INSERT INTO humans (name, token_hash) VALUES (?1, ?2)",
            params![name, hash_token(&token)],
        ) {
            Ok(_) => Ok(token),
            Err(e) if is_constraint(&e) => Err(format!("name '{name}' is already taken")),
            Err(e) => Err(db_err(e)),
        }
    }

    pub fn remove_human(&self, name: &str) -> Result<bool> {
        let n = self
            .db
            .execute("DELETE FROM humans WHERE name = ?1", params![name])
            .map_err(db_err)?;
        Ok(n > 0)
    }

    pub fn humans(&self) -> Result<Vec<(String, u64)>> {
        let mut stmt = self
            .db
            .prepare("SELECT name, last_seen FROM humans ORDER BY name")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }
}

impl Board {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self { db: open_db(path.as_ref())? })
    }

    // ---- identity -----------------------------------------------------

    /// Agents may register themselves. Returns the token (shown once; only its hash is stored).
    pub fn register_agent(&self, name: &str) -> Result<String> {
        validate_name("agent", name)?;
        let token = new_token("agt_", 16)?;
        match self.db.execute(
            "INSERT INTO agents (name, token_hash) VALUES (?1, ?2)",
            params![name, hash_token(&token)],
        ) {
            Ok(_) => Ok(token),
            Err(e) if is_constraint(&e) => Err(format!("name '{name}' is not available")),
            Err(e) => Err(db_err(e)),
        }
    }

    pub fn authenticate_agent(&self, token: &str) -> Result<AgentId> {
        self.db
            .query_row(
                "UPDATE agents SET last_seen = unixepoch() WHERE token_hash = ?1 RETURNING name",
                params![hash_token(token)],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?
            .map(AgentId)
            .ok_or_else(|| "invalid token".to_string())
    }

    pub fn authenticate_human(&self, token: &str) -> Result<HumanId> {
        self.db
            .query_row(
                "UPDATE humans SET last_seen = unixepoch() WHERE token_hash = ?1 RETURNING name",
                params![hash_token(token)],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?
            .map(HumanId)
            .ok_or_else(|| "invalid token".to_string())
    }

    /// Everyone on the board, agents and humans.
    pub fn principals(&self) -> Result<Vec<Principal>> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT name, 'agent', created_at, last_seen FROM agents
                 UNION ALL
                 SELECT name, 'human', created_at, last_seen FROM humans
                 ORDER BY 1",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Principal {
                    name: r.get(0)?,
                    kind: Kind::parse(&r.get::<_, String>(1)?),
                    created_at: r.get::<_, i64>(2)? as u64,
                    last_seen: r.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    fn require_principal(&self, name: &str) -> Result<()> {
        let found: bool = self
            .db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM agents WHERE name = ?1)
                     OR EXISTS (SELECT 1 FROM humans WHERE name = ?1)",
                params![name],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        if found { Ok(()) } else { Err(format!("unknown agent '{name}'")) }
    }

    // ---- channels -----------------------------------------------------

    /// Idempotent: creating an existing channel returns it unchanged.
    pub fn create_channel(&self, name: &str, description: &str) -> Result<Channel> {
        validate_name("channel", name)?;
        self.db
            .execute(
                "INSERT OR IGNORE INTO channels (name, description) VALUES (?1, ?2)",
                params![name, description],
            )
            .map_err(db_err)?;
        self.channel(name)?.ok_or_else(|| "channel vanished".into())
    }

    pub fn channels(&self) -> Result<Vec<Channel>> {
        let mut stmt = self
            .db
            .prepare(&format!("{CHANNEL_SELECT} GROUP BY c.name ORDER BY c.name"))
            .map_err(db_err)?;
        let rows = stmt.query_map([], row_to_channel).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    fn channel(&self, name: &str) -> Result<Option<Channel>> {
        self.db
            .query_row(
                &format!("{CHANNEL_SELECT} WHERE c.name = ?1 GROUP BY c.name"),
                params![name],
                row_to_channel,
            )
            .optional()
            .map_err(db_err)
    }

    fn require_channel(&self, name: &str) -> Result<()> {
        match self.channel(name)? {
            Some(_) => Ok(()),
            None => Err(format!("unknown channel '{name}'; create it with create_channel")),
        }
    }

    // ---- messages -----------------------------------------------------

    pub fn post_as_agent(&self, who: &AgentId, channel: &str, to: Option<&str>, body: &str) -> Result<Message> {
        self.post_inner(&who.0, Kind::Agent, channel, to, body)
    }

    pub fn post_as_human(&self, who: &HumanId, channel: &str, to: Option<&str>, body: &str) -> Result<Message> {
        self.post_inner(&who.0, Kind::Human, channel, to, body)
    }

    fn post_inner(&self, from: &str, kind: Kind, channel: &str, to: Option<&str>, body: &str) -> Result<Message> {
        if body.len() > MAX_BODY {
            return Err(format!("message too long (max {MAX_BODY} bytes)"));
        }
        self.require_channel(channel)?;
        if let Some(to) = to {
            self.require_principal(to)?;
        }
        self.db
            .query_row(
                &format!(
                    "INSERT INTO messages (channel, sender, sender_kind, recipient, body)
                     VALUES (?1, ?2, ?3, ?4, ?5) RETURNING {MSG_COLS}"
                ),
                params![channel, from, kind.as_str(), to, body],
                row_to_message,
            )
            .map_err(db_err)
    }

    /// Agent reads.
    ///
    /// * `channel` + `since = None`: unread in that channel (after the acked cursor,
    ///   excluding the agent's own posts).
    /// * `channel` + `since = Some(id)`: everything visible to the agent after `id`.
    /// * no channel: unread across all channels.
    ///
    /// Directed messages are visible only to sender and recipient.
    pub fn read(&self, who: &AgentId, channel: Option<&str>, since: Option<u64>, limit: u32) -> Result<Vec<Message>> {
        let me = who.name();
        let Some(channel) = channel else {
            return match since {
                None => self.inbox(me, limit),
                Some(_) => Err("`since` needs a channel".into()),
            };
        };
        self.require_channel(channel)?;
        let (after, skip_own) = match since {
            Some(s) => (s as i64, 0),
            None => (self.cursor(me, channel)?, 1),
        };
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {MSG_COLS} FROM messages
                 WHERE channel = ?1 AND id > ?2
                   AND (recipient IS NULL OR recipient = ?3 OR sender = ?3)
                   AND (?4 = 0 OR sender != ?3)
                 ORDER BY id LIMIT ?5"
            ))
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![channel, after, me, skip_own, limit], row_to_message)
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    fn inbox(&self, me: &str, limit: u32) -> Result<Vec<Message>> {
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {MSG_COLS_M} FROM messages m
                 LEFT JOIN cursors c ON c.agent = ?1 AND c.channel = m.channel
                 WHERE m.id > COALESCE(c.acked, 0) AND m.sender != ?1
                   AND (m.recipient IS NULL OR m.recipient = ?1)
                 ORDER BY m.id LIMIT ?2"
            ))
            .map_err(db_err)?;
        let rows = stmt.query_map(params![me, limit], row_to_message).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    fn unread_count(&self, me: &str) -> Result<u64> {
        self.db
            .query_row(
                "SELECT COUNT(*) FROM messages m
                 LEFT JOIN cursors c ON c.agent = ?1 AND c.channel = m.channel
                 WHERE m.id > COALESCE(c.acked, 0) AND m.sender != ?1
                   AND (m.recipient IS NULL OR m.recipient = ?1)",
                params![me],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as u64)
            .map_err(db_err)
    }

    /// Cursor-free scan used by integrations that decide for themselves what to
    /// do with new messages. `after = None` only reports the current position.
    pub fn watch(&self, who: &AgentId, after: Option<u64>, limit: u32) -> Result<Watch> {
        let me = who.name();
        let global_max: i64 = self
            .db
            .query_row("SELECT COALESCE(MAX(id), 0) FROM messages", [], |r| r.get(0))
            .map_err(db_err)?;
        let unread = self.unread_count(me)?;
        let Some(after) = after else {
            return Ok(Watch { messages: vec![], latest: global_max as u64, unread });
        };
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {MSG_COLS} FROM messages
                 WHERE id > ?1 AND sender != ?2 AND (recipient IS NULL OR recipient = ?2)
                 ORDER BY id LIMIT ?3"
            ))
            .map_err(db_err)?;
        let messages: Vec<Message> = stmt
            .query_map(params![after as i64, me, limit], row_to_message)
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        // A full page means there may be more; otherwise everything up to the
        // global max has been examined (the board lock is held for this scan).
        let latest = if messages.len() as u32 >= limit {
            messages.last().map(|m| m.id).unwrap_or(after)
        } else {
            (global_max as u64).max(after)
        };
        Ok(Watch { messages, latest, unread })
    }

    /// Every message in the channel after `since`, directed ones included.
    /// Requires a `HumanId`, which agents cannot obtain.
    pub fn observe(&self, _by: &HumanId, channel: &str, since: u64, limit: u32) -> Result<Vec<Message>> {
        self.require_channel(channel)?;
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {MSG_COLS} FROM messages WHERE channel = ?1 AND id > ?2 ORDER BY id LIMIT ?3"
            ))
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![channel, since as i64, limit], row_to_message)
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    fn cursor(&self, agent: &str, channel: &str) -> Result<i64> {
        Ok(self
            .db
            .query_row(
                "SELECT acked FROM cursors WHERE agent = ?1 AND channel = ?2",
                params![agent, channel],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?
            .unwrap_or(0))
    }

    /// Advances the agent's cursor (never backwards). Returns the new cursor.
    pub fn ack(&self, who: &AgentId, channel: &str, id: u64) -> Result<u64> {
        self.require_channel(channel)?;
        self.db
            .query_row(
                "INSERT INTO cursors (agent, channel, acked) VALUES (?1, ?2, ?3)
                 ON CONFLICT (agent, channel) DO UPDATE SET acked = MAX(acked, excluded.acked)
                 RETURNING acked",
                params![who.name(), channel, id as i64],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(db_err)
    }

    // ---- tasks --------------------------------------------------------

    /// `by` comes from an authenticated identity (agent or human).
    pub fn enqueue_task(&self, by: &str, queue: &str, payload: &str) -> Result<Task> {
        validate_name("queue", queue)?;
        if payload.len() > MAX_BODY {
            return Err(format!("payload too long (max {MAX_BODY} bytes)"));
        }
        self.db
            .query_row(
                &format!(
                    "INSERT INTO tasks (queue, payload, created_by) VALUES (?1, ?2, ?3)
                     RETURNING {TASK_COLS}"
                ),
                params![queue, payload, by],
                row_to_task,
            )
            .map_err(db_err)
    }

    /// Claims the oldest pending task, or one whose lease has expired.
    pub fn claim_task(&self, who: &AgentId, queue: &str, lease_s: u64) -> Result<Option<Task>> {
        self.db
            .query_row(
                &format!(
                    "UPDATE tasks SET status = 'claimed', claimed_by = ?2,
                                      lease_until = unixepoch() + ?3
                     WHERE id = (SELECT id FROM tasks
                                 WHERE queue = ?1
                                   AND (status = 'pending'
                                        OR (status = 'claimed' AND lease_until <= unixepoch()))
                                 ORDER BY id LIMIT 1)
                     RETURNING {TASK_COLS}"
                ),
                params![queue, who.name(), lease_s as i64],
                row_to_task,
            )
            .optional()
            .map_err(db_err)
    }

    pub fn complete_task(&self, who: &AgentId, id: u64, result: &str) -> Result<Task> {
        self.db
            .query_row(
                &format!(
                    "UPDATE tasks SET status = 'done', result = ?3
                     WHERE id = ?1 AND claimed_by = ?2 AND status = 'claimed'
                     RETURNING {TASK_COLS}"
                ),
                params![id as i64, who.name(), result],
                row_to_task,
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| format!("task {id} is not claimed by you (unknown, done, or lease lost)"))
    }

    pub fn queues(&self) -> Result<Vec<QueueStat>> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT queue,
                        SUM(status = 'pending'), SUM(status = 'claimed'), SUM(status = 'done')
                 FROM tasks GROUP BY queue ORDER BY queue",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(QueueStat {
                    queue: r.get(0)?,
                    pending: r.get::<_, i64>(1)? as u64,
                    claimed: r.get::<_, i64>(2)? as u64,
                    done: r.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    pub fn list_tasks(&self, queue: &str, status: Option<&str>, limit: u32) -> Result<Vec<Task>> {
        let mut stmt = self
            .db
            .prepare(&format!(
                "SELECT {TASK_COLS} FROM tasks
                 WHERE queue = ?1 AND (?2 IS NULL OR status = ?2)
                 ORDER BY id LIMIT ?3"
            ))
            .map_err(db_err)?;
        let rows = stmt.query_map(params![queue, status, limit], row_to_task).map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }
}

fn row_to_channel(r: &Row) -> rusqlite::Result<Channel> {
    Ok(Channel {
        name: r.get(0)?,
        description: r.get(1)?,
        created_at: r.get::<_, i64>(2)? as u64,
        last_id: r.get::<_, i64>(3)? as u64,
    })
}

fn row_to_message(r: &Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get::<_, i64>(0)? as u64,
        channel: r.get(1)?,
        from: r.get(2)?,
        from_type: Kind::parse(&r.get::<_, String>(3)?),
        to: r.get(4)?,
        body: r.get(5)?,
        ts: r.get::<_, i64>(6)? as u64,
    })
}

fn row_to_task(r: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get::<_, i64>(0)? as u64,
        queue: r.get(1)?,
        payload: r.get(2)?,
        status: r.get(3)?,
        created_by: r.get(4)?,
        claimed_by: r.get(5)?,
        lease_until: r.get::<_, Option<i64>>(6)?.map(|v| v as u64),
        result: r.get(7)?,
        created_at: r.get::<_, i64>(8)? as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDb(std::path::PathBuf);
    impl TempDb {
        fn new(tag: &str) -> Self {
            let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            Self(std::env::temp_dir().join(format!("agentboard-test-{tag}-{}-{n}.db", std::process::id())))
        }
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
            }
        }
    }

    fn setup(tag: &str) -> (TempDb, Board, Admin) {
        let t = TempDb::new(tag);
        let admin = Admin::open(&t.0).unwrap();
        let board = Board::open(&t.0).unwrap();
        board.create_channel("dev", "").unwrap();
        (t, board, admin)
    }

    #[test]
    fn tokens_are_kind_specific() {
        let (_t, board, admin) = setup("kind");
        let agent_tok = board.register_agent("claude").unwrap();
        let human_tok = admin.add_human("moritz").unwrap();
        assert!(board.authenticate_agent(&agent_tok).is_ok());
        assert!(board.authenticate_human(&human_tok).is_ok());
        assert!(board.authenticate_human(&agent_tok).is_err());
        assert!(board.authenticate_agent(&human_tok).is_err());
    }

    #[test]
    fn names_cannot_cross_kinds() {
        let (_t, board, admin) = setup("names");
        admin.add_human("moritz").unwrap();
        assert!(board.register_agent("moritz").is_err());
        board.register_agent("claude").unwrap();
        assert!(admin.add_human("claude").is_err());
    }

    #[test]
    fn removed_human_name_stays_reserved_once_they_have_posted() {
        let (_t, board, admin) = setup("reserved");
        let tok = admin.add_human("moritz").unwrap();
        let me = board.authenticate_human(&tok).unwrap();
        board.post_as_human(&me, "dev", None, "hi").unwrap();
        admin.remove_human("moritz").unwrap();
        assert!(board.register_agent("moritz").is_err());
    }

    #[test]
    fn sender_kind_is_stamped_by_the_server() {
        let (_t, board, admin) = setup("stamp");
        let a = board.authenticate_agent(&board.register_agent("claude").unwrap()).unwrap();
        let h = board.authenticate_human(&admin.add_human("moritz").unwrap()).unwrap();
        let m1 = board.post_as_agent(&a, "dev", None, "x").unwrap();
        let m2 = board.post_as_human(&h, "dev", None, "y").unwrap();
        assert_eq!(m1.from_type, Kind::Agent);
        assert_eq!(m2.from_type, Kind::Human);
    }

    #[test]
    fn messages_are_immutable() {
        let (_t, board, _admin) = setup("immutable");
        let a = board.authenticate_agent(&board.register_agent("claude").unwrap()).unwrap();
        board.post_as_agent(&a, "dev", None, "x").unwrap();
        assert!(board.db.execute("UPDATE messages SET sender_kind = 'human'", []).is_err());
    }

    #[test]
    fn directed_messages_are_private_except_to_observers() {
        let (_t, board, admin) = setup("dm");
        let a = board.authenticate_agent(&board.register_agent("claude").unwrap()).unwrap();
        let b = board.authenticate_agent(&board.register_agent("codex").unwrap()).unwrap();
        let c = board.authenticate_agent(&board.register_agent("third").unwrap()).unwrap();
        let h = board.authenticate_human(&admin.add_human("moritz").unwrap()).unwrap();
        board.post_as_agent(&a, "dev", Some("codex"), "secret").unwrap();
        assert_eq!(board.read(&b, Some("dev"), None, 50).unwrap().len(), 1);
        assert_eq!(board.read(&c, Some("dev"), None, 50).unwrap().len(), 0);
        assert_eq!(board.read(&c, Some("dev"), Some(0), 50).unwrap().len(), 0);
        assert_eq!(board.observe(&h, "dev", 0, 50).unwrap().len(), 1);
    }

    #[test]
    fn inbox_and_ack() {
        let (_t, board, _admin) = setup("ack");
        board.create_channel("ops", "").unwrap();
        let a = board.authenticate_agent(&board.register_agent("claude").unwrap()).unwrap();
        let b = board.authenticate_agent(&board.register_agent("codex").unwrap()).unwrap();
        board.post_as_agent(&a, "dev", None, "1").unwrap();
        let m = board.post_as_agent(&a, "ops", None, "2").unwrap();
        assert_eq!(board.read(&b, None, None, 50).unwrap().len(), 2);
        assert_eq!(board.watch(&b, None, 50).unwrap().unread, 2);
        board.ack(&b, "ops", m.id).unwrap();
        assert_eq!(board.read(&b, None, None, 50).unwrap().len(), 1);
        assert_eq!(board.read(&a, None, None, 50).unwrap().len(), 0); // own posts excluded
    }

    #[test]
    fn watch_pages_and_does_not_lose_messages() {
        let (_t, board, _admin) = setup("watch");
        let a = board.authenticate_agent(&board.register_agent("claude").unwrap()).unwrap();
        let b = board.authenticate_agent(&board.register_agent("codex").unwrap()).unwrap();
        let start = board.watch(&b, None, 2).unwrap().latest;
        for i in 0..5 {
            board.post_as_agent(&a, "dev", None, &i.to_string()).unwrap();
        }
        let mut after = start;
        let mut seen = vec![];
        loop {
            let w = board.watch(&b, Some(after), 2).unwrap();
            if w.messages.is_empty() {
                break;
            }
            seen.extend(w.messages.iter().map(|m| m.body.clone()));
            after = w.latest;
        }
        assert_eq!(seen, ["0", "1", "2", "3", "4"]);
    }

    #[test]
    fn old_plaintext_tokens_are_migrated_to_hashes() {
        let t = TempDb::new("migrate");
        {
            let db = Connection::open(&t.0).unwrap();
            db.execute_batch(
                "CREATE TABLE agents (name TEXT PRIMARY KEY, token TEXT NOT NULL UNIQUE,
                     type TEXT NOT NULL DEFAULT 'agent',
                     created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     last_seen INTEGER NOT NULL DEFAULT (unixepoch()));
                 INSERT INTO agents (name, token, type) VALUES ('old', 'oldtoken', 'agent');
                 INSERT INTO agents (name, token, type) VALUES ('fake', 'faketoken', 'human');",
            )
            .unwrap();
        }
        let board = Board::open(&t.0).unwrap();
        assert!(board.authenticate_agent("oldtoken").is_ok());
        assert!(board.authenticate_agent("faketoken").is_err());
        assert!(board.authenticate_human("faketoken").is_err());
    }
}
