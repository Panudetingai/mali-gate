use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Deny,
    Ask,
}

impl Effect {
    pub fn as_str(self) -> &'static str {
        match self {
            Effect::Allow => "allow",
            Effect::Deny => "deny",
            Effect::Ask => "ask",
        }
    }

    pub fn parse_str(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "allow" => Some(Effect::Allow),
            "deny" => Some(Effect::Deny),
            "ask" => Some(Effect::Ask),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListRules {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

pub type McpServerPolicy = McpToolRules;

/// `mcp.<server>` รับได้ทั้งตาราง `{ allow_tools, deny_tools, mode }` และแบบย่อ `"ask"`
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(from = "McpRulesRepr")]
pub struct McpToolRules {
    #[serde(default)]
    pub allow_tools: Vec<String>,
    #[serde(default)]
    pub deny_tools: Vec<String>,
    #[serde(default)]
    pub mode: Option<Effect>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum McpRulesRepr {
    Mode(Effect),
    Table {
        #[serde(default)]
        allow_tools: Vec<String>,
        #[serde(default)]
        deny_tools: Vec<String>,
        #[serde(default)]
        mode: Option<Effect>,
    },
}

impl From<McpRulesRepr> for McpToolRules {
    fn from(r: McpRulesRepr) -> Self {
        match r {
            McpRulesRepr::Mode(m) => Self {
                mode: Some(m),
                ..Self::default()
            },
            McpRulesRepr::Table {
                allow_tools,
                deny_tools,
                mode,
            } => Self {
                allow_tools,
                deny_tools,
                mode,
            },
        }
    }
}

/// `[shared]` — ทุก agent ได้ของในส่วนนี้ (ยกเว้นถูก deny ที่ระดับ agent)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SharedPolicy {
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub mcp: Vec<String>,
    #[serde(default)]
    pub mcp_tools: std::collections::HashMap<String, McpToolRules>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentPolicy {
    #[serde(default)]
    pub default: Option<Effect>,
    #[serde(default)]
    pub skills: Option<ListRules>,
    #[serde(default)]
    pub mcp: std::collections::HashMap<String, McpServerPolicy>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PolicyDocument {
    #[serde(default)]
    pub shared: SharedPolicy,
    #[serde(default)]
    pub agents: std::collections::HashMap<String, AgentPolicy>,
}
