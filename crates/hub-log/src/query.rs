use std::io::Write;

use rusqlite::types::Value;
use serde::Serialize;

use crate::{Decision, EventKind, EventRow, LogError, Logger};

/// ตัวกรองอ่าน log — ใช้กับ TUI, plain logs, export
#[derive(Debug, Clone)]
pub struct LogQuery {
    pub limit: usize,
    pub agent: Option<String>,
    pub kind: Option<EventKind>,
    pub decision: Option<Decision>,
    /// สำหรับ `--follow`: เอาเฉพาะ id มากกว่าค่านี้
    pub after_id: Option<i64>,
}

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            limit: 50,
            agent: None,
            kind: None,
            decision: None,
            after_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStats {
    pub agent: String,
    pub events: i64,
    pub last_ts: String,
}

/// สรุปต่อ agent สำหรับ TUI — รวมประมาณการ token จาก payload (หรือฟิลด์ `*_tokens` ใน args)
#[derive(Debug, Clone, Serialize)]
pub struct AgentUsageStats {
    pub agent: String,
    pub events: i64,
    pub est_tokens: i64,
    pub last_ts: String,
}

/// skill / MCP server ที่ agent เคยใช้สำเร็จ (allow หรือ ask_approved)
#[derive(Debug, Clone, Default)]
pub struct UsedResources {
    pub skills: std::collections::BTreeSet<String>,
    pub mcp_servers: std::collections::BTreeSet<String>,
}

#[derive(Serialize)]
struct JsonlEvent<'a> {
    id: i64,
    ts: &'a str,
    agent: &'a str,
    session_id: &'a str,
    kind: &'a str,
    target: &'a str,
    tool: &'a str,
    args: &'a serde_json::Value,
    decision: Option<&'a str>,
    result_status: Option<&'a str>,
    duration_ms: Option<i64>,
    error: Option<&'a str>,
}

impl Logger {
    pub fn max_event_id(&self) -> Result<i64, LogError> {
        let id: i64 =
            self.conn
                .query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |row| {
                    row.get(0)
                })?;
        Ok(id)
    }

    pub fn query(&self, q: &LogQuery) -> Result<Vec<EventRow>, LogError> {
        let mut sql = String::from(
            "SELECT id, ts, agent, session_id, kind, target, tool, \
             args_redacted, decision, result_status, duration_ms, error \
             FROM events WHERE 1=1",
        );
        let mut params: Vec<Value> = Vec::new();
        if let Some(agent) = &q.agent {
            sql.push_str(" AND agent = ?");
            params.push(agent.clone().into());
        }
        if let Some(kind) = &q.kind {
            sql.push_str(" AND kind = ?");
            params.push(Value::Text(kind.as_str().into()));
        }
        if let Some(decision) = &q.decision {
            sql.push_str(" AND decision = ?");
            params.push(Value::Text(decision.as_str().into()));
        }
        if let Some(after) = q.after_id {
            sql.push_str(" AND id > ?");
            params.push(after.into());
        }
        sql.push_str(" ORDER BY id DESC LIMIT ?");
        params.push((q.limit as i64).into());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), crate::map_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(LogError::from)
    }

    pub fn agent_usage_stats(&self) -> Result<Vec<AgentUsageStats>, LogError> {
        let mut stmt = self.conn.prepare(
            "SELECT agent, COUNT(*) AS n, MAX(ts) AS last_ts, \
             SUM(LENGTH(args_redacted) / 4) AS est \
             FROM events GROUP BY agent ORDER BY n DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AgentUsageStats {
                agent: row.get(0)?,
                events: row.get(1)?,
                last_ts: row.get(2)?,
                est_tokens: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(LogError::from)
    }

    /// ของที่ agent เคยใช้จริง — ใช้ตอนย้ายไปโหมด lazy (`agent lazy`)
    /// `mcp_list` ถูก log เป็น kind=connect แต่ target คือคำค้น จึงต้องตัดออกด้วย tool
    pub fn resources_used_by(&self, agent: &str) -> Result<UsedResources, LogError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT kind, target FROM events \
             WHERE agent = ?1 AND decision IN ('allow', 'ask_approved') \
             AND kind IN ('skill_load', 'skill_file', 'connect', 'call') \
             AND tool != 'mcp_list' AND target != ''",
        )?;
        let rows = stmt.query_map([agent], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut used = UsedResources::default();
        for row in rows {
            let (kind, target) = row?;
            if kind.starts_with("skill_") {
                used.skills.insert(target);
            } else {
                used.mcp_servers.insert(target);
            }
        }
        Ok(used)
    }

    /// จำนวน event ต่อช่วง id (ล่าสุด) — ใช้ sparkline ใน TUI
    pub fn activity_buckets(&self, bucket_count: usize) -> Result<Vec<u64>, LogError> {
        let bucket_count = bucket_count.clamp(8, 64);
        let max_id: i64 = self.max_event_id()?;
        if max_id == 0 {
            return Ok(vec![0; bucket_count]);
        }
        let window: i64 = 500;
        let min_id = (max_id - window).max(1);
        let span = (max_id - min_id + 1).max(1);
        let mut buckets = vec![0u64; bucket_count];
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM events WHERE id >= ?1 AND id <= ?2 ORDER BY id")?;
        let rows = stmt.query_map([min_id, max_id], |row| row.get::<_, i64>(0))?;
        for id in rows {
            let id = id?;
            let idx = ((id - min_id) as u64 * bucket_count as u64 / span as u64) as usize;
            if let Some(slot) = buckets.get_mut(idx.min(bucket_count - 1)) {
                *slot += 1;
            }
        }
        Ok(buckets)
    }

    pub fn stats_by_agent(&self) -> Result<Vec<AgentStats>, LogError> {
        let mut stmt = self.conn.prepare(
            "SELECT agent, COUNT(*) AS n, MAX(ts) AS last_ts \
             FROM events GROUP BY agent ORDER BY n DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(AgentStats {
                agent: row.get(0)?,
                events: row.get(1)?,
                last_ts: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(LogError::from)
    }

    pub fn export_jsonl(&self, q: &LogQuery, mut writer: impl Write) -> Result<usize, LogError> {
        let rows = self.query(q)?;
        for row in rows.iter().rev() {
            let line = JsonlEvent {
                id: row.id,
                ts: &row.ts,
                agent: &row.agent,
                session_id: &row.session_id,
                kind: row.kind.as_str(),
                target: &row.target,
                tool: &row.tool,
                args: &row.args,
                decision: row.decision.as_ref().map(|d| d.as_str()),
                result_status: row.result_status.as_deref(),
                duration_ms: row.duration_ms,
                error: row.error.as_deref(),
            };
            serde_json::to_writer(&mut writer, &line).map_err(LogError::Json)?;
            writer.write_all(b"\n").map_err(LogError::Io)?;
        }
        Ok(rows.len())
    }
}

#[cfg(test)]
mod used_tests {
    use crate::{Decision, Event, EventKind, Logger};

    fn ev(agent: &str, kind: EventKind, target: &str, tool: &str, d: Decision) -> Event {
        Event {
            agent: agent.into(),
            session_id: "s".into(),
            kind,
            target: target.into(),
            tool: tool.into(),
            args: serde_json::json!({}),
            decision: Some(d),
            result_status: Some("ok".into()),
            duration_ms: Some(1),
            error: None,
        }
    }

    #[test]
    fn resources_used_by_collects_allowed_only() {
        let path = std::env::temp_dir().join(format!("mg-used-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let log = Logger::open(&path).unwrap();
        use Decision::*;
        use EventKind::*;
        for e in [
            ev("a", SkillLoad, "pdf", "skills_load", Allow),
            ev("a", SkillFile, "pdf", "skills_read_file", Allow),
            ev("a", SkillLoad, "secret", "skills_load", Deny),
            ev("a", SkillLoad, "asked", "skills_load", AskApproved),
            ev("a", Connect, "github", "connect", Allow),
            ev("a", Call, "figma", "call", Allow),
            ev("a", Connect, "design", "mcp_list", Allow),
            ev("a", SkillSearch, "pdf stuff", "skills_search", Allow),
            ev("b", SkillLoad, "other", "skills_load", Allow),
        ] {
            log.write(&e).unwrap();
        }
        let used = log.resources_used_by("a").unwrap();
        assert_eq!(
            used.skills.into_iter().collect::<Vec<_>>(),
            vec!["asked", "pdf"]
        );
        assert_eq!(
            used.mcp_servers.into_iter().collect::<Vec<_>>(),
            vec!["figma", "github"]
        );
        let _ = std::fs::remove_file(&path);
    }
}
