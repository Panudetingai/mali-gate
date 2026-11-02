use rusqlite::Connection;
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("time format error: {0}")]
    Time(#[from] time::error::Format),
    #[error("agent not found: {0}")]
    NotFound(String),
    #[error("agent id already exists: {0}")]
    Exists(String),
}

#[derive(Debug, Clone)]
pub struct AgentRecord {
    pub id: String,
    pub label: String,
    pub created_at: String,
}

pub struct AgentStore {
    conn: Connection,
}

impl AgentStore {
    pub fn open(path: &std::path::Path) -> Result<Self, AgentError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agents (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                token_hash TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )?;
        Ok(Self { conn })
    }

    pub fn count(&self) -> Result<usize, AgentError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM agents", [], |row| row.get(0))?;
        Ok(n as usize)
    }

    pub fn list(&self) -> Result<Vec<AgentRecord>, AgentError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, label, created_at FROM agents ORDER BY created_at ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok(AgentRecord {
                id: row.get(0)?,
                label: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(AgentError::from)
    }

    pub fn add(&self, id: &str, label: &str) -> Result<(String, AgentRecord), AgentError> {
        if self.id_exists(id)? {
            return Err(AgentError::Exists(id.into()));
        }
        let token = generate_token();
        let hash = hash_token(&token);
        let created_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        self.conn.execute(
            "INSERT INTO agents (id, label, token_hash, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, label, hash, created_at],
        )?;
        Ok((
            token,
            AgentRecord {
                id: id.into(),
                label: label.into(),
                created_at,
            },
        ))
    }

    /// มี agent id นี้ใน DB หรือไม่
    pub fn exists(&self, id: &str) -> Result<bool, AgentError> {
        self.id_exists(id)
    }

    /// ออก token ใหม่ให้ agent เดิม (token เก่าใช้ไม่ได้ทันที) — คืน token แบบ plain ครั้งเดียว
    pub fn rotate(&self, id: &str) -> Result<String, AgentError> {
        let token = generate_token();
        let n = self.conn.execute(
            "UPDATE agents SET token_hash = ?1 WHERE id = ?2",
            rusqlite::params![hash_token(&token), id],
        )?;
        if n == 0 {
            return Err(AgentError::NotFound(id.into()));
        }
        Ok(token)
    }

    pub fn revoke(&self, id: &str) -> Result<(), AgentError> {
        let n = self
            .conn
            .execute("DELETE FROM agents WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(AgentError::NotFound(id.into()));
        }
        Ok(())
    }

    pub fn resolve_token(&self, token: &str) -> Result<Option<String>, AgentError> {
        if self.count()? == 0 {
            return Ok(None);
        }
        let hash = hash_token(token);
        let found: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM agents WHERE token_hash = ?1",
                [hash],
                |row| row.get(0),
            )
            .ok();
        Ok(found)
    }

    fn id_exists(&self, id: &str) -> Result<bool, AgentError> {
        let n: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM agents WHERE id = ?1", [id], |row| {
                    row.get(0)
                })?;
        Ok(n > 0)
    }
}

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    format!("mg_{}", hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hub-policy-agents-{name}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn add_list_revoke() {
        let path = tmp_db("cycle");
        let store = AgentStore::open(&path).unwrap();
        let (token, rec) = store.add("cursor", "Cursor IDE").unwrap();
        assert_eq!(rec.id, "cursor");
        assert!(store.resolve_token(&token).unwrap().as_deref() == Some("cursor"));
        assert_eq!(store.list().unwrap().len(), 1);
        store.revoke("cursor").unwrap();
        assert!(store.resolve_token(&token).unwrap().is_none());
    }

    #[test]
    fn rotate_invalidates_old_token() {
        let path = tmp_db("rotate");
        let store = AgentStore::open(&path).unwrap();
        let (old, _) = store.add("codex", "Codex").unwrap();
        // ต้องมี agent อื่นด้วย เพื่อให้ resolve_token ไม่ short-circuit
        store.add("other", "Other").unwrap();
        let new = store.rotate("codex").unwrap();
        assert_ne!(old, new);
        assert!(store.resolve_token(&old).unwrap().is_none());
        assert_eq!(store.resolve_token(&new).unwrap().as_deref(), Some("codex"));
        assert!(store.exists("codex").unwrap());
        assert!(matches!(store.rotate("nope"), Err(AgentError::NotFound(_))));
    }
}
