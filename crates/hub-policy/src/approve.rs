use rusqlite::Connection;
use serde_json::Value;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Error, Debug)]
pub enum ApproveError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("time format error: {0}")]
    Time(#[from] time::error::Format),
    #[error("approval not found: {0}")]
    NotFound(i64),
    #[error("approval already resolved")]
    AlreadyResolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Approved,
    Denied,
}

/// ผู้ใช้ตอบคำขอแบบไหน — ใช้ตัดสินว่าจะเขียนสิทธิ์ลง policy อย่างไร
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantScope {
    /// ทุก agent (`[shared]`)
    Shared,
    /// เฉพาะ agent ที่ขอ
    Agent,
    /// ครั้งนี้ครั้งเดียว ไม่เขียน policy
    Once,
    /// ไม่ให้ + จำเป็น deny ของ agent นั้น
    Deny,
}

impl GrantScope {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantScope::Shared => "shared",
            GrantScope::Agent => "agent",
            GrantScope::Once => "once",
            GrantScope::Deny => "deny",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "shared" => Some(GrantScope::Shared),
            "agent" => Some(GrantScope::Agent),
            "once" => Some(GrantScope::Once),
            "deny" => Some(GrantScope::Deny),
            _ => None,
        }
    }
}

/// คำขอแบบไหนให้สิทธิ์อะไร — `call` ให้ทั้ง server (ไม่ต้องถามทีละ tool)
/// target ที่มี `*`/`?` หรือว่าง ไม่มีวันกลายเป็นสิทธิ์ (กัน agent ขอ "*" แล้วได้ทุกอย่าง)
pub fn grant_for(kind: &str, target: &str) -> Option<crate::Grant> {
    if target.is_empty() || target.contains(['*', '?']) {
        return None;
    }
    match kind {
        "skill_load" | "skill_file" => Some(crate::Grant::Skill(target.into())),
        "connect" | "call" => Some(crate::Grant::McpServer(target.into())),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub id: i64,
    pub created_at: String,
    pub agent: String,
    pub kind: String,
    pub target: String,
    pub tool: String,
    pub summary: String,
    pub grant_scope: Option<GrantScope>,
}

pub struct ApprovalStore {
    conn: Connection,
}

impl ApprovalStore {
    pub fn open(path: &std::path::Path) -> Result<Self, ApproveError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_approvals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at TEXT NOT NULL,
                agent TEXT NOT NULL,
                kind TEXT NOT NULL,
                target TEXT NOT NULL,
                tool TEXT NOT NULL,
                args_redacted TEXT NOT NULL DEFAULT '{}',
                resolved TEXT
            );",
        )?;
        // migration: grant_scope (shared|agent|once|deny)
        let has_scope = conn
            .prepare("PRAGMA table_info(pending_approvals)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(Result::ok)
            .any(|c| c == "grant_scope");
        if !has_scope {
            conn.execute_batch("ALTER TABLE pending_approvals ADD COLUMN grant_scope TEXT;")?;
        }
        Ok(Self { conn })
    }

    pub fn insert(
        &self,
        agent: &str,
        kind: &str,
        target: &str,
        tool: &str,
        args: &Value,
    ) -> Result<i64, ApproveError> {
        let created_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let args_text = args.to_string();
        self.conn.execute(
            "INSERT INTO pending_approvals (created_at, agent, kind, target, tool, args_redacted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![created_at, agent, kind, target, tool, args_text],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_open(&self) -> Result<Vec<PendingApproval>, ApproveError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, agent, kind, target, tool, args_redacted
             FROM pending_approvals WHERE resolved IS NULL ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let args: String = row.get(6)?;
            Ok(PendingApproval {
                id: row.get(0)?,
                created_at: row.get(1)?,
                agent: row.get(2)?,
                kind: row.get(3)?,
                target: row.get(4)?,
                tool: row.get(5)?,
                summary: args,
                grant_scope: None,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ApproveError::from)
    }

    pub fn get(&self, id: i64) -> Result<PendingApproval, ApproveError> {
        self.conn
            .query_row(
                "SELECT id, created_at, agent, kind, target, tool, args_redacted, grant_scope
                 FROM pending_approvals WHERE id = ?1",
                [id],
                |row| {
                    let scope: Option<String> = row.get(7)?;
                    Ok(PendingApproval {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        agent: row.get(2)?,
                        kind: row.get(3)?,
                        target: row.get(4)?,
                        tool: row.get(5)?,
                        summary: row.get(6)?,
                        grant_scope: scope.as_deref().and_then(GrantScope::parse),
                    })
                },
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => ApproveError::NotFound(id),
                other => ApproveError::Db(other),
            })
    }

    pub fn resolve(&self, id: i64, outcome: ApprovalOutcome) -> Result<(), ApproveError> {
        let scope = match outcome {
            ApprovalOutcome::Approved => GrantScope::Once,
            ApprovalOutcome::Denied => GrantScope::Deny,
        };
        self.resolve_scoped(id, scope)
    }

    pub fn resolve_scoped(&self, id: i64, scope: GrantScope) -> Result<(), ApproveError> {
        let tag = match scope {
            GrantScope::Deny => "denied",
            _ => "approved",
        };
        let n = self.conn.execute(
            "UPDATE pending_approvals SET resolved = ?1, grant_scope = ?2
             WHERE id = ?3 AND resolved IS NULL",
            rusqlite::params![tag, scope.as_str(), id],
        )?;
        if n == 0 {
            self.get(id)?;
            return Err(ApproveError::AlreadyResolved);
        }
        Ok(())
    }

    pub fn poll_resolved(&self, id: i64) -> Result<Option<ApprovalOutcome>, ApproveError> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT resolved FROM pending_approvals WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .ok();
        Ok(match text.as_deref() {
            None | Some("") => None,
            Some("approved") => Some(ApprovalOutcome::Approved),
            Some("denied") => Some(ApprovalOutcome::Denied),
            Some(_) => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Grant;

    fn tmp_db(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hub-policy-approve-{name}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn migration_adds_grant_scope_to_old_table() {
        let path = tmp_db("migrate");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE pending_approvals (
                id INTEGER PRIMARY KEY AUTOINCREMENT, created_at TEXT NOT NULL,
                agent TEXT NOT NULL, kind TEXT NOT NULL, target TEXT NOT NULL,
                tool TEXT NOT NULL, args_redacted TEXT NOT NULL DEFAULT '{}', resolved TEXT);",
        )
        .unwrap();
        drop(conn);
        let store = ApprovalStore::open(&path).unwrap();
        let _again = ApprovalStore::open(&path).unwrap(); // migration idempotent
        let id = store
            .insert(
                "a",
                "skill_load",
                "pdf",
                "skills_load",
                &serde_json::json!({}),
            )
            .unwrap();
        store.resolve_scoped(id, GrantScope::Agent).unwrap();
        assert_eq!(store.get(id).unwrap().grant_scope, Some(GrantScope::Agent));
    }

    #[test]
    fn resolve_scoped_sets_outcome() {
        let store = ApprovalStore::open(&tmp_db("scoped")).unwrap();
        let args = serde_json::json!({});
        let a = store
            .insert("a", "connect", "gh", "connect", &args)
            .unwrap();
        let b = store
            .insert("a", "connect", "gh", "connect", &args)
            .unwrap();
        let c = store
            .insert("a", "connect", "gh", "connect", &args)
            .unwrap();
        store.resolve_scoped(a, GrantScope::Shared).unwrap();
        store.resolve_scoped(b, GrantScope::Deny).unwrap();
        store.resolve_scoped(c, GrantScope::Once).unwrap();
        assert_eq!(
            store.poll_resolved(a).unwrap(),
            Some(ApprovalOutcome::Approved)
        );
        assert_eq!(
            store.poll_resolved(b).unwrap(),
            Some(ApprovalOutcome::Denied)
        );
        assert_eq!(
            store.poll_resolved(c).unwrap(),
            Some(ApprovalOutcome::Approved)
        );
        assert!(matches!(
            store.resolve_scoped(a, GrantScope::Once),
            Err(ApproveError::AlreadyResolved)
        ));
        let got = store.get(a).unwrap();
        assert_eq!((got.agent.as_str(), got.kind.as_str()), ("a", "connect"));
    }

    #[test]
    fn grant_for_maps_kinds() {
        assert_eq!(
            grant_for("skill_load", "pdf"),
            Some(Grant::Skill("pdf".into()))
        );
        assert_eq!(
            grant_for("skill_file", "pdf"),
            Some(Grant::Skill("pdf".into()))
        );
        assert_eq!(
            grant_for("connect", "gh"),
            Some(Grant::McpServer("gh".into()))
        );
        assert_eq!(grant_for("call", "gh"), Some(Grant::McpServer("gh".into())));
        assert_eq!(grant_for("install", "x"), None);
        assert_eq!(GrantScope::parse("shared"), Some(GrantScope::Shared));
        assert_eq!(GrantScope::Deny.as_str(), "deny");
    }

    #[test]
    fn grant_for_refuses_wildcards() {
        for t in ["*", "pdf*", "*x", "a?b", ""] {
            assert_eq!(grant_for("connect", t), None, "{t}");
            assert_eq!(grant_for("skill_load", t), None, "{t}");
        }
    }
}
