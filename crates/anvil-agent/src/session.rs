use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub title: Option<String>,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Paused,
    Completed,
    Abandoned,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Abandoned => write!(f, "abandoned"),
        }
    }
}

pub struct SessionStore {
    db_path: std::path::PathBuf,
    conn: Connection,
}

impl Clone for SessionStore {
    fn clone(&self) -> Self {
        Self::open(&self.db_path).expect("failed to reopen session database")
    }
}

impl SessionStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let store = Self {
            db_path: db_path.to_path_buf(),
            conn,
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                title TEXT,
                status TEXT NOT NULL DEFAULT 'active'
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT,
                tool_calls TEXT,
                tool_call_id TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id)
            );

            CREATE TABLE IF NOT EXISTS tool_calls (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                arguments TEXT NOT NULL,
                result TEXT,
                duration_ms INTEGER,
                permission TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(id),
                FOREIGN KEY (message_id) REFERENCES messages(id)
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
            CREATE INDEX IF NOT EXISTS idx_tool_calls_session ON tool_calls(session_id);
            ",
        )?;
        Ok(())
    }

    pub fn create_session(&self) -> Result<Session> {
        let now = Utc::now();
        let session = Session {
            id: Uuid::new_v4().to_string(),
            created_at: now,
            updated_at: now,
            title: None,
            status: SessionStatus::Active,
        };

        self.conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at, status) VALUES (?1, ?2, ?3, ?4)",
            params![
                session.id,
                session.created_at.to_rfc3339(),
                session.updated_at.to_rfc3339(),
                session.status.to_string(),
            ],
        )?;

        Ok(session)
    }

    pub fn save_message(
        &self,
        session_id: &str,
        role: &str,
        content: Option<&str>,
        tool_calls_json: Option<&str>,
        tool_call_id: Option<&str>,
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO messages (id, session_id, role, content, tool_calls, tool_call_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, session_id, role, content, tool_calls_json, tool_call_id, now],
        )?;

        // Update session timestamp
        self.conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;

        Ok(id)
    }

    pub fn save_tool_call(&self, entry: &ToolCallEntry<'_>) -> Result<()> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO tool_calls (id, session_id, message_id, tool_name, arguments, result, duration_ms, permission, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                entry.session_id,
                entry.message_id,
                entry.tool_name,
                entry.arguments,
                entry.result,
                entry.duration_ms,
                entry.permission,
                now
            ],
        )?;

        Ok(())
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, updated_at, title, status FROM sessions ORDER BY updated_at DESC LIMIT ?1",
        )?;

        let sessions = stmt
            .query_map(params![limit], |row| {
                let status_str: String = row.get(4)?;
                let status = match status_str.as_str() {
                    "active" => SessionStatus::Active,
                    "paused" => SessionStatus::Paused,
                    "completed" => SessionStatus::Completed,
                    _ => SessionStatus::Abandoned,
                };
                Ok(Session {
                    id: row.get(0)?,
                    created_at: row
                        .get::<_, String>(1)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(2)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    title: row.get(3)?,
                    status,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(sessions)
    }

    pub fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, content, tool_calls, tool_call_id, created_at FROM messages WHERE session_id = ?1 ORDER BY created_at",
        )?;

        let messages = stmt
            .query_map(params![session_id], |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                    tool_calls_json: row.get(3)?,
                    tool_call_id: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(messages)
    }

    pub fn update_session_status(&self, session_id: &str, status: &SessionStatus) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sessions SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.to_string(), now, session_id],
        )?;
        Ok(())
    }

    pub fn find_latest_resumable(&self) -> Result<Option<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, updated_at, title, status FROM sessions \
             WHERE status IN ('active', 'paused') \
             ORDER BY updated_at DESC LIMIT 1",
        )?;

        let mut sessions = stmt
            .query_map([], |row| {
                let status_str: String = row.get(4)?;
                let status = match status_str.as_str() {
                    "active" => SessionStatus::Active,
                    "paused" => SessionStatus::Paused,
                    "completed" => SessionStatus::Completed,
                    _ => SessionStatus::Abandoned,
                };
                Ok(Session {
                    id: row.get(0)?,
                    created_at: row
                        .get::<_, String>(1)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(2)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    title: row.get(3)?,
                    status,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(sessions.pop())
    }

    pub fn find_by_prefix(&self, prefix: &str) -> Result<Option<Session>> {
        let pattern = format!("{prefix}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, updated_at, title, status FROM sessions \
             WHERE id LIKE ?1 ORDER BY updated_at DESC LIMIT 1",
        )?;

        let mut sessions = stmt
            .query_map(params![pattern], |row| {
                let status_str: String = row.get(4)?;
                let status = match status_str.as_str() {
                    "active" => SessionStatus::Active,
                    "paused" => SessionStatus::Paused,
                    "completed" => SessionStatus::Completed,
                    _ => SessionStatus::Abandoned,
                };
                Ok(Session {
                    id: row.get(0)?,
                    created_at: row
                        .get::<_, String>(1)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    updated_at: row
                        .get::<_, String>(2)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                    title: row.get(3)?,
                    status,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(sessions.pop())
    }
}

pub struct ToolCallEntry<'a> {
    pub session_id: &'a str,
    pub message_id: &'a str,
    pub tool_name: &'a str,
    pub arguments: &'a str,
    pub result: Option<&'a str>,
    pub duration_ms: Option<i64>,
    pub permission: &'a str,
}

#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub id: String,
    pub role: String,
    pub content: Option<String>,
    pub tool_calls_json: Option<String>,
    pub tool_call_id: Option<String>,
    pub created_at: String,
}
