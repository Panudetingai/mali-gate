//! Log events ลง SQLite ตาราง `events`
//! (id, ts, agent, session_id, kind, target, tool, args_redacted, decision,
//!  result_status, duration_ms, error)
//!
//! กติกา: ทุกการเขียน log ต้องผ่านฟังก์ชัน redact ก่อนเขียนเสมอ (ดู CLAUDE.md)
//!
//! เริ่มจริงในเฟส 1 — ตารางคือ migration-safe (`CREATE TABLE IF NOT EXISTS`)

mod query;

use rusqlite::Connection;

pub use query::{AgentStats, AgentUsageStats, LogQuery, UsedResources};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// ชนิดเหตุการณ์ที่ hub บันทึกได้ (คงที่ตาม schema ใน CLAUDE.md)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventKind {
    SkillSearch,
    SkillLoad,
    SkillFile,
    Install,
    Connect,
    Call,
    Release,
    PolicyDenied,
    Approval,
    // เฟสถัด ๆ ไปเพิ่มได้ — จัดเก็บเป็น text ใน SQL
}

impl EventKind {
    /// ข้อความที่จัดเก็บในฐานข้อมูล (ไม่เปลี่ยนเมื่อครั้งหน้า)
    pub fn as_str(&self) -> &'static str {
        self.store()
    }

    fn store(&self) -> &'static str {
        match self {
            EventKind::SkillSearch => "skill_search",
            EventKind::SkillLoad => "skill_load",
            EventKind::SkillFile => "skill_file",
            EventKind::Install => "install",
            EventKind::Connect => "connect",
            EventKind::Call => "call",
            EventKind::Release => "release",
            EventKind::PolicyDenied => "policy_denied",
            EventKind::Approval => "approval",
        }
    }

    /// แปลกลับจากสตริงในฐานข้อมูล
    pub fn parse_str(text: &str) -> Option<Self> {
        Self::parse(text)
    }

    fn parse(text: &str) -> Option<Self> {
        let found = match text {
            "skill_search" => EventKind::SkillSearch,
            "skill_load" => EventKind::SkillLoad,
            "skill_file" => EventKind::SkillFile,
            "install" => EventKind::Install,
            "connect" => EventKind::Connect,
            "call" => EventKind::Call,
            "release" => EventKind::Release,
            "policy_denied" => EventKind::PolicyDenied,
            "approval" => EventKind::Approval,
            _ => return None,
        };
        Some(found)
    }
}

/// ผลตัดสินสิทธิ์ของเหตุการณ์ (allow/deny/ask)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Decision {
    Allow,
    Deny,
    AskApproved,
    AskDenied,
}

impl Decision {
    pub fn as_str(&self) -> &'static str {
        self.store()
    }

    fn store(&self) -> &'static str {
        match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
            Decision::AskApproved => "ask_approved",
            Decision::AskDenied => "ask_denied",
        }
    }

    pub fn parse_str(text: &str) -> Option<Self> {
        Self::parse(text)
    }

    fn parse(text: &str) -> Option<Self> {
        let found = match text {
            "allow" => Decision::Allow,
            "deny" => Decision::Deny,
            "ask_approved" => Decision::AskApproved,
            "ask_denied" => Decision::AskDenied,
            _ => return None,
        };
        Some(found)
    }
}

/// เหตุการณ์ที่ agent ร้องขอผ่าน hub — เขียนครั้งละหนึ่งเรคอร์ด
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// ตัวตน agent ตาม token ที่ออกให้ (ไม่ใช่ label จาก handshake)
    pub agent: String,
    pub session_id: String,
    pub kind: EventKind,
    /// เป้าหมายที่จะกระทำ เช่น ชื่อ skill หรือชื่อ server
    pub target: String,
    /// ชื่อเครื่องมือที่ร้องขอ เช่น skills_search
    pub tool: String,
    /// พารามิเตอร์เดิม — redact ก่อนลงฐานเสมอ
    pub args: Value,
    pub decision: Option<Decision>,
    pub result_status: Option<String>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
}

/// เรคอร์ดที่อ่านคืนจากฐาน (มี id + ts ที่ hub ใส่ตอนเขียน)
#[derive(Debug, Clone)]
pub struct EventRow {
    pub id: i64,
    pub ts: String,
    pub agent: String,
    pub session_id: String,
    pub kind: EventKind,
    pub target: String,
    pub tool: String,
    pub args: Value,
    pub decision: Option<Decision>,
    pub result_status: Option<String>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum LogError {
    #[error("failed to open log database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("failed to stamp timestamp: {0}")]
    Time(#[from] time::error::Format),
    #[error("failed to encode args: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unknown event kind stored in db: {0}")]
    UnknownKind(String),
    #[error("unknown decision stored in db: {0}")]
    UnknownDecision(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// คีย์ตรง marker ทั้งก้อน หรือเป็นส่วนคำคั่นด้วย `_`/`-` (ไม่ใช่ substring กลางคำ)
fn key_matches_secret_marker(key: &str, needle: &str) -> bool {
    if key == needle {
        return true;
    }
    key.starts_with(&format!("{needle}_"))
        || key.ends_with(&format!("_{needle}"))
        || key.contains(&format!("_{needle}_"))
        || key.starts_with(&format!("{needle}-"))
        || key.ends_with(&format!("-{needle}"))
        || key.contains(&format!("-{needle}-"))
}

/// คำชี้บอกว่าคีย์นี้พกความลับ
fn looks_secret(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "token",
        "password",
        "secret",
        "apikey",
        "api_key",
        "authorization",
        "credential",
        "passwd",
        "access_key",
    ]
    .iter()
    .any(|needle| key_matches_secret_marker(&lower, needle))
}

/// ตัดความลับออกจาก args — เดิน object ลึกทุกชั้น คีย์ที่พกความลับ
/// ถูกแทนด้วย "***"; สตริงแบบ bearer/sk- ก็ถูกตัดด้วย
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, val)| {
                    if looks_secret(key) {
                        (key.clone(), Value::String("***".into()))
                    } else {
                        (key.clone(), redact(val))
                    }
                })
                .collect::<Map<String, Value>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(text) if text.starts_with("sk-") || text.starts_with("Bearer ") => {
            Value::String("***".into())
        }
        other => other.clone(),
    }
}

/// ผู้เขียนเหตุการณ์ลง SQLite — ตาราง `events` ตาม schema ใน CLAUDE.md
pub struct Logger {
    conn: Connection,
}

impl Logger {
    /// เปิดฐานและยืนยันว่าตารางพร้อม (สร้างถ้าไม่มี — migration-safe)
    pub fn open(path: &std::path::Path) -> Result<Self, LogError> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                ts            TEXT NOT NULL,
                agent         TEXT NOT NULL,
                session_id    TEXT NOT NULL DEFAULT '',
                kind          TEXT NOT NULL,
                target        TEXT NOT NULL DEFAULT '',
                tool          TEXT NOT NULL DEFAULT '',
                args_redacted TEXT NOT NULL DEFAULT '{}',
                decision      TEXT,
                result_status TEXT,
                duration_ms   INTEGER,
                error         TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts DESC);",
        )?;
        Ok(Self { conn })
    }

    /// เขียน event — หน้าที่ของฟังก์ชันนี้คือ redact ให้ก่อนเสมอ
    pub fn write(&self, event: &Event) -> Result<i64, LogError> {
        let ts = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let args = serde_json::to_string(&redact(&event.args))?;
        self.conn.execute(
            "INSERT INTO events (ts, agent, session_id, kind, target, tool, \
             args_redacted, decision, result_status, duration_ms, error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                ts,
                event.agent,
                event.session_id,
                event.kind.store(),
                event.target,
                event.tool,
                args,
                event.decision.as_ref().map(Decision::store),
                event.result_status,
                event.duration_ms,
                event.error,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// อ่าน n เหตุการณ์ล่าสุด (เรียงใหม่ไปเก่า) — ใช้กับ `hub logs`
    pub fn last_n(&self, n: usize) -> Result<Vec<EventRow>, LogError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, agent, session_id, kind, target, tool, \
             args_redacted, decision, result_status, duration_ms, error \
             FROM events ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([n as i64], map_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(LogError::from)
    }
}

pub(crate) fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    let kind_text: String = row.get("kind")?;
    let kind = EventKind::parse(&kind_text).ok_or_else(|| {
        rusqlite::Error::InvalidParameterName(format!("unknown kind: {kind_text}"))
    })?;
    let decision = match row.get::<_, Option<String>>("decision")? {
        None => None,
        Some(text) => Some(Decision::parse(&text).ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(format!("unknown decision: {text}"))
        })?),
    };
    let args_text: String = row.get("args_redacted")?;
    let args = serde_json::from_str(&args_text).map_err(|_| {
        rusqlite::Error::InvalidColumnType(7, "args".into(), rusqlite::types::Type::Text)
    })?;
    Ok(EventRow {
        id: row.get("id")?,
        ts: row.get("ts")?,
        agent: row.get("agent")?,
        session_id: row.get("session_id")?,
        kind,
        target: row.get("target")?,
        tool: row.get("tool")?,
        args,
        decision,
        result_status: row.get("result_status")?,
        duration_ms: row.get("duration_ms")?,
        error: row.get("error")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// redact ต้องตัดทุกชั้นของ object และแบบ bearer/sk- ด้วย
    #[test]
    fn redact_cuts_secrets_at_any_depth() {
        let deep = json!({
            "query": "pdf",
            "meta": { "github_token": "abc123", "note": { "api_key": "zzz" } },
            "header": "Bearer xyz",
            "openai_key": "sk-abc",
        });
        let cut = redact(&deep);
        let text = cut.to_string();
        assert!(!text.contains("abc"));
        assert!(!text.contains("zzz"));
        assert!(!text.contains("xyz"));
        assert!(text.contains("pdf"));
        assert!(text.matches("***").count() >= 4);
    }

    /// คีย์ธรรมดาที่ "มี" อักขระซ้ำกับชื่อความลับ เช่น tokens_used ต้องหลุด?
    /// ไม่ — กติกาที่แน่นคือ ตัดเฉพาะคีย์ที่สื่อความลับ; tokens_used ยอมผ่าน
    #[test]
    fn redact_keeps_benign_keys() {
        let data = json!({ "tokens_used": 3, "note": "safe" });
        let cut = redact(&data);
        assert_eq!(cut["tokens_used"], 3);
        assert_eq!(cut["note"], "safe");
    }

    /// kind/decision จัดเก็บ-แปลกลับต้องเป็นคู่เดียวกัน
    #[test]
    fn enum_mapping_round_trips() {
        for kind in [
            EventKind::SkillSearch,
            EventKind::SkillLoad,
            EventKind::Call,
            EventKind::PolicyDenied,
            EventKind::Approval,
        ] {
            assert_eq!(EventKind::parse(kind.store()), Some(kind));
        }
    }

    /// parse ต้องทำในความละเอียดทาว่า namespaced keys เท่านั้น
    #[test]
    fn redact_keeps_nested_data_intact() {
        let nested = json!({ "a": { "b": [1, 2, { "c": 3 }] } });
        assert_eq!(redact(&nested), nested);
    }
}
