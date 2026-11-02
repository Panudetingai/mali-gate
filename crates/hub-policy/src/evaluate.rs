use crate::matchers::any_pattern_match;
use crate::types::{AgentPolicy, Effect, PolicyDocument, Verdict};

pub struct PolicyEngine {
    doc: PolicyDocument,
}

impl PolicyEngine {
    pub fn new(doc: PolicyDocument) -> Self {
        Self { doc }
    }

    /// policy ว่าง = default deny ทุก agent
    pub fn empty() -> Self {
        Self::new(PolicyDocument::default())
    }

    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        let doc: PolicyDocument = toml::from_str(text)?;
        Ok(Self::new(doc))
    }

    pub fn agent_ids(&self) -> impl Iterator<Item = &String> {
        self.doc.agents.keys()
    }

    pub fn is_configured(&self) -> bool {
        !self.doc.agents.is_empty()
    }

    fn agent_policy(&self, agent_id: &str) -> Option<&AgentPolicy> {
        self.doc
            .agents
            .get(agent_id)
            .or_else(|| self.doc.agents.get("*"))
    }

    /// default ของ agent: `[agents.A]` → `[agents."*"]` → Deny
    pub fn agent_default(&self, agent_id: &str) -> Effect {
        self.agent_policy(agent_id)
            .and_then(|a| a.default)
            .unwrap_or(Effect::Deny)
    }

    /// agent เห็นทุกอย่างอยู่แล้ว (`default = "allow"` หรือ `skills.allow` มี `"*"`) — ไม่ได้อยู่ในโหมด lazy
    pub fn sees_everything(&self, agent_id: &str) -> bool {
        self.agent_default(agent_id) == Effect::Allow
            || self
                .doc
                .agents
                .get(agent_id)
                .and_then(|a| a.skills.as_ref())
                .is_some_and(|r| r.allow.iter().any(|p| p == "*"))
    }

    /// ลำดับ: deny ของ agent → allow ของ agent / shared → default
    pub fn evaluate_skill(&self, agent_id: &str, skill_name: &str) -> Verdict {
        let rules = self
            .doc
            .agents
            .get(agent_id)
            .and_then(|a| a.skills.as_ref());
        if rules.is_some_and(|r| any_pattern_match(&r.deny, skill_name)) {
            return Verdict::Deny;
        }
        if rules.is_some_and(|r| any_pattern_match(&r.allow, skill_name))
            || any_pattern_match(&self.doc.shared.skills, skill_name)
        {
            return Verdict::Allow;
        }
        self.agent_default(agent_id).into()
    }

    pub fn evaluate_mcp_tool(&self, agent_id: &str, server: &str, tool: &str) -> Verdict {
        let mine = self
            .doc
            .agents
            .get(agent_id)
            .and_then(|a| a.mcp.get(server));
        let shared_rules = self.doc.shared.mcp_tools.get(server);
        if mine
            .is_some_and(|r| r.mode == Some(Effect::Deny) || any_pattern_match(&r.deny_tools, tool))
            || shared_rules.is_some_and(|r| any_pattern_match(&r.deny_tools, tool))
        {
            return Verdict::Deny;
        }
        if mine.is_some_and(|r| any_pattern_match(&r.allow_tools, tool)) {
            return Verdict::Allow;
        }
        if self.shares_server(server)
            && shared_rules
                .is_none_or(|r| r.allow_tools.is_empty() || any_pattern_match(&r.allow_tools, tool))
        {
            return Verdict::Allow;
        }
        match mine.and_then(|r| r.mode) {
            Some(mode) => mode.into(),
            None => self.agent_default(agent_id).into(),
        }
    }

    /// สิทธิ์ระดับ server (ใช้กับ `connect` / `mcp_list`)
    pub fn evaluate_mcp_server(&self, agent_id: &str, server: &str) -> Verdict {
        let mine = self
            .doc
            .agents
            .get(agent_id)
            .and_then(|a| a.mcp.get(server));
        if mine.is_some_and(|r| r.mode == Some(Effect::Deny)) {
            return Verdict::Deny;
        }
        if self.shares_server(server)
            || mine.is_some_and(|r| !r.allow_tools.is_empty() || r.mode == Some(Effect::Allow))
        {
            return Verdict::Allow;
        }
        if mine.is_some_and(|r| r.mode == Some(Effect::Ask)) {
            return Verdict::Ask;
        }
        self.agent_default(agent_id).into()
    }

    fn shares_server(&self, server: &str) -> bool {
        any_pattern_match(&self.doc.shared.mcp, server)
    }

    pub fn evaluate_meta_tool(&self, agent_id: &str, tool: &str, target: Option<&str>) -> Verdict {
        match tool {
            // คืนแค่ข้อมูลที่กรองสิทธิ์แล้ว — ถ้า ask ทุกครั้งที่ค้น agent จะติดรอ
            "skills_search" | "mcp_list" | "release" => Verdict::Allow,
            "skills_load" | "skills_read_file" => {
                self.evaluate_skill(agent_id, target.unwrap_or(""))
            }
            "connect" => self.evaluate_mcp_server(agent_id, target.unwrap_or("")),
            "skills_install" => Verdict::Allow, // อนุมัติติดตั้งแยกใน pending_installs
            _ => Verdict::Deny,                 // call ใช้ evaluate_mcp_tool
        }
    }
}

impl From<Effect> for Verdict {
    fn from(e: Effect) -> Self {
        match e {
            Effect::Allow => Verdict::Allow,
            Effect::Deny => Verdict::Deny,
            Effect::Ask => Verdict::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[agents.claude-code]
default = "deny"
skills = { allow = ["weekly-report", "pdf-*"], deny = ["deploy-*"] }

[agents.claude-code.mcp.github]
allow_tools = ["search_*", "get_*"]
deny_tools = ["delete_*"]

[agents.claude-code.mcp.postgres]
mode = "ask"

[agents."*"]
default = "deny"
"#;

    #[test]
    fn deny_wins_on_skills() {
        let p = PolicyEngine::from_toml(SAMPLE).unwrap();
        assert_eq!(
            p.evaluate_skill("claude-code", "weekly-report"),
            Verdict::Allow
        );
        assert_eq!(
            p.evaluate_skill("claude-code", "pdf-helper"),
            Verdict::Allow
        );
        assert_eq!(
            p.evaluate_skill("claude-code", "deploy-prod"),
            Verdict::Deny
        );
        assert_eq!(
            p.evaluate_skill("claude-code", "unknown-skill"),
            Verdict::Deny
        );
    }

    #[test]
    fn mcp_tool_rules() {
        let p = PolicyEngine::from_toml(SAMPLE).unwrap();
        assert_eq!(
            p.evaluate_mcp_tool("claude-code", "github", "search_repos"),
            Verdict::Allow
        );
        assert_eq!(
            p.evaluate_mcp_tool("claude-code", "github", "delete_repo"),
            Verdict::Deny
        );
        assert_eq!(
            p.evaluate_mcp_tool("claude-code", "postgres", "query"),
            Verdict::Ask
        );
    }

    #[test]
    fn unknown_agent_denied() {
        let p = PolicyEngine::from_toml(SAMPLE).unwrap();
        assert_eq!(p.evaluate_skill("nobody", "weekly-report"), Verdict::Deny);
    }

    const SHARED: &str = r#"
[shared]
skills = ["pdf-*"]
mcp = ["github"]
[shared.mcp_tools.github]
deny_tools = ["delete_*"]

[agents.a]
default = "ask"
skills = { allow = ["deploy-docs"], deny = ["deploy-*", "pdf-secret"] }
mcp.postgres = { mode = "ask" }
mcp.slack = { allow_tools = ["post_*"] }

[agents.b]
default = "allow"

[agents."*"]
default = "deny"
"#;

    fn shared() -> PolicyEngine {
        PolicyEngine::from_toml(SHARED).unwrap()
    }

    #[test]
    fn shared_skill_allowed_for_everyone() {
        let e = shared();
        assert_eq!(e.evaluate_skill("a", "pdf-merge"), Verdict::Allow);
        assert_eq!(e.evaluate_skill("zz", "pdf-merge"), Verdict::Allow);
    }

    #[test]
    fn agent_deny_beats_shared() {
        assert_eq!(shared().evaluate_skill("a", "pdf-secret"), Verdict::Deny);
    }

    #[test]
    fn agent_deny_beats_agent_allow() {
        assert_eq!(shared().evaluate_skill("a", "deploy-docs"), Verdict::Deny);
    }

    #[test]
    fn unlisted_skill_uses_default() {
        let e = shared();
        assert_eq!(e.evaluate_skill("a", "x"), Verdict::Ask);
        assert_eq!(e.evaluate_skill("b", "x"), Verdict::Allow);
        assert_eq!(e.evaluate_skill("zz", "x"), Verdict::Deny);
    }

    #[test]
    fn shared_server_tools() {
        let e = shared();
        assert_eq!(e.evaluate_mcp_tool("a", "github", "search"), Verdict::Allow);
        assert_eq!(
            e.evaluate_mcp_tool("a", "github", "delete_repo"),
            Verdict::Deny
        );
    }

    #[test]
    fn agent_tool_allow_and_mode() {
        let e = shared();
        assert_eq!(
            e.evaluate_mcp_tool("a", "slack", "post_msg"),
            Verdict::Allow
        );
        assert_eq!(e.evaluate_mcp_tool("a", "postgres", "query"), Verdict::Ask);
    }

    #[test]
    fn server_verdicts() {
        let e = shared();
        assert_eq!(e.evaluate_mcp_server("a", "github"), Verdict::Allow);
        assert_eq!(e.evaluate_mcp_server("a", "slack"), Verdict::Allow);
        assert_eq!(e.evaluate_mcp_server("a", "postgres"), Verdict::Ask);
        assert_eq!(e.evaluate_mcp_server("a", "other"), Verdict::Ask);
        assert_eq!(e.evaluate_mcp_server("zz", "other"), Verdict::Deny);
        assert_eq!(e.evaluate_mcp_server("zz", "github"), Verdict::Allow);
    }

    #[test]
    fn listing_meta_tools_always_allowed() {
        let e = shared();
        for t in ["skills_search", "mcp_list", "release"] {
            assert_eq!(e.evaluate_meta_tool("zz", t, None), Verdict::Allow, "{t}");
        }
        assert_eq!(
            e.evaluate_meta_tool("a", "connect", Some("other")),
            Verdict::Ask
        );
    }

    #[test]
    fn old_policy_without_shared_still_parses() {
        PolicyEngine::from_toml("[agents.cursor]\ndefault=\"allow\"\n").unwrap();
    }

    #[test]
    fn mcp_mode_shorthand_string_parses() {
        let e = PolicyEngine::from_toml("[agents.a]\ndefault = \"deny\"\nmcp.postgres = \"ask\"\n")
            .unwrap();
        assert_eq!(e.evaluate_mcp_server("a", "postgres"), Verdict::Ask);
        assert_eq!(e.agent_default("a"), Effect::Deny);
    }

    #[test]
    fn agent_mode_deny_beats_shared_and_allow_tools() {
        let e = PolicyEngine::from_toml(
            "[shared]\nmcp = [\"github\"]\n[agents.a]\ndefault = \"ask\"\n\
             mcp.github = \"deny\"\nmcp.slack = { allow_tools = [\"post_*\"], mode = \"deny\" }\n",
        )
        .unwrap();
        assert_eq!(e.evaluate_mcp_tool("a", "github", "search"), Verdict::Deny);
        assert_eq!(e.evaluate_mcp_tool("a", "slack", "post_x"), Verdict::Deny);
        assert_eq!(e.evaluate_mcp_server("a", "slack"), Verdict::Deny);
    }

    #[test]
    fn sees_everything_detects_allow_default_and_star() {
        let e = PolicyEngine::from_toml(
            "[agents.a]\ndefault = \"allow\"\n[agents.b]\ndefault = \"ask\"\n\
             skills = { allow = [\"*\"] }\n[agents.c]\ndefault = \"ask\"\n",
        )
        .unwrap();
        assert!(e.sees_everything("a"));
        assert!(e.sees_everything("b"));
        assert!(!e.sees_everything("c"));
    }
}
