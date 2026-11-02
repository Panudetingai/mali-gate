// Integration test งาน 3: logger เขียน events ลง SQLite + redact ก่อนเขียน
use hub_log::{Decision, Event, EventKind, Logger};
use serde_json::json;

/// สร้างพาธฐานข้อมูลชั่วคราวแบบไม่ซ้ำกันระหว่างเทสต์
fn tmp_db(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    dir.join(format!("hub-log-{name}-{unique}{}.db", std::process::id()))
}

/// สร้าง event ตัวอย่างย่อสำหรับเทสต์
fn sample_event() -> Event {
    Event {
        agent: "claude-code".into(),
        session_id: "sess-1".into(),
        kind: EventKind::SkillSearch,
        target: "skills".into(),
        tool: "skills_search".into(),
        args: json!({ "query": "pdf" }),
        decision: Some(Decision::Allow),
        result_status: Some("ok".into()),
        duration_ms: Some(12),
        error: None,
    }
}

/// เขียนแล้วอ่านคืน ต้องได้ข้อมูลครบเหมือนเดิม (ยกเว้น ts ที่ hub ใส่เอง)
#[test]
fn writes_and_reads_back_events() {
    let path = tmp_db("readback");
    let logger = Logger::open(&path).unwrap();
    let id = logger.write(&sample_event()).unwrap();

    let rows = logger.last_n(10).unwrap();
    assert_eq!(rows.len(), 1);
    let ev = &rows[0];
    assert_eq!(ev.id, id);
    assert_eq!(ev.agent, "claude-code");
    assert_eq!(ev.session_id, "sess-1");
    assert!(matches!(ev.kind, EventKind::SkillSearch));
    assert_eq!(ev.tool, "skills_search");
    assert_eq!(ev.args["query"], "pdf");
    assert!(matches!(ev.decision, Some(Decision::Allow)));
    assert_eq!(ev.result_status.as_deref(), Some("ok"));
    assert_eq!(ev.duration_ms, Some(12));
    // ts ต้องถูกใส่เองและเป็นสตริงไม่ว่าง (RFC3339)
    assert!(!ev.ts.is_empty());
}

/// ความลับใน args ต้องถูก redact ก่อนเขียนเสมอ — เห็น "***" ไม่เห็นค่าจริง
#[test]
fn redacts_secret_args_before_writing() {
    let path = tmp_db("redact");
    let logger = Logger::open(&path).unwrap();
    let event = Event {
        args: json!({
            "token": "abc-secret-123",
            "api_key": "sk-very-secret",
            "query": "pdf",
        }),
        ..sample_event()
    };
    logger.write(&event).unwrap();

    let rows = logger.last_n(10).unwrap();
    let stored = rows[0].args.to_string();
    assert!(!stored.contains("abc-secret-123"));
    assert!(!stored.contains("sk-very-secret"));
    assert!(stored.contains("***"));
    assert!(stored.contains("pdf"));
}

/// ฐานว่าง last_n ต้องคืนเวกเตอร์ว่าง ไม่พัง
#[test]
fn empty_db_returns_no_rows() {
    let path = tmp_db("empty");
    let logger = Logger::open(&path).unwrap();
    assert!(logger.last_n(10).unwrap().is_empty());
}

/// ฐานจัดเก็บหลาย event และ last_n(2) ต้องได้ 2 ล่าสุด (เรียงใหม่ไปเก่า)
#[test]
fn last_n_returns_newest_first() {
    let path = tmp_db("lastn");
    let logger = Logger::open(&path).unwrap();
    logger.write(&sample_event()).unwrap(); // skill_search
    logger
        .write(&Event {
            kind: EventKind::SkillLoad,
            target: "pdf-master".into(),
            tool: "skills_load".into(),
            ..sample_event()
        })
        .unwrap();
    logger
        .write(&Event {
            kind: EventKind::Call,
            target: "github".into(),
            tool: "get_issue".into(),
            ..sample_event()
        })
        .unwrap();

    let rows = logger.last_n(2).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(matches!(rows[0].kind, EventKind::Call));
    assert!(matches!(rows[1].kind, EventKind::SkillLoad));
}
