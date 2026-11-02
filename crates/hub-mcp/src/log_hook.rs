//! บันทึก meta-tool calls ลง `events`

use std::sync::{Arc, Mutex};
use std::time::Instant;

use hub_log::{Decision, Event, EventKind, Logger};
use serde_json::Value;

#[derive(Clone)]
pub(crate) struct EventLog {
    inner: Arc<Mutex<Logger>>,
    agent: String,
    session_id: String,
}

impl EventLog {
    pub(crate) fn open(logger: Arc<Mutex<Logger>>, agent: &str) -> Self {
        Self {
            inner: logger,
            agent: agent.to_string(),
            session_id: session_label(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &self,
        kind: EventKind,
        target: String,
        tool: &str,
        args: Value,
        started: Instant,
        decision: Decision,
        result: &Result<String, String>,
    ) {
        let event = Event {
            agent: self.agent.clone(),
            session_id: self.session_id.clone(),
            kind,
            target,
            tool: tool.into(),
            args,
            decision: Some(decision),
            result_status: Some(if result.is_ok() {
                "ok".into()
            } else {
                "error".into()
            }),
            error: result.as_ref().err().cloned(),
            duration_ms: Some(started.elapsed().as_millis() as i64),
        };
        if let Ok(guard) = self.inner.lock() {
            let _ = guard.write(&event);
        }
    }
}

fn session_label() -> String {
    std::env::var("MALI_GATE_SESSION").unwrap_or_else(|_| format!("pid-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hub_log::Logger;
    use serde_json::json;

    fn tmp_db(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hub-mcp-log-{tag}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn record_persists_skill_search() {
        let path = tmp_db("search");
        let logger = Arc::new(Mutex::new(Logger::open(&path).unwrap()));
        let log = EventLog::open(logger.clone(), "test");
        let started = Instant::now();
        let ok: Result<String, String> = Ok("[]".into());
        log.record(
            EventKind::SkillSearch,
            "pdf".into(),
            "skills_search",
            json!({ "query": "pdf" }),
            started,
            Decision::Allow,
            &ok,
        );
        let rows = logger.lock().unwrap().last_n(5).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tool, "skills_search");
    }
}
