//! MCP server — meta-tools สำหรับ agent (skills, connect/call/release)

mod gate;
mod install_flow;
mod log_hook;
mod session;

use std::sync::Arc;
use std::time::Instant;

use hub_log::EventKind;
use hub_policy::Verdict;
use hub_registry::skill;
use rmcp::model::{ServerCapabilities, ServerConfig};
use rmcp::transport::stdio;
use rmcp::{
    ServerHandler, ServiceExt, handler::server::wrapper::Parameters, schemars, tool, tool_handler,
    tool_router,
};
use serde::Serialize;
use serde_json::json;

use gate::{GateOutcome, check_mcp_call, check_meta, check_skill};
use session::HubSession;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchParams {
    query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct NameParams {
    name: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ReadFileParams {
    name: String,
    path: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ConnectParams {
    server: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct CallParams {
    server: String,
    tool: String,
    #[serde(default)]
    args: serde_json::Value,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct McpListParams {
    /// optional keyword filter on server name/description
    #[serde(default)]
    query: Option<String>,
}

/// คำแนะนำที่ส่งให้ agent ตอน handshake (spec §2.3)
const INSTRUCTIONS: &str = "Mali Gate hub. Find before you load: use skills_search for skills and mcp_list for MCP servers, then skills_load / connect only what the task needs, and release when done. Items under \"requestable\" can be used — the hub will ask the user once and remember.";

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ReleaseParams {
    server: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct InstallParams {
    /// git URL or local directory path
    source: String,
}

#[derive(Clone)]
struct MaliGateMcp {
    hub: Arc<HubSession>,
}

impl MaliGateMcp {
    fn open() -> Result<Self, String> {
        Ok(Self {
            hub: Arc::new(HubSession::open()?),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn run_gated(
        &self,
        gate: GateOutcome,
        kind: EventKind,
        target: String,
        tool: &str,
        args: serde_json::Value,
        started: Instant,
        exec: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        if !gate.allowed {
            let msg = gate.error.unwrap_or_else(|| "denied".into());
            let err: Result<String, String> = Err(msg);
            self.hub.events.record(
                EventKind::PolicyDenied,
                target,
                tool,
                args,
                started,
                gate.decision,
                &err,
            );
            return err;
        }
        let result = exec();
        self.hub
            .events
            .record(kind, target, tool, args, started, gate.decision, &result);
        result
    }
}

#[derive(Serialize)]
struct SearchHitOut {
    name: String,
    description: String,
}

#[derive(Serialize)]
struct LoadOut {
    name: String,
    description: String,
    skill_md: String,
    reference_files: Vec<String>,
}

#[tool_router]
impl MaliGateMcp {
    #[tool(description = "Search skills in the Mali Gate library by task description or keywords")]
    fn skills_search(
        &self,
        Parameters(SearchParams { query }): Parameters<SearchParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args = json!({ "query": query });
        let target = query.clone();
        let gate = check_meta(
            &self.hub,
            "skills_search",
            &target,
            args.clone(),
            EventKind::SkillSearch,
        );
        self.run_gated(
            gate,
            EventKind::SkillSearch,
            target,
            "skills_search",
            args,
            started,
            || {
                let policy = self.hub.policy();
                let agent = &self.hub.agent_id;
                let open = !self.hub.strict_tokens && !policy.is_configured();
                let hits = self
                    .hub
                    .with_index(|index| index.search(&query).map_err(|e| e.to_string()))?;
                // ใช้ได้ → results; ขอได้ → requestable (ชื่อ+คำอธิบาย); deny → ซ่อน
                let mut results = Vec::new();
                let mut requestable = Vec::new();
                for h in hits {
                    let verdict = if open {
                        Verdict::Allow
                    } else {
                        policy.evaluate_skill(agent, &h.name)
                    };
                    let hit = SearchHitOut {
                        name: h.name,
                        description: h.description,
                    };
                    match verdict {
                        Verdict::Allow => results.push(hit),
                        Verdict::Ask => requestable.push(hit),
                        Verdict::Deny => {}
                    }
                }
                let mut out = json!({ "results": results, "requestable": requestable });
                let entries =
                    hub_registry::load_entries(&self.hub.registry_cfg.registry).unwrap_or_default();
                let reg_hits = hub_registry::search_registry(&entries, &query);
                if !reg_hits.is_empty() {
                    let merged = hub_registry::merge_search_json("[]", &reg_hits)
                        .map_err(|e| e.to_string())?;
                    let merged: serde_json::Value =
                        serde_json::from_str(&merged).map_err(|e| e.to_string())?;
                    out["registry_suggestions"] = merged["registry_suggestions"].clone();
                }
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            },
        )
    }

    #[tool(
        description = "Request installing a skill from git URL or path (requires user approval)"
    )]
    fn skills_install(
        &self,
        Parameters(InstallParams { source }): Parameters<InstallParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args = json!({ "source": source });
        let target = source.clone();
        let gate = check_meta(
            &self.hub,
            "skills_install",
            &target,
            args.clone(),
            EventKind::Install,
        );
        self.run_gated(
            gate,
            EventKind::Install,
            target,
            "skills_install",
            args,
            started,
            || install_flow::run_install_with_approval(&self.hub, &source),
        )
    }

    #[tool(description = "Load a skill's SKILL.md body and list files under references/")]
    fn skills_load(
        &self,
        Parameters(NameParams { name }): Parameters<NameParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args = json!({ "name": name });
        let target = name.clone();
        let gate = check_skill(&self.hub, &name, "skills_load");
        self.run_gated(
            gate,
            EventKind::SkillLoad,
            target,
            "skills_load",
            args,
            started,
            || {
                self.hub.with_index(|index| {
                    let hit = index
                        .get_by_name(&name)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("skill not found: {name}"))?;
                    let skill_md = skill::read_skill_md(&hit.dir).map_err(|e| e.to_string())?;
                    let reference_files =
                        skill::list_reference_files(&hit.dir).map_err(|e| e.to_string())?;
                    let out = LoadOut {
                        name: hit.name,
                        description: hit.description,
                        skill_md,
                        reference_files,
                    };
                    serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
                })
            },
        )
    }

    #[tool(description = "Read a file inside a skill folder (path must stay within the skill)")]
    fn skills_read_file(
        &self,
        Parameters(ReadFileParams { name, path }): Parameters<ReadFileParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args = json!({ "name": name, "path": path });
        let target = name.clone();
        let gate = check_skill(&self.hub, &name, "skills_read_file");
        self.run_gated(
            gate,
            EventKind::SkillFile,
            target,
            "skills_read_file",
            args,
            started,
            || {
                self.hub.with_index(|index| {
                    let hit = index
                        .get_by_name(&name)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("skill not found: {name}"))?;
                    skill::read_skill_file(&hit.dir, &path).map_err(|e| e.to_string())
                })
            },
        )
    }

    #[tool(
        description = "List MCP servers you can use (name, description, access). Connect only to what the task needs."
    )]
    fn mcp_list(
        &self,
        Parameters(McpListParams { query }): Parameters<McpListParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args = json!({ "query": query });
        let target = query.clone().unwrap_or_default();
        let gate = check_meta(
            &self.hub,
            "mcp_list",
            &target,
            args.clone(),
            EventKind::Connect,
        );
        self.run_gated(
            gate,
            EventKind::Connect,
            target,
            "mcp_list",
            args,
            started,
            || {
                let policy = self.hub.policy();
                let open = !self.hub.strict_tokens && !policy.is_configured();
                let needle = query.as_deref().map(str::to_lowercase);
                let servers: Vec<serde_json::Value> = self
                    .hub
                    .connectors
                    .servers()
                    .into_iter()
                    .filter(|(name, desc)| {
                        needle.as_ref().is_none_or(|q| {
                            name.to_lowercase().contains(q)
                                || desc
                                    .as_deref()
                                    .is_some_and(|d| d.to_lowercase().contains(q))
                        })
                    })
                    .filter_map(|(name, desc)| {
                        let access = if open {
                            Verdict::Allow
                        } else {
                            policy.evaluate_mcp_server(&self.hub.agent_id, &name)
                        };
                        let access = match access {
                            Verdict::Allow => "allow",
                            Verdict::Ask => "ask",
                            Verdict::Deny => return None,
                        };
                        Some(json!({
                            "name": name,
                            "description": desc,
                            "tool_count": null,
                            "access": access,
                        }))
                    })
                    .collect();
                serde_json::to_string_pretty(&json!({ "servers": servers }))
                    .map_err(|e| e.to_string())
            },
        )
    }

    #[tool(description = "Connect to a configured upstream MCP server (lazy)")]
    fn connect(
        &self,
        Parameters(ConnectParams { server }): Parameters<ConnectParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args = json!({ "server": server });
        let target = server.clone();
        let gate = check_meta(
            &self.hub,
            "connect",
            &target,
            args.clone(),
            EventKind::Connect,
        );
        self.run_gated(
            gate,
            EventKind::Connect,
            target,
            "connect",
            args,
            started,
            || {
                let tools = self
                    .hub
                    .connectors
                    .connect(&server)
                    .map_err(|e| e.to_string())?;
                let policy = self.hub.policy();
                let open = !self.hub.strict_tokens && !policy.is_configured();
                let (mut allowed, mut requestable) = (Vec::new(), Vec::new());
                for t in tools {
                    match if open {
                        Verdict::Allow
                    } else {
                        policy.evaluate_mcp_tool(&self.hub.agent_id, &server, &t)
                    } {
                        Verdict::Allow => allowed.push(t),
                        Verdict::Ask => requestable.push(t),
                        Verdict::Deny => {}
                    }
                }
                serde_json::to_string_pretty(&json!({
                    "server": server,
                    "tools": allowed,
                    "requestable_tools": requestable,
                }))
                .map_err(|e| e.to_string())
            },
        )
    }

    #[tool(description = "Call a tool on a connected upstream MCP server")]
    fn call(
        &self,
        Parameters(CallParams { server, tool, args }): Parameters<CallParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args_json = json!({ "server": server, "tool": tool, "args": args });
        let target = server.clone();
        let gate = check_mcp_call(&self.hub, &server, &tool, args_json.clone());
        self.run_gated(
            gate,
            EventKind::Call,
            target,
            "call",
            args_json,
            started,
            || {
                let result = self
                    .hub
                    .connectors
                    .call(&server, &tool, args)
                    .map_err(|e| e.to_string())?;
                serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
            },
        )
    }

    #[tool(description = "Release an upstream MCP connection")]
    fn release(
        &self,
        Parameters(ReleaseParams { server }): Parameters<ReleaseParams>,
    ) -> Result<String, String> {
        let started = Instant::now();
        let args = json!({ "server": server });
        let target = server.clone();
        let gate = check_meta(
            &self.hub,
            "release",
            &target,
            args.clone(),
            EventKind::Release,
        );
        self.run_gated(
            gate,
            EventKind::Release,
            target,
            "release",
            args,
            started,
            || {
                self.hub
                    .connectors
                    .release(&server)
                    .map_err(|e| e.to_string())?;
                Ok(json!({ "released": server }).to_string())
            },
        )
    }
}

#[tool_handler]
impl ServerHandler for MaliGateMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(INSTRUCTIONS)
    }
}

pub async fn serve_stdio() -> anyhow::Result<()> {
    let server = MaliGateMcp::open().map_err(anyhow::Error::msg)?;
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
