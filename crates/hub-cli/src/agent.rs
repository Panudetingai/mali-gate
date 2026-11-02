use hub_core::{data_root, db_path, policy_path};
use hub_log::Logger;
use hub_policy::{AgentStore, Effect, Grant, PolicyEditor, ensure_default_policy};

use crate::Palette;

pub fn run_agent_add(id: &str, label: Option<&str>, paint: &Palette) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_root())?;
    ensure_default_policy(&policy_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let store = AgentStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let label = label.unwrap_or(id);
    let (token, rec) = store.add(id, label).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "  {}",
        paint.heading(&format!("agent `{}` created", rec.id))
    );
    println!("  {}", paint.quiet(&format!("label: {}", rec.label)));
    println!(
        "  {}",
        paint.heading("token (shown once — store in MCP env MALI_GATE_TOKEN)")
    );
    println!("  {token}");
    println!();
    println!(
        "  {}",
        paint.soft("Add to .cursor/mcp.json → mali-gate → env.MALI_GATE_TOKEN")
    );
    Ok(())
}

pub fn run_agent_list(paint: &Palette) -> anyhow::Result<()> {
    let store = AgentStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rows = store.list().map_err(|e| anyhow::anyhow!("{e}"))?;
    if rows.is_empty() {
        println!(
            "  {}",
            paint.soft("no agents — run `mali_gate agent add cursor`")
        );
        return Ok(());
    }
    println!("  {}", paint.heading("agents"));
    for row in rows {
        println!(
            "  {}  {}",
            paint.heading(&row.id),
            paint.quiet(&format!("{} · {}", row.label, row.created_at))
        );
    }
    Ok(())
}

pub fn run_agent_revoke(id: &str, paint: &Palette) -> anyhow::Result<()> {
    let store = AgentStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    store.revoke(id).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("  {}", paint.heading(&format!("revoked agent `{id}`")));
    Ok(())
}

/// `agent lazy <id>`: ย้าย skill/MCP ที่ agent เคยใช้จริงเข้า `[shared]` แล้วตั้ง `default = "ask"`
/// (agent ยังทำงานเดิมได้ แต่ของใหม่ต้องขอก่อน)
pub fn run_agent_lazy(id: &str, dry_run: bool, paint: &Palette) -> anyhow::Result<()> {
    let used = Logger::open(&db_path())
        .and_then(|l| l.resources_used_by(id))
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut ed = PolicyEditor::open(&policy_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    for s in &used.skills {
        ed.grant_shared(&Grant::Skill(s.clone()));
    }
    for m in &used.mcp_servers {
        ed.grant_shared(&Grant::McpServer(m.clone()));
    }
    let default_changed = ed.set_default(id, Effect::Ask);
    // `allow = ["*"]` ทำให้ยังเห็นทุก skill แม้ default = ask — ต้องตัดออกด้วย
    let stripped = ed.remove_agent_wildcards(id);
    let list = |set: &std::collections::BTreeSet<String>| {
        if set.is_empty() {
            "—".to_string()
        } else {
            set.iter().cloned().collect::<Vec<_>>().join(", ")
        }
    };
    println!(
        "  {}",
        paint.heading(&format!("agent `{id}` → lazy (default = \"ask\")"))
    );
    println!("  {} {}", paint.soft("shared skills:"), list(&used.skills));
    println!(
        "  {} {}",
        paint.soft("shared mcp:   "),
        list(&used.mcp_servers)
    );
    if !stripped.is_empty() {
        println!("  {} {}", paint.soft("removed:      "), stripped.join(", "));
    }
    if dry_run {
        println!("  {}", paint.quiet("dry run — nothing written"));
        return Ok(());
    }
    ed.save().map_err(|e| anyhow::anyhow!("{e}"))?;
    if !default_changed {
        println!("  {}", paint.quiet("default was already \"ask\""));
    }
    Ok(())
}
