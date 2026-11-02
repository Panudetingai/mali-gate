use hub_core::{db_path, policy_path};
use hub_policy::{ApprovalOutcome, ApprovalStore, Grant, GrantScope, PolicyEditor, apply_approval};

use crate::Palette;

pub fn run_approve_list(paint: &Palette) -> anyhow::Result<()> {
    let store = ApprovalStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rows = store.list_open().map_err(|e| anyhow::anyhow!("{e}"))?;
    if rows.is_empty() {
        println!("  {}", paint.soft("no pending approvals"));
        return Ok(());
    }
    println!("  {}", paint.heading("pending approvals"));
    for row in rows {
        println!(
            "  {}  {}  {}  {}  {}",
            paint.heading(&row.id.to_string()),
            paint.quiet(&row.agent),
            paint.heading(&row.kind),
            paint.quiet(&row.target),
            paint.soft(&row.summary)
        );
    }
    println!(
        "  {}",
        paint.quiet(
            "answer: mali_gate approve resolve <id> --shared | --agent | --once | --no | --deny"
        )
    );
    Ok(())
}

/// ตอบคำขอ: เขียนสิทธิ์ตาม scope แล้ว resolve
/// `remember = false` ใช้กับ `--no` (ปฏิเสธครั้งนี้ ไม่เขียน deny ถาวร)
pub fn run_approve(
    id: i64,
    scope: GrantScope,
    remember: bool,
    paint: &Palette,
) -> anyhow::Result<()> {
    let store = ApprovalStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let applied = if !remember {
        store
            .resolve(id, ApprovalOutcome::Denied)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        "denied (this time)".to_string()
    } else {
        let r = apply_approval(&store, &policy_path(), id, scope)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let what = match &r.grant {
            Some(Grant::Skill(n)) => format!("skill `{n}`"),
            Some(Grant::McpServer(n)) => format!("mcp `{n}`"),
            None => "request".into(),
        };
        match scope {
            GrantScope::Shared => format!("{what} → shared (every agent)"),
            GrantScope::Agent => format!("{what} → agent `{}`", r.agent),
            GrantScope::Once => format!("{what} → allowed once"),
            GrantScope::Deny => format!("{what} → denied for `{}`", r.agent),
        }
    };
    println!("  {}", paint.heading(&format!("approval #{id}: {applied}")));
    Ok(())
}

/// `skills grant|revoke <name> [--agent A]`
pub fn run_grant(
    name: &str,
    agent: Option<&str>,
    grant: bool,
    paint: &Palette,
) -> anyhow::Result<()> {
    let mut ed = PolicyEditor::open(&policy_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let g = Grant::Skill(name.into());
    let changed = match (grant, agent) {
        (true, None) => ed.grant_shared(&g),
        (true, Some(a)) => ed.grant_agent(a, &g),
        (false, a) => ed.revoke(a, &g),
    };
    if changed {
        ed.save().map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    let scope = agent.map_or("shared".to_string(), |a| format!("agent `{a}`"));
    let verb = if grant { "granted to" } else { "revoked from" };
    let note = if changed { "" } else { " (no change)" };
    println!(
        "  {}",
        paint.heading(&format!("skill `{name}` {verb} {scope}{note}"))
    );
    Ok(())
}
