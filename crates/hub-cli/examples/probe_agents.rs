//! จำลองหลาย CLI เรียก skills ผ่าน MCP stdio — สร้าง log ให้ดูใน `mali_gate logs --follow`

use std::path::PathBuf;

use hub_connect::StdioMcpClient;
use hub_core::probe_agents_config_path;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
struct ProbeFile {
    #[allow(dead_code)]
    workspace: PathBuf,
    agent: Vec<ProbeAgent>,
}

#[derive(Debug, Deserialize)]
struct ProbeAgent {
    id: String,
    token: String,
}

fn default_mali_gate_exe() -> PathBuf {
    PathBuf::from(r"D:\cargo-target\skillhub\debug\mali_gate.exe")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = probe_agents_config_path().ok_or_else(|| {
        anyhow::anyhow!(
            "probe-agents.toml not found — copy to {} or set MALI_GATE_PROBE_FILE",
            hub_core::user_data_root()
                .join("probe-agents.toml")
                .display()
        )
    })?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let probe: ProbeFile = toml::from_str(&text)?;

    let exe = std::env::var("MALI_GATE_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_mali_gate_exe());
    if !exe.is_file() {
        anyhow::bail!(
            "mali_gate binary not found at {} — build with `cargo build -p hub-cli`",
            exe.display()
        );
    }
    let command = exe.to_string_lossy().into_owned();
    let args = vec!["serve".to_string()];

    for agent in &probe.agent {
        println!("--- agent `{}` → skills_search + skills_load", agent.id);
        let mut env = vec![("MALI_GATE_TOKEN".to_string(), agent.token.clone())];
        for key in ["MALI_GATE_HOME", "SKILLHUB_HOME"] {
            if let Ok(value) = std::env::var(key) {
                env.push((key.to_string(), value));
            }
        }
        let client = StdioMcpClient::connect(&command, &args, &env).await?;

        let query = if agent.id == "codex" { "dice" } else { "skill" };
        let search = client
            .call_tool("skills_search", json!({ "query": query }))
            .await?;
        println!("  skills_search: is_error={:?}", search.is_error);

        let skill = if agent.id == "codex" {
            "roll-dice"
        } else {
            "agent-skills-overview"
        };
        let load = client
            .call_tool("skills_load", json!({ "name": skill }))
            .await?;
        println!("  skills_load({skill}): is_error={:?}", load.is_error);

        client.shutdown().await?;
    }

    println!("\nDone — check `mali_gate logs --limit 30` or `logs --follow`");
    Ok(())
}
