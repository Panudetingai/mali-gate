use hub_log::{Decision, Event, EventKind, LogQuery, Logger};
use serde_json::json;

fn tmp_db(name: &str) -> std::path::PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "hub-log-query-{name}-{}-{}.db",
        std::process::id(),
        unique
    ))
}

#[test]
fn query_filters_agent_and_kind() {
    let path = tmp_db("filter");
    let logger = Logger::open(&path).unwrap();
    logger
        .write(&Event {
            agent: "a1".into(),
            session_id: "s".into(),
            kind: EventKind::SkillSearch,
            target: "t".into(),
            tool: "skills_search".into(),
            args: json!({}),
            decision: Some(Decision::Allow),
            result_status: Some("ok".into()),
            duration_ms: None,
            error: None,
        })
        .unwrap();
    logger
        .write(&Event {
            agent: "a2".into(),
            session_id: "s".into(),
            kind: EventKind::Call,
            target: "mcp".into(),
            tool: "x".into(),
            args: json!({}),
            decision: None,
            result_status: None,
            duration_ms: None,
            error: None,
        })
        .unwrap();

    let q = LogQuery {
        limit: 10,
        agent: Some("a1".into()),
        kind: Some(EventKind::SkillSearch),
        decision: None,
        after_id: None,
    };
    let rows = logger.query(&q).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent, "a1");
}

#[test]
fn export_jsonl_writes_lines() {
    let path = tmp_db("jsonl");
    let logger = Logger::open(&path).unwrap();
    logger
        .write(&Event {
            agent: "ex".into(),
            session_id: "s".into(),
            kind: EventKind::SkillLoad,
            target: "skill".into(),
            tool: "skills_load".into(),
            args: json!({ "name": "skill" }),
            decision: Some(Decision::Allow),
            result_status: Some("ok".into()),
            duration_ms: Some(1),
            error: None,
        })
        .unwrap();

    let mut buf = Vec::new();
    let n = logger
        .export_jsonl(
            &LogQuery {
                limit: 5,
                ..LogQuery::default()
            },
            &mut buf,
        )
        .unwrap();
    assert_eq!(n, 1);
    let text = String::from_utf8(buf).unwrap();
    assert!(text.contains("\"agent\":\"ex\""));
    assert!(text.ends_with('\n'));
}
