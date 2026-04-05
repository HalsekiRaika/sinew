//! SQLite database operations for the Broker.

use jiff::Timestamp;
use rand::Rng;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use thiserror::Error;

use crate::types::{Message, Peer, PeerId, PeerScope, RegisterRequest, SendMessageRequest};

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("peer not found: {0}")]
    PeerNotFound(String),
}

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    /// Initialize the database: create pool, set WAL mode, create tables and indexes.
    pub async fn new(path: &str) -> Result<Self, DbError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS peers (
                id TEXT PRIMARY KEY,
                pid INTEGER NOT NULL,
                cwd TEXT NOT NULL,
                git_root TEXT,
                tty TEXT,
                summary TEXT,
                registered_at TEXT NOT NULL,
                last_seen TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                text TEXT NOT NULL,
                sent_at TEXT NOT NULL,
                delivered INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_messages_to_delivered ON messages(to_id, delivered)",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_peers_pid ON peers(pid)")
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    /// Initialize an in-memory database (primarily for testing).
    pub async fn new_in_memory() -> Result<Self, DbError> {
        let options = SqliteConnectOptions::new()
            .filename(":memory:")
            .journal_mode(SqliteJournalMode::Wal)
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        let db = Self { pool };

        // Run schema creation manually for in-memory DB
        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS peers (
                id TEXT PRIMARY KEY,
                pid INTEGER NOT NULL,
                cwd TEXT NOT NULL,
                git_root TEXT,
                tty TEXT,
                summary TEXT,
                registered_at TEXT NOT NULL,
                last_seen TEXT NOT NULL
            )",
        )
        .execute(&db.pool)
        .await?;

        sqlx::query(
            r"CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                text TEXT NOT NULL,
                sent_at TEXT NOT NULL,
                delivered INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&db.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_messages_to_delivered ON messages(to_id, delivered)",
        )
        .execute(&db.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_peers_pid ON peers(pid)")
            .execute(&db.pool)
            .await?;

        Ok(db)
    }

    // ---- Peer operations ----

    /// Generate an 8-character alphanumeric peer ID.
    fn generate_peer_id() -> PeerId {
        let mut rng = rand::rng();
        let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789".chars().collect();
        (0..8)
            .map(|_| chars[rng.random_range(0..chars.len())])
            .collect()
    }

    /// Register a new peer, or return existing peer ID if same PID is already registered.
    pub async fn register_peer(&self, req: &RegisterRequest) -> Result<PeerId, DbError> {
        // Check for existing peer with same PID
        if let Some(existing) = self.find_peer_by_pid(req.pid).await? {
            // Update existing registration
            let now = Timestamp::now().to_string();
            sqlx::query("UPDATE peers SET cwd = ?, git_root = ?, tty = ?, summary = ?, last_seen = ? WHERE id = ?")
                .bind(&req.cwd)
                .bind(&req.git_root)
                .bind(&req.tty)
                .bind(&req.summary)
                .bind(&now)
                .bind(&existing.id)
                .execute(&self.pool)
                .await?;
            return Ok(existing.id);
        }

        let id = Self::generate_peer_id();
        let now = Timestamp::now().to_string();

        sqlx::query(
            "INSERT INTO peers (id, pid, cwd, git_root, tty, summary, registered_at, last_seen) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(req.pid)
        .bind(&req.cwd)
        .bind(&req.git_root)
        .bind(&req.tty)
        .bind(&req.summary)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(id)
    }

    /// Update the last_seen timestamp for a peer.
    pub async fn update_heartbeat(&self, id: &PeerId) -> Result<(), DbError> {
        let now = Timestamp::now().to_string();
        let result = sqlx::query("UPDATE peers SET last_seen = ? WHERE id = ?")
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::PeerNotFound(id.clone()));
        }
        Ok(())
    }

    /// Remove a peer from the database.
    pub async fn unregister_peer(&self, id: &PeerId) -> Result<(), DbError> {
        sqlx::query("DELETE FROM peers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Find a peer by PID.
    pub async fn find_peer_by_pid(&self, pid: u32) -> Result<Option<Peer>, DbError> {
        let row = sqlx::query_as::<_, PeerRow>("SELECT * FROM peers WHERE pid = ?")
            .bind(pid)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(Into::into))
    }

    /// Update a peer's summary text.
    pub async fn set_summary(&self, id: &PeerId, summary: &str) -> Result<(), DbError> {
        let result = sqlx::query("UPDATE peers SET summary = ? WHERE id = ?")
            .bind(summary)
            .bind(id)
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::PeerNotFound(id.clone()));
        }
        Ok(())
    }

    /// Get all peers.
    pub async fn get_all_peers(&self) -> Result<Vec<Peer>, DbError> {
        let rows = sqlx::query_as::<_, PeerRow>("SELECT * FROM peers")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Remove multiple peers by their IDs.
    pub async fn remove_peers(&self, ids: &[PeerId]) -> Result<(), DbError> {
        for id in ids {
            sqlx::query("DELETE FROM peers WHERE id = ?")
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// List peers filtered by scope, excluding the requesting peer.
    pub async fn list_peers(
        &self,
        scope: &PeerScope,
        exclude_id: &PeerId,
        reference_cwd: &str,
        reference_git_root: Option<&str>,
    ) -> Result<Vec<Peer>, DbError> {
        let rows = match scope {
            PeerScope::Machine => {
                sqlx::query_as::<_, PeerRow>("SELECT * FROM peers WHERE id != ?")
                    .bind(exclude_id)
                    .fetch_all(&self.pool)
                    .await?
            }
            PeerScope::Directory => {
                sqlx::query_as::<_, PeerRow>("SELECT * FROM peers WHERE id != ? AND cwd = ?")
                    .bind(exclude_id)
                    .bind(reference_cwd)
                    .fetch_all(&self.pool)
                    .await?
            }
            PeerScope::Repo => {
                match reference_git_root {
                    Some(root) => {
                        sqlx::query_as::<_, PeerRow>(
                            "SELECT * FROM peers WHERE id != ? AND git_root = ?",
                        )
                        .bind(exclude_id)
                        .bind(root)
                        .fetch_all(&self.pool)
                        .await?
                    }
                    None => vec![], // No git root means no repo-scoped peers
                }
            }
        };
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ---- Message operations ----

    /// Send a message (enqueue in database).
    pub async fn send_message(&self, msg: &SendMessageRequest) -> Result<i64, DbError> {
        let now = Timestamp::now().to_string();
        let result = sqlx::query(
            "INSERT INTO messages (from_id, to_id, text, sent_at, delivered) VALUES (?, ?, ?, ?, 0)",
        )
        .bind(&msg.from_id)
        .bind(&msg.to_id)
        .bind(&msg.text)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        Ok(result.last_insert_rowid())
    }

    /// Poll undelivered messages for a peer and mark them as delivered.
    pub async fn poll_messages(&self, peer_id: &PeerId) -> Result<Vec<Message>, DbError> {
        let rows = sqlx::query_as::<_, MessageRow>(
            "SELECT * FROM messages WHERE to_id = ? AND delivered = 0",
        )
        .bind(peer_id)
        .fetch_all(&self.pool)
        .await?;

        // Mark as delivered
        sqlx::query("UPDATE messages SET delivered = 1 WHERE to_id = ? AND delivered = 0")
            .bind(peer_id)
            .execute(&self.pool)
            .await?;

        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Check if a peer exists.
    pub async fn peer_exists(&self, id: &PeerId) -> Result<bool, DbError> {
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM peers WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(count.0 > 0)
    }
}

// Internal row types for sqlx::FromRow mapping

#[derive(sqlx::FromRow)]
struct PeerRow {
    id: String,
    pid: i64,
    cwd: String,
    git_root: Option<String>,
    tty: Option<String>,
    summary: Option<String>,
    registered_at: String,
    last_seen: String,
}

impl From<PeerRow> for Peer {
    fn from(row: PeerRow) -> Self {
        Self {
            id: row.id,
            pid: row.pid as u32,
            cwd: row.cwd,
            git_root: row.git_root,
            tty: row.tty,
            summary: row.summary,
            registered_at: row.registered_at,
            last_seen: row.last_seen,
        }
    }
}

#[derive(sqlx::FromRow)]
struct MessageRow {
    id: i64,
    from_id: String,
    to_id: String,
    text: String,
    sent_at: String,
    delivered: i32,
}

impl From<MessageRow> for Message {
    fn from(row: MessageRow) -> Self {
        Self {
            id: row.id,
            from_id: row.from_id,
            to_id: row.to_id,
            text: row.text,
            sent_at: row.sent_at,
            delivered: row.delivered != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> Database {
        Database::new_in_memory()
            .await
            .expect("failed to create in-memory db")
    }

    fn test_register_request(pid: u32, cwd: &str) -> RegisterRequest {
        RegisterRequest {
            pid,
            cwd: cwd.into(),
            git_root: Some("/repo".into()),
            tty: Some("/dev/pts/0".into()),
            summary: None,
        }
    }

    // ---- Peer registration ----

    #[tokio::test]
    async fn register_peer_returns_8char_alphanumeric_id() {
        let db = setup_db().await;
        let id = db
            .register_peer(&test_register_request(1000, "/home"))
            .await
            .unwrap();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[tokio::test]
    async fn duplicate_pid_reuses_existing_id() {
        let db = setup_db().await;
        let id1 = db
            .register_peer(&test_register_request(1000, "/home"))
            .await
            .unwrap();
        let id2 = db
            .register_peer(&test_register_request(1000, "/other"))
            .await
            .unwrap();
        assert_eq!(id1, id2);
    }

    #[tokio::test]
    async fn different_pids_get_unique_ids() {
        let db = setup_db().await;
        let id1 = db
            .register_peer(&test_register_request(1000, "/home"))
            .await
            .unwrap();
        let id2 = db
            .register_peer(&test_register_request(2000, "/home"))
            .await
            .unwrap();
        assert_ne!(id1, id2);
    }

    // ---- Peer lifecycle ----

    #[tokio::test]
    async fn heartbeat_fails_for_nonexistent_peer() {
        let db = setup_db().await;
        let result = db.update_heartbeat(&"notfound".into()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unregister_removes_peer() {
        let db = setup_db().await;
        let id = db
            .register_peer(&test_register_request(1000, "/home"))
            .await
            .unwrap();
        db.unregister_peer(&id).await.unwrap();
        let found = db.find_peer_by_pid(1000).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn set_summary_persists_and_is_retrievable() {
        let db = setup_db().await;
        let id = db
            .register_peer(&test_register_request(1000, "/home"))
            .await
            .unwrap();
        db.set_summary(&id, "Working on feature X").await.unwrap();
        let peer = db.find_peer_by_pid(1000).await.unwrap().unwrap();
        assert_eq!(peer.summary, Some("Working on feature X".into()));
    }

    #[tokio::test]
    async fn set_summary_fails_for_nonexistent_peer() {
        let db = setup_db().await;
        let result = db.set_summary(&"notfound".into(), "test").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn remove_peers_only_deletes_specified_ids() {
        let db = setup_db().await;
        let id1 = db
            .register_peer(&test_register_request(1000, "/a"))
            .await
            .unwrap();
        let id2 = db
            .register_peer(&test_register_request(2000, "/b"))
            .await
            .unwrap();
        db.remove_peers(&[id1]).await.unwrap();
        let peers = db.get_all_peers().await.unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, id2);
    }

    // ---- Message delivery ----

    #[tokio::test]
    async fn poll_returns_undelivered_messages_then_marks_as_delivered() {
        let db = setup_db().await;
        db.send_message(&SendMessageRequest {
            from_id: "a".into(),
            to_id: "b".into(),
            text: "msg1".into(),
        })
        .await
        .unwrap();
        db.send_message(&SendMessageRequest {
            from_id: "a".into(),
            to_id: "b".into(),
            text: "msg2".into(),
        })
        .await
        .unwrap();

        let msgs = db.poll_messages(&"b".into()).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "msg1");
        assert_eq!(msgs[1].text, "msg2");

        // Second poll returns nothing — messages are now delivered
        let msgs = db.poll_messages(&"b".into()).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn messages_are_routed_only_to_target_peer() {
        let db = setup_db().await;
        db.send_message(&SendMessageRequest {
            from_id: "a".into(),
            to_id: "b".into(),
            text: "for b".into(),
        })
        .await
        .unwrap();
        db.send_message(&SendMessageRequest {
            from_id: "a".into(),
            to_id: "c".into(),
            text: "for c".into(),
        })
        .await
        .unwrap();

        let msgs_b = db.poll_messages(&"b".into()).await.unwrap();
        assert_eq!(msgs_b.len(), 1);
        assert_eq!(msgs_b[0].text, "for b");
    }

    // ---- Scope-based peer listing ----

    async fn setup_scoped_peers(db: &Database) -> (PeerId, PeerId, PeerId) {
        let id1 = db
            .register_peer(&RegisterRequest {
                pid: 1000,
                cwd: "/project/a".into(),
                git_root: Some("/project".into()),
                tty: None,
                summary: None,
            })
            .await
            .unwrap();
        let id2 = db
            .register_peer(&RegisterRequest {
                pid: 2000,
                cwd: "/project/a".into(),
                git_root: Some("/project".into()),
                tty: None,
                summary: None,
            })
            .await
            .unwrap();
        let id3 = db
            .register_peer(&RegisterRequest {
                pid: 3000,
                cwd: "/other".into(),
                git_root: Some("/other".into()),
                tty: None,
                summary: None,
            })
            .await
            .unwrap();
        (id1, id2, id3)
    }

    #[tokio::test]
    async fn machine_scope_returns_all_peers_except_self() {
        let db = setup_db().await;
        let (id1, _id2, _id3) = setup_scoped_peers(&db).await;
        let peers = db
            .list_peers(&PeerScope::Machine, &id1, "/project/a", Some("/project"))
            .await
            .unwrap();
        assert_eq!(peers.len(), 2);
        assert!(peers.iter().all(|p| p.id != id1));
    }

    #[tokio::test]
    async fn directory_scope_returns_only_same_cwd() {
        let db = setup_db().await;
        let (id1, id2, _id3) = setup_scoped_peers(&db).await;
        let peers = db
            .list_peers(&PeerScope::Directory, &id1, "/project/a", Some("/project"))
            .await
            .unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, id2);
    }

    #[tokio::test]
    async fn repo_scope_returns_only_same_git_root() {
        let db = setup_db().await;
        let (id1, id2, _id3) = setup_scoped_peers(&db).await;
        let peers = db
            .list_peers(&PeerScope::Repo, &id1, "/project/a", Some("/project"))
            .await
            .unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].id, id2);
    }

    #[tokio::test]
    async fn repo_scope_without_git_root_returns_empty() {
        let db = setup_db().await;
        let (id1, _id2, _id3) = setup_scoped_peers(&db).await;
        let peers = db
            .list_peers(&PeerScope::Repo, &id1, "/project/a", None)
            .await
            .unwrap();
        assert!(peers.is_empty());
    }
}
