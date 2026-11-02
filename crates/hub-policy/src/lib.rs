//! ตัวตัดสินสิทธิ์: default deny, deny ชนะ allow, token ต่อ agent, โหมด ask

mod agents;
mod apply;
mod approve;
mod editor;
mod evaluate;
mod matchers;
mod types;

pub use agents::{AgentError, AgentRecord, AgentStore, hash_token};
pub use apply::{ApplyError, ApprovalApplied, apply_approval};
pub use approve::{
    ApprovalOutcome, ApprovalStore, ApproveError, GrantScope, PendingApproval, grant_for,
};
pub use editor::{Grant, PolicyEditor};
pub use evaluate::PolicyEngine;
pub use types::{Effect, Verdict};

use hub_core::policy_path;
use std::fs;
use std::path::Path;

#[derive(thiserror::Error, Debug)]
pub enum PolicyError {
    #[error("failed to read policy: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid policy toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid policy after edit: {0}")]
    Invalid(String),
}

/// โหลด policy จาก disk — ไม่มีไฟล์ = default deny ทุก agent
pub fn load_policy_from_disk() -> Result<PolicyEngine, PolicyError> {
    let path = policy_path();
    if !path.is_file() {
        return Ok(PolicyEngine::new(types::PolicyDocument::default()));
    }
    let text = fs::read_to_string(&path)?;
    Ok(PolicyEngine::from_toml(&text)?)
}

pub fn ensure_default_policy(path: &Path) -> Result<(), PolicyError> {
    if path.is_file() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let sample = r#"# Mali Gate policy — default deny; deny ชนะ allow เสมอ
# สร้าง agent: mali_gate agent add cursor

[shared]
skills = []
mcp = []

[agents.cursor]
default = "ask"

[agents."*"]
default = "deny"
"#;
    fs::write(path, sample)?;
    Ok(())
}
