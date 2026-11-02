//! `approve resolve --shared|--agent|--once|--no|--deny` และ `skills grant/revoke` ผ่าน binary จริง

mod common;

use std::process::Command;

use common::TempHome;
use hub_policy::{ApprovalStore, PolicyEngine, Verdict};

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

fn engine(home: &TempHome) -> PolicyEngine {
    PolicyEngine::from_toml(&std::fs::read_to_string(home.path().join("policy.toml")).unwrap())
        .unwrap()
}

fn ask(home: &TempHome, kind: &str, target: &str) -> i64 {
    ApprovalStore::open(&home.db())
        .unwrap()
        .insert("a", kind, target, kind, &serde_json::json!({}))
        .unwrap()
}

fn setup(name: &str) -> TempHome {
    let home = TempHome::new(name);
    home.write("policy.toml", "[agents.a]\ndefault = \"ask\"\n");
    home
}

#[test]
fn resolve_without_flag_means_shared() {
    let home = setup("noflag");
    let id = ask(&home, "skill_load", "pdf");
    let (ok, out) = run(&home, &["approve", "resolve", &id.to_string()]);
    assert!(ok, "{out}");
    assert!(out.contains("shared"), "{out}");
    assert_eq!(engine(&home).evaluate_skill("b", "pdf"), Verdict::Allow);
}

#[test]
fn resolve_agent_and_conflicting_flags() {
    let home = setup("agent");
    let id = ask(&home, "connect", "github");
    let (ok, out) = run(&home, &["approve", "resolve", &id.to_string(), "--agent"]);
    assert!(ok, "{out}");
    assert_eq!(
        engine(&home).evaluate_mcp_server("a", "github"),
        Verdict::Allow
    );

    let id2 = ask(&home, "connect", "x");
    let (ok, _) = run(
        &home,
        &["approve", "resolve", &id2.to_string(), "--shared", "--no"],
    );
    assert!(!ok, "conflicting flags must be rejected");
}

#[test]
fn legacy_yes_still_works() {
    let home = setup("yes");
    let id = ask(&home, "skill_load", "pdf");
    let (ok, out) = run(&home, &["approve", "resolve", &id.to_string(), "--yes"]);
    assert!(ok, "{out}");
    assert_eq!(engine(&home).evaluate_skill("b", "pdf"), Verdict::Allow);
}

#[test]
fn list_shows_scope_hint() {
    let home = setup("list");
    ask(&home, "skill_load", "pdf");
    let (ok, out) = run(&home, &["approve", "list"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("--shared") && out.contains("skill_load"),
        "{out}"
    );
}

#[test]
fn skills_grant_and_revoke() {
    let home = setup("grant");
    let (ok, out) = run(&home, &["skills", "grant", "pdf", "--agent", "a"]);
    assert!(ok, "{out}");
    assert_eq!(engine(&home).evaluate_skill("a", "pdf"), Verdict::Allow);
    let (ok, out) = run(&home, &["skills", "revoke", "pdf", "--agent", "a"]);
    assert!(ok, "{out}");
    assert_eq!(engine(&home).evaluate_skill("a", "pdf"), Verdict::Ask);

    let (ok, _) = run(&home, &["skills", "grant", "pdf"]);
    assert!(ok);
    assert_eq!(engine(&home).evaluate_skill("zz", "pdf"), Verdict::Allow);
}
