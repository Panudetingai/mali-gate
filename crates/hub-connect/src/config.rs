use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use hub_core::config_path;

use crate::ConnectError;

#[derive(Debug, Clone, Deserialize)]
pub struct HubConfig {
    #[serde(default)]
    pub hub: HubSection,
    #[serde(default)]
    pub mcp: McpSection,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HubSection {
    #[serde(default = "default_idle_secs")]
    pub idle_timeout_secs: u64,
}

fn default_idle_secs() -> u64 {
    300
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpSection {
    #[serde(default)]
    pub servers: HashMap<String, McpServerSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpServerSpec {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        description: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        bearer_env: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    Rest {
        openapi: String,
        base_url: String,
        #[serde(default)]
        description: Option<String>,
    },
}

impl McpServerSpec {
    /// คำอธิบายสั้นที่ผู้ใช้ตั้งไว้ (แสดงใน `mcp_list`)
    pub fn description(&self) -> Option<&str> {
        match self {
            McpServerSpec::Stdio { description, .. }
            | McpServerSpec::Http { description, .. }
            | McpServerSpec::Rest { description, .. } => description.as_deref(),
        }
    }
}

impl HubConfig {
    pub fn load_from_disk() -> Result<Self, ConnectError> {
        let path = config_path();
        if !path.is_file() {
            return Ok(Self::default_empty());
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| ConnectError::Config(e.to_string()))?;
        toml::from_str(&text).map_err(|e| ConnectError::Config(e.to_string()))
    }

    pub fn default_empty() -> Self {
        Self {
            hub: HubSection {
                idle_timeout_secs: default_idle_secs(),
            },
            mcp: McpSection {
                servers: HashMap::new(),
            },
        }
    }

    pub fn ensure_sample(path: &Path) -> Result<(), ConnectError> {
        if path.is_file() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConnectError::Config(e.to_string()))?;
        }
        let sample = r#"# Mali Gate — upstream MCP servers (lazy connect via meta-tools)

[hub]
idle_timeout_secs = 300

# ตัวอย่าง: ชี้กลับ hub เอง (ทดสอบ connect/call)
[mcp.servers.mali-gate-self]
transport = "stdio"
command = "cargo"
args = ["run", "-p", "hub-cli", "--bin", "mali_gate", "--", "serve"]

[install]
allow_local_paths = true
allow_git_hosts = ["github.com", "gitlab.com", "bitbucket.org"]

[registry]
files = []
"#;
        std::fs::write(path, sample).map_err(|e| ConnectError::Config(e.to_string()))?;
        Ok(())
    }
}

impl Default for HubConfig {
    fn default() -> Self {
        Self::default_empty()
    }
}
