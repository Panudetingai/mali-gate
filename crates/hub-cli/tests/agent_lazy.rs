//! `mali_gate agent lazy <id>`: ย้ายของที่เคยใช้เข้า [shared] แล้วตั้ง default = "ask"

mod common;

use std::process::Command;

use common::TempHome;
use hub_log::{Decision, Event, EventKind, Logger};
use hub_policy::{Effect, PolicyEngine, Verdict};

fn run(home: &TempHome, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mali_gate"))
        .args(args)
        .env("MALI_GATE_HOME", home.path())
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn setup(name: &str) -> TempHome {
    let home = TempHome::new(name);
    home.write("policy.toml", "# mine\n[agents.cc]\ndefault = \"allow\"\n");
    let log = Logger::open(&home.db()).unwrap();
    for (kind, target, tool) in [
        (EventKind::SkillLoad, "pdf", "skills_load"),
        (EventKind::Connect, "github", "connect"),
    ] {
        log.write(&Event {
            agent: "cc".into(),
            session_id: "s".into(),
            kind,
            target: target.into(),
            tool: tool.into(),
            args: serde_json::json!({}),
            decision: Some(Decision::Allow),
            result_status: Some("ok".into()),
            duration_ms: Some(1),
            error: None,
        })
        .unwrap();
    }
    home
}

fn policy(home: &TempHome) -> String {
    std::fs::read_to_string(home.path().join("policy.toml")).unwrap()
}

#[test]
fn agent_lazy_moves_used_into_shared_and_sets_ask() {
    let home = setup("lazy");
    let (ok, out) = run(&home, &["agent", "lazy", "cc"]);
    assert!(ok, "{out}");
    assert!(out.contains("pdf") && out.contains("github"), "{out}");
    let text = policy(&home);
    assert!(text.contains("# mine"), "{text}");
    let e = PolicyEngine::from_toml(&text).unwrap();
    assert_eq!(e.agent_default("cc"), Effect::Ask);
    assert_eq!(e.evaluate_skill("cc", "pdf"), Verdict::Allow);
    assert_eq!(e.evaluate_mcp_server("cc", "github"), Verdict::Allow);
    assert_eq!(e.evaluate_skill("cc", "never-used"), Verdict::Ask);
}

#[test]
fn agent_lazy_dry_run_writes_nothing() {
    let home = setup("dry");
    let before = policy(&home);
    let (ok, out) = run(&home, &["agent", "lazy", "cc", "--dry-run"]);
    assert!(ok, "{out}");
    assert!(out.contains("pdf") && out.contains("github"), "{out}");
    assert_eq!(policy(&home), before);
}

#[test]
fn agent_lazy_strips_allow_star() {
    let home = setup("star");
    home.write(
        "policy.toml",
        "[agents.cc]\ndefault = \"allow\"\nskills = { allow = [\"*\"], deny = [] }\n",
    );
    let (ok, out) = run(&home, &["agent", "lazy", "cc"]);
    assert!(ok, "{out}");
    let e = PolicyEngine::from_toml(&policy(&home)).unwrap();
    assert_eq!(e.evaluate_skill("cc", "never-used"), Verdict::Ask);
    assert_eq!(e.evaluate_skill("cc", "pdf"), Verdict::Allow);
    assert!(!e.sees_everything("cc"));
}
