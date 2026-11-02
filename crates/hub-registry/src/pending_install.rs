//! คิวติดตั้งจาก agent — รอผู้ใช้อนุมัติผ่าน CLI

use rusqlite::Connection;
use serde_json::Value;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Error, Debug)]
pub enum PendingInstallError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("time error: {0}")]
    Time(#[from] time::error::Format),
    #[error("not found: {0}")]
    NotFound(i64),
    #[error("already resolved")]
    AlreadyResolved,
}

#[derive(Debug, Clone)]
pub struct PendingInstall {
    pub id: i64,
    pub created_at: String,
    pub agent: String,
    pub source: String,
    pub skill_name: String,
    pub description: String,
    pub preview_md: String,
    pub pinned_hash: String,
}

pub struct PendingInstallStore {
    conn: Connection,
}

impl PendingInstallStore {
    pub fn open(path: &std::path::Path) -> Result<Self, PendingInstallError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_installs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at TEXT NOT NULL,
                agent TEXT NOT NULL,
                source TEXT NOT NULL,
                skill_name TEXT NOT NULL,
                description TEXT NOT NULL,
                preview_md TEXT NOT NULL,
                pinned_hash TEXT NOT NULL,
                resolved TEXT
            );",
        )?;
        Ok(Self { conn })
    }

    pub fn insert(
        &self,
        agent: &str,
        source: &str,
        skill_name: &str,
        description: &str,
        preview_md: &str,
        pinned_hash: &str,
    ) -> Result<i64, PendingInstallError> {
        let created_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        self.conn.execute(
            "INSERT INTO pending_installs
             (created_at, agent, source, skill_name, description, preview_md, pinned_hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![
                created_at,
                agent,
                source,
                skill_name,
                description,
                preview_md,
                pinned_hash
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_open(&self) -> Result<Vec<PendingInstall>, PendingInstallError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, agent, source, skill_name, description, preview_md, pinned_hash
             FROM pending_installs WHERE resolved IS NULL ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PendingInstall {
                id: row.get(0)?,
                created_at: row.get(1)?,
                agent: row.get(2)?,
                source: row.get(3)?,
                skill_name: row.get(4)?,
                description: row.get(5)?,
                preview_md: row.get(6)?,
                pinned_hash: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(PendingInstallError::from)
    }

    pub fn get_open(&self, id: i64) -> Result<PendingInstall, PendingInstallError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, agent, source, skill_name, description, preview_md, pinned_hash
             FROM pending_installs WHERE id = ?1 AND resolved IS NULL",
        )?;
        stmt.query_row([id], |row| {
            Ok(PendingInstall {
                id: row.get(0)?,
                created_at: row.get(1)?,
                agent: row.get(2)?,
                source: row.get(3)?,
                skill_name: row.get(4)?,
                description: row.get(5)?,
                preview_md: row.get(6)?,
                pinned_hash: row.get(7)?,
            })
        })
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => PendingInstallError::NotFound(id),
            other => PendingInstallError::Db(other),
        })
    }

    pub fn resolve(&self, id: i64, tag: &str) -> Result<(), PendingInstallError> {
        let n = self.conn.execute(
            "UPDATE pending_installs SET resolved = ?1 WHERE id = ?2 AND resolved IS NULL",
            rusqlite::params![tag, id],
        )?;
        if n == 0 {
            return Err(PendingInstallError::NotFound(id));
        }
        Ok(())
    }

    pub fn poll_resolved(&self, id: i64) -> Result<Option<String>, PendingInstallError> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT resolved FROM pending_installs WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .ok();
        Ok(text.filter(|t| t == "approved" || t == "denied"))
    }

    pub fn log_args(&self, row: &PendingInstall) -> Value {
        serde_json::json!({
            "source": row.source,
            "skill_name": row.skill_name,
            "pinned_hash": row.pinned_hash,
        })
    }
}
