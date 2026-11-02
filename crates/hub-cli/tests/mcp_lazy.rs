//! Agent เห็นเฉพาะของที่มีสิทธิ์ (spec §2.3) — คุยกับ `mali_gate serve` จริง

mod common;

use common::{McpClient, TempHome, names};
use serde_json::json;

const POLICY: &str = r#"
[shared]
skills = ["pdf-master"]
mcp = ["alpha"]

[agents.a]
default = "ask"
skills = { deny = ["db-backup"] }

[agents.z]
default = "deny"
"#;

const CONFIG: &str = r#"
[mcp.servers.alpha]
transport = "stdio"
command = "definitely-not-a-real-command-mg"
description = "Alpha design tools"

[mcp.servers.beta]
transport = "stdio"
command = "definitely-not-a-real-command-mg"
"#;

fn setup(name: &str) -> (TempHome, String, String) {
    let home = TempHome::new(name);
    home.add_fixture_skills(&["pdf-master", "pdf-summary", "db-backup", "weekly-report"]);
    home.write("policy.toml", POLICY);
    home.write("config.toml", CONFIG);
    let a = home.add_agent("a");
    let z = home.add_agent("z");
    (home, a, z)
}

#[test]
fn search_splits_allowed_and_requestable() {
    let (home, a, _) = setup("split");
    let mut c = McpClient::spawn(&home, &a);
    let out = c.call_json("skills_search", json!({"query": "PDF"}));
    assert_eq!(names(&out["results"]), vec!["pdf-master"]);
    assert_eq!(names(&out["requestable"]), vec!["pdf-summary"]);
}

#[test]
fn denied_skill_is_hidden_everywhere() {
    let (home, a, _) = setup("deny");
    let mut c = McpClient::spawn(&home, &a);
    let out = c.call_json("skills_search", json!({"query": "สัปดาห์"}));
    let text = out.to_string();
    assert!(!text.contains("db-backup"), "{text}");
    assert_eq!(names(&out["results"]), Vec::<String>::new());
    assert_eq!(names(&out["requestable"]), vec!["weekly-report"]);
}

#[test]
fn default_deny_agent_sees_only_shared_and_no_bodies() {
    let (home, _, z) = setup("zdeny");
    let body_marker = std::fs::read_to_string(home.path().join("skills/pdf-summary/SKILL.md"))
        .unwrap()
        .lines()
        .rev()
        .find(|l| l.trim().len() > 10)
        .unwrap()
        .to_string();
    let mut c = McpClient::spawn(&home, &z);
    let out = c.call_json("skills_search", json!({"query": "PDF"}));
    assert_eq!(names(&out["results"]), vec!["pdf-master"]);
    assert_eq!(names(&out["requestable"]), Vec::<String>::new());
    assert!(!out.to_string().contains(&body_marker));
}

#[test]
fn mcp_list_filters_by_access() {
    let (home, a, z) = setup("mcplist");
    let mut ca = McpClient::spawn(&home, &a);
    let out = ca.call_json("mcp_list", json!({}));
    let servers = out["servers"].as_array().unwrap();
    let find = |n: &str| servers.iter().find(|s| s["name"] == n).cloned();
    assert_eq!(find("alpha").unwrap()["access"], "allow");
    assert_eq!(find("alpha").unwrap()["description"], "Alpha design tools");
    assert_eq!(find("beta").unwrap()["access"], "ask");

    let filtered = ca.call_json("mcp_list", json!({"query": "design"}));
    assert_eq!(names(&filtered["servers"]), vec!["alpha"]);

    let mut cz = McpClient::spawn(&home, &z);
    let out = cz.call_json("mcp_list", json!({}));
    assert_eq!(names(&out["servers"]), vec!["alpha"]);
}

#[test]
fn handshake_has_instructions() {
    let (home, a, _) = setup("instr");
    let c = McpClient::spawn(&home, &a);
    let text = c.init["instructions"].as_str().unwrap_or("");
    assert!(
        text.contains("skills_search") && text.contains("mcp_list"),
        "{text}"
    );
}

#[test]
fn tools_list_has_eight_meta_tools() {
    let (home, a, _) = setup("tools");
    let mut c = McpClient::spawn(&home, &a);
    let mut got = c.tool_names();
    got.sort();
    let mut want = vec![
        "call",
        "connect",
        "mcp_list",
        "release",
        "skills_install",
        "skills_load",
        "skills_read_file",
        "skills_search",
    ];
    want.sort();
    assert_eq!(got, want);
}

#[test]
fn unknown_or_wildcard_targets_are_denied_without_asking() {
    let (home, a, _) = setup("wild");
    let mut c = McpClient::spawn(&home, &a);
    for (tool, args) in [
        ("connect", json!({"server": "*"})),
        ("connect", json!({"server": "no-such-server"})),
        ("skills_load", json!({"name": "pdf*"})),
        ("skills_load", json!({"name": "no-such-skill"})),
        ("call", json!({"server": "*", "tool": "x", "args": {}})),
    ] {
        let (text, is_err) = c.call_raw(tool, args);
        assert!(is_err, "{tool} should fail: {text}");
    }
    let pending = hub_policy::ApprovalStore::open(&home.db())
        .unwrap()
        .list_open()
        .unwrap();
    assert!(
        pending.is_empty(),
        "no approval must be created: {pending:?}"
    );
}
