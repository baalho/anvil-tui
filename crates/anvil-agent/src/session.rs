use anyhow::Result;
use anvil_llm::ChatMessage;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// Snapshot of agent state that survives process exit.
///
/// Captures everything needed to reconstruct an Agent from cold start:
/// the full message history plus mode, persona, skills, and model profile.
/// Persisted to SQLite after every turn so `anvil --continue` restores
/// the exact agent state. In v2.0, this same snapshot enables the daemon
/// to resume sessions after restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    /// Active skill keys (not full Skill structs — those are resolved at load time).
    pub active_skills: Vec<String>,
    /// Operating mode: "coding" or "creative".
    pub mode: String,
    /// Active persona key, if any.
    pub persona: Option<String>,
    /// Model profile name for re-matching on resume.
    pub model_profile: Option<String>,
}

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

        // FTS5 virtual table for full-text search across message content.
        self.conn.execute_batch(
            "
            CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                session_id,
                role,
                content
            );
            ",
        )?;

        // Add token usage and cost columns to sessions (idempotent).
        // These track cumulative usage per session for cost reporting.
        let has_tokens_col: bool = self
            .conn
            .prepare("SELECT prompt_tokens FROM sessions LIMIT 0")
            .is_ok();
        if !has_tokens_col {
            self.conn.execute_batch(
                "
                ALTER TABLE sessions ADD COLUMN prompt_tokens INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE sessions ADD COLUMN completion_tokens INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE sessions ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE sessions ADD COLUMN estimated_cost_usd REAL;
                ",
            )?;
        }

        // v1.9: Add agent state columns for session resume.
        // These capture mode, persona, skills, and model profile so
        // `anvil --continue` restores the full agent state, not just messages.
        let has_mode_col: bool = self
            .conn
            .prepare("SELECT mode FROM sessions LIMIT 0")
            .is_ok();
        if !has_mode_col {
            self.conn.execute_batch(
                "
                ALTER TABLE sessions ADD COLUMN active_skills TEXT DEFAULT '[]';
                ALTER TABLE sessions ADD COLUMN mode TEXT DEFAULT 'coding';
                ALTER TABLE sessions ADD COLUMN persona TEXT DEFAULT NULL;
                ALTER TABLE sessions ADD COLUMN model_profile TEXT DEFAULT NULL;
                ",
            )?;
            tracing::info!("migrated sessions table: added agent state columns (v1.9)");
        }

        // v2.1: Incremental turn message persistence.
        // Stores the full ChatMessage JSON per row so crash recovery
        // can reconstruct the exact conversation state without relying
        // on the decomposed `messages` table.
        let has_turn_messages: bool = self
            .conn
            .prepare("SELECT id FROM turn_messages LIMIT 0")
            .is_ok();
        if !has_turn_messages {
            self.conn.execute_batch(
                "
                CREATE TABLE turn_messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL,
                    seq INTEGER NOT NULL,
                    message_json TEXT NOT NULL,
                    created_at DATETIME NOT NULL DEFAULT (datetime('now')),
                    FOREIGN KEY (session_id) REFERENCES sessions(id)
                );
                CREATE INDEX idx_turn_messages_session
                    ON turn_messages(session_id, seq);
                ",
            )?;
            tracing::info!("created turn_messages table (v2.1)");
        }

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

        // Index content for full-text search
        if let Some(text) = content {
            if !text.is_empty() {
                let _ = self.conn.execute(
                    "INSERT INTO messages_fts (session_id, role, content) VALUES (?1, ?2, ?3)",
                    params![session_id, role, text],
                );
            }
        }

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

    /// Persist cumulative token usage and cost for a session.
    /// Called after each agent turn so cost data survives restarts.
    pub fn update_session_usage(
        &self,
        session_id: &str,
        usage: &anvil_llm::TokenUsage,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sessions SET prompt_tokens = ?1, completion_tokens = ?2, \
             total_tokens = ?3, estimated_cost_usd = ?4, updated_at = ?5 WHERE id = ?6",
            params![
                usage.prompt_tokens as i64,
                usage.completion_tokens as i64,
                usage.total_tokens as i64,
                usage.estimated_cost_usd,
                now,
                session_id,
            ],
        )?;
        Ok(())
    }

    /// Append a single ChatMessage to the turn log.
    ///
    /// Called after each message is added to the agent's message history
    /// during a turn. If the daemon crashes mid-turn, all messages up to
    /// the crash point are recoverable from this table.
    pub fn append_turn_message(
        &self,
        session_id: &str,
        seq: usize,
        message: &ChatMessage,
    ) -> Result<()> {
        let json = serde_json::to_string(message)?;
        self.conn.execute(
            "INSERT INTO turn_messages (session_id, seq, message_json)
             VALUES (?1, ?2, ?3)",
            params![session_id, seq as i64, json],
        )?;
        Ok(())
    }

    /// Load all turn messages for a session, ordered by sequence number.
    ///
    /// Returns the full `Vec<ChatMessage>` for session resume. Falls back
    /// gracefully if the table doesn't exist (pre-v2.1 databases).
    pub fn load_turn_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>> {
        let mut stmt = match self.conn.prepare(
            "SELECT message_json FROM turn_messages
             WHERE session_id = ?1 ORDER BY seq",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()), // pre-v2.1 database
        };

        let messages: Vec<ChatMessage> = stmt
            .query_map(params![session_id], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

        Ok(messages)
    }

    /// Clear turn messages for a session (called on session end or compaction).
    pub fn clear_turn_messages(&self, session_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM turn_messages WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// Persist agent state metadata after each turn.
    ///
    /// This does NOT re-serialize messages — those are already saved
    /// individually by `save_message()` during the turn. This only
    /// captures the agent's configuration state (mode, persona, skills,
    /// profile) so resume can reconstruct the full agent.
    pub fn save_snapshot(&self, session_id: &str, snapshot: &SessionSnapshot) -> Result<()> {
        let skills_json = serde_json::to_string(&snapshot.active_skills)?;
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            "UPDATE sessions SET
                active_skills = ?1,
                mode = ?2,
                persona = ?3,
                model_profile = ?4,
                updated_at = ?5
             WHERE id = ?6",
            params![
                skills_json,
                snapshot.mode,
                snapshot.persona,
                snapshot.model_profile,
                now,
                session_id,
            ],
        )?;
        Ok(())
    }

    /// Load agent state metadata for session resume.
    ///
    /// Returns `None` for pre-v1.9 sessions that lack the metadata columns.
    /// Messages are loaded separately via `load_messages()`.
    pub fn load_snapshot(&self, session_id: &str) -> Result<Option<SessionSnapshot>> {
        let result = self.conn.query_row(
            "SELECT active_skills, mode, persona, model_profile
             FROM sessions WHERE id = ?1",
            params![session_id],
            |row| {
                let skills_json: String = row.get(0)?;
                let mode: String = row.get(1)?;
                let persona: Option<String> = row.get(2)?;
                let model_profile: Option<String> = row.get(3)?;
                Ok((skills_json, mode, persona, model_profile))
            },
        );

        match result {
            Ok((skills_json, mode, persona, model_profile)) => {
                let active_skills: Vec<String> =
                    serde_json::from_str(&skills_json).unwrap_or_default();

                Ok(Some(SessionSnapshot {
                    active_skills,
                    mode,
                    persona,
                    model_profile,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
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

/// A search result from full-text search across sessions.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub session_id: String,
    pub role: String,
    pub snippet: String,
    pub session_date: String,
}

impl SessionStore {
    /// Search session content using FTS5 full-text search.
    ///
    /// Returns matching snippets with session context. The query supports
    /// FTS5 syntax: quoted phrases, AND/OR/NOT operators, prefix matching.
    pub fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.session_id, f.role, snippet(messages_fts, 2, '»', '«', '...', 32),
                    s.created_at
             FROM messages_fts f
             JOIN sessions s ON s.id = f.session_id
             WHERE messages_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;

        let results = stmt
            .query_map(params![query, limit], |row| {
                Ok(SearchResult {
                    session_id: row.get(0)?,
                    role: row.get(1)?,
                    snippet: row.get(2)?,
                    session_date: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SessionStore {
        SessionStore::open(Path::new(":memory:")).expect("in-memory db")
    }

    #[test]
    fn migration_is_idempotent() {
        let store = test_store();
        // Running migrate again should not fail
        store.migrate().expect("second migration");
        store.migrate().expect("third migration");
    }

    #[test]
    fn save_and_load_snapshot_roundtrip() {
        let store = test_store();
        let session = store.create_session().unwrap();

        let snapshot = SessionSnapshot {
            active_skills: vec!["deploy".into(), "docker".into()],
            mode: "creative".into(),
            persona: Some("sparkle".into()),
            model_profile: Some("qwen3-coder-tq4".into()),
        };

        store.save_snapshot(&session.id, &snapshot).unwrap();
        let loaded = store.load_snapshot(&session.id).unwrap().unwrap();

        assert_eq!(loaded.active_skills, vec!["deploy", "docker"]);
        assert_eq!(loaded.mode, "creative");
        assert_eq!(loaded.persona.as_deref(), Some("sparkle"));
        assert_eq!(loaded.model_profile.as_deref(), Some("qwen3-coder-tq4"));
    }

    #[test]
    fn load_snapshot_defaults_for_new_session() {
        let store = test_store();
        let session = store.create_session().unwrap();

        // No save_snapshot called — should return defaults from migration
        let loaded = store.load_snapshot(&session.id).unwrap().unwrap();
        assert!(loaded.active_skills.is_empty());
        assert_eq!(loaded.mode, "coding");
        assert!(loaded.persona.is_none());
        assert!(loaded.model_profile.is_none());
    }

    #[test]
    fn load_snapshot_nonexistent_session() {
        let store = test_store();
        let loaded = store.load_snapshot("nonexistent-id").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn snapshot_overwrites_previous() {
        let store = test_store();
        let session = store.create_session().unwrap();

        let snap1 = SessionSnapshot {
            active_skills: vec!["deploy".into()],
            mode: "coding".into(),
            persona: None,
            model_profile: None,
        };
        store.save_snapshot(&session.id, &snap1).unwrap();

        let snap2 = SessionSnapshot {
            active_skills: vec!["docker".into(), "k8s".into()],
            mode: "creative".into(),
            persona: Some("bolt".into()),
            model_profile: Some("mlx-default".into()),
        };
        store.save_snapshot(&session.id, &snap2).unwrap();

        let loaded = store.load_snapshot(&session.id).unwrap().unwrap();
        assert_eq!(loaded.active_skills, vec!["docker", "k8s"]);
        assert_eq!(loaded.mode, "creative");
        assert_eq!(loaded.persona.as_deref(), Some("bolt"));
        assert_eq!(loaded.model_profile.as_deref(), Some("mlx-default"));
    }

    #[test]
    fn snapshot_with_empty_skills() {
        let store = test_store();
        let session = store.create_session().unwrap();

        let snapshot = SessionSnapshot {
            active_skills: vec![],
            mode: "coding".into(),
            persona: None,
            model_profile: None,
        };
        store.save_snapshot(&session.id, &snapshot).unwrap();

        let loaded = store.load_snapshot(&session.id).unwrap().unwrap();
        assert!(loaded.active_skills.is_empty());
    }

    #[test]
    fn append_and_load_turn_messages() {
        let store = test_store();
        let session = store.create_session().unwrap();

        let msg1 = ChatMessage::user("hello");
        let msg2 = ChatMessage::assistant("hi there");

        store.append_turn_message(&session.id, 0, &msg1).unwrap();
        store.append_turn_message(&session.id, 1, &msg2).unwrap();

        let loaded = store.load_turn_messages(&session.id).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].role, anvil_llm::Role::User);
        assert_eq!(loaded[1].role, anvil_llm::Role::Assistant);
        assert_eq!(loaded[0].content.as_deref(), Some("hello"));
        assert_eq!(loaded[1].content.as_deref(), Some("hi there"));
    }

    #[test]
    fn load_turn_messages_empty_session() {
        let store = test_store();
        let session = store.create_session().unwrap();

        let loaded = store.load_turn_messages(&session.id).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_turn_messages_nonexistent_session() {
        let store = test_store();
        let loaded = store.load_turn_messages("nonexistent").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn clear_turn_messages() {
        let store = test_store();
        let session = store.create_session().unwrap();

        store
            .append_turn_message(&session.id, 0, &ChatMessage::user("test"))
            .unwrap();
        assert_eq!(store.load_turn_messages(&session.id).unwrap().len(), 1);

        store.clear_turn_messages(&session.id).unwrap();
        assert!(store.load_turn_messages(&session.id).unwrap().is_empty());
    }

    #[test]
    fn turn_messages_preserve_order() {
        let store = test_store();
        let session = store.create_session().unwrap();

        for i in 0..5 {
            let msg = ChatMessage::user(&format!("msg-{i}"));
            store.append_turn_message(&session.id, i, &msg).unwrap();
        }

        let loaded = store.load_turn_messages(&session.id).unwrap();
        assert_eq!(loaded.len(), 5);
        for (i, msg) in loaded.iter().enumerate() {
            assert_eq!(msg.content.as_deref(), Some(format!("msg-{i}").as_str()));
        }
    }
}

