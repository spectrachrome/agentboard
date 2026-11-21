use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_derive::Serialize;
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

fn db_err(e: rusqlite::Error) -> String {
    e.to_string()
}

#[derive(Clone, Debug, Serialize)]
pub struct Message {
    pub id: u64,
    pub channel: String,
    pub from: String,
    /// Set for directed messages: only `from` and `to` can see them.
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
pub struct Agent {
    pub name: String,
    /// What the agent does: "agent" (an LLM) or "human" (a person using the UI).
    #[serde(rename = "type")]
    pub kind: String,
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

pub struct Board {
    db: Connection,
}

const MSG_COLS: &str = "id, channel, sender, recipient, body, ts";
const TASK_COLS: &str =
    "id, queue, payload, status, created_by, claimed_by, lease_until, result, created_at";
const CHANNEL_SELECT: &str = "SELECT c.name, c.description, c.created_at, COALESCE(MAX(m.id), 0)
     FROM channels c LEFT JOIN messages m ON m.channel = c.name";

impl Board {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = Connection::open(path).map_err(db_err)?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS agents (
                 name       TEXT PRIMARY KEY,
                 token      TEXT NOT NULL UNIQUE,
                 type       TEXT NOT NULL DEFAULT 'agent',
                 created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                 last_seen  INTEGER NOT NULL DEFAULT (unixepoch())
             );
             CREATE TABLE IF NOT EXISTS channels (
                 name        TEXT PRIMARY KEY,
                 description TEXT NOT NULL DEFAULT '',
                 created_at  INTEGER NOT NULL DEFAULT (unixepoch())
             );
             CREATE TABLE IF NOT EXISTS messages (
                 id        INTEGER PRIMARY KEY AUTOINCREMENT,
                 channel   TEXT NOT NULL REFERENCES channels(name),
                 sender    TEXT NOT NULL,
                 recipient TEXT,
                 body      TEXT NOT NULL,
                 ts        INTEGER NOT NULL DEFAULT (unixepoch())
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
        // Databases created before agent types existed.
        let has_type: bool = db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM pragma_table_info('agents') WHERE name = 'type')",
                [],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        if !has_type {
            db.execute_batch("ALTER TABLE agents ADD COLUMN type TEXT NOT NULL DEFAULT 'agent';")
                .map_err(db_err)?;
        }
        // Databases created before directed messages existed.
        let has_recipient: bool = db
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM pragma_table_info('messages') WHERE name = 'recipient')",
                [],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        if !has_recipient {
            db.execute_batch("ALTER TABLE messages ADD COLUMN recipient TEXT;")
                .map_err(db_err)?;
        }
        // Databases from the implicit-channel version: register their channels.
        db.execute_batch("INSERT OR IGNORE INTO channels (name) SELECT DISTINCT channel FROM messages;")
            .map_err(db_err)?;
        Ok(Self { db })
    }

    // ---- agents -------------------------------------------------------

    /// Returns the new agent's token. A name can be registered only once.
    pub fn register(&self, name: &str, kind: &str) -> Result<String> {
        validate_name("agent", name)?;
        if kind != "agent" && kind != "human" {
            return Err("agent type must be 'agent' or 'human'".into());
        }
        let token: Option<String> = self
            .db
            .query_row(
                "INSERT OR IGNORE INTO agents (name, token, type) VALUES (?1, lower(hex(randomblob(16))), ?2)
                 RETURNING token",
                params![name, kind],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        token.ok_or_else(|| format!("agent name '{name}' is already registered"))
    }

    /// Resolves a token to (name, type) and refreshes `last_seen`.
    pub fn authenticate(&self, token: &str) -> Result<(String, String)> {
        self.db
            .query_row(
                "UPDATE agents SET last_seen = unixepoch() WHERE token = ?1 RETURNING name, type",
                params![token],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_err)?
            .ok_or_else(|| "invalid token".to_string())
    }

    pub fn agents(&self) -> Result<Vec<Agent>> {
        let mut stmt = self
            .db
            .prepare("SELECT name, type, created_at, last_seen FROM agents ORDER BY name")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Agent {
                    name: r.get(0)?,
                    kind: r.get(1)?,
                    created_at: r.get::<_, i64>(2)? as u64,
                    last_seen: r.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    fn require_agent(&self, name: &str) -> Result<()> {
        let found: bool = self
            .db
            .query_row("SELECT EXISTS (SELECT 1 FROM agents WHERE name = ?1)", params![name], |r| r.get(0))
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

    pub fn post(&self, from: &str, channel: &str, to: Option<&str>, body: &str) -> Result<Message> {
        self.require_channel(channel)?;
        if let Some(to) = to {
            self.require_agent(to)?;
        }
        self.db
            .query_row(
                &format!(
                    "INSERT INTO messages (channel, sender, recipient, body) VALUES (?1, ?2, ?3, ?4)
                     RETURNING {MSG_COLS}"
                ),
                params![channel, from, to, body],
                row_to_message,
            )
            .map_err(db_err)
    }

    /// With `since = None`: the agent's unread messages (after its acked cursor,
    /// excluding its own posts). With `Some(id)`: everything it may see after `id`.
    /// Directed messages are visible only to sender and recipient.
    pub fn read(&self, agent: &str, channel: &str, since: Option<u64>, limit: u32) -> Result<Vec<Message>> {
        self.require_channel(channel)?;
        let (after, skip_own) = match since {
            Some(s) => (s as i64, 0),
            None => (self.cursor(agent, channel)?, 1),
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
            .query_map(params![channel, after, agent, skip_own, limit], row_to_message)
            .map_err(db_err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_err)
    }

    /// Every message in the channel after `since`, directed ones included.
    /// For supervising UIs; the daemon restricts it to human-type agents.
    pub fn observe(&self, channel: &str, since: u64, limit: u32) -> Result<Vec<Message>> {
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
    pub fn ack(&self, agent: &str, channel: &str, id: u64) -> Result<u64> {
        self.require_channel(channel)?;
        self.db
            .query_row(
                "INSERT INTO cursors (agent, channel, acked) VALUES (?1, ?2, ?3)
                 ON CONFLICT (agent, channel) DO UPDATE SET acked = MAX(acked, excluded.acked)
                 RETURNING acked",
                params![agent, channel, id as i64],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(db_err)
    }

    // ---- tasks --------------------------------------------------------

    pub fn enqueue_task(&self, by: &str, queue: &str, payload: &str) -> Result<Task> {
        validate_name("queue", queue)?;
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
    pub fn claim_task(&self, agent: &str, queue: &str, lease_s: u64) -> Result<Option<Task>> {
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
                params![queue, agent, lease_s as i64],
                row_to_task,
            )
            .optional()
            .map_err(db_err)
    }

    pub fn complete_task(&self, agent: &str, id: u64, result: &str) -> Result<Task> {
        self.db
            .query_row(
                &format!(
                    "UPDATE tasks SET status = 'done', result = ?3
                     WHERE id = ?1 AND claimed_by = ?2 AND status = 'claimed'
                     RETURNING {TASK_COLS}"
                ),
                params![id as i64, agent, result],
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
        to: r.get(3)?,
        body: r.get(4)?,
        ts: r.get::<_, i64>(5)? as u64,
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
