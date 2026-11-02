//! แก้ policy.toml แบบคงคอมเมนต์/ลำดับเดิม (toml_edit) — CLI/TUI/MCP เขียนสิทธิ์ผ่านตัวนี้เท่านั้น

use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::{Effect, PolicyEngine, PolicyError};

/// ของที่ให้/ถอนสิทธิ์ได้
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    Skill(String),
    McpServer(String),
}

pub struct PolicyEditor {
    path: PathBuf,
    doc: DocumentMut,
}

fn new_table() -> Item {
    let mut t = Table::new();
    t.set_implicit(true);
    Item::Table(t)
}

fn new_dotted() -> Item {
    let mut t = Table::new();
    t.set_dotted(true);
    Item::Table(t)
}

fn new_inline() -> Item {
    Item::Value(Value::InlineTable(InlineTable::new()))
}

/// ลูกที่เป็นตาราง — สร้างถ้าไม่มี; ค่าที่ไม่ใช่ตาราง (เช่น `mcp.x = "ask"`) ถูกแทนที่
fn child<'a>(
    parent: &'a mut dyn TableLike,
    key: &str,
    make: fn() -> Item,
) -> &'a mut dyn TableLike {
    let item = parent.entry(key).or_insert(make());
    if item.as_table_like().is_none() {
        *item = make();
    }
    item.as_table_like_mut()
        .unwrap_or_else(|| unreachable!("child() always leaves a table"))
}

fn existing<'a>(parent: &'a mut dyn TableLike, key: &str) -> Option<&'a mut dyn TableLike> {
    parent.get_mut(key)?.as_table_like_mut()
}

fn push_unique(t: &mut dyn TableLike, key: &str, val: &str) -> bool {
    let item = t
        .entry(key)
        .or_insert(Item::Value(Value::Array(Array::new())));
    if item.as_array().is_none() {
        *item = Item::Value(Value::Array(Array::new()));
    }
    let Some(arr) = item.as_array_mut() else {
        return false;
    };
    if arr.iter().any(|v| v.as_str() == Some(val)) {
        return false;
    }
    arr.push(val);
    true
}

fn remove_value(t: &mut dyn TableLike, key: &str, val: &str) -> bool {
    let Some(arr) = t.get_mut(key).and_then(Item::as_array_mut) else {
        return false;
    };
    let before = arr.len();
    arr.retain(|v| v.as_str() != Some(val));
    arr.len() != before
}

fn set_str(t: &mut dyn TableLike, key: &str, val: &str) -> bool {
    if t.get(key).and_then(Item::as_str) == Some(val) {
        return false;
    }
    t.insert(key, toml_edit::value(val));
    true
}

impl PolicyEditor {
    pub fn open(path: &Path) -> Result<Self, PolicyError> {
        let text = if path.is_file() {
            std::fs::read_to_string(path)?
        } else {
            String::new()
        };
        Self::from_str(path, &text)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(path: &Path, text: &str) -> Result<Self, PolicyError> {
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| PolicyError::Invalid(e.to_string()))?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    pub fn to_text(&self) -> String {
        self.doc.to_string()
    }

    fn shared(&mut self) -> &mut dyn TableLike {
        child(self.doc.as_table_mut(), "shared", new_table)
    }

    /// `[agents.<id>]` — สร้างใหม่ด้วย `default = "ask"`
    fn agent(&mut self, id: &str) -> &mut dyn TableLike {
        let agents = child(self.doc.as_table_mut(), "agents", new_table);
        if agents.get(id).is_none() {
            let mut t = Table::new();
            t.insert("default", toml_edit::value(Effect::Ask.as_str()));
            agents.insert(id, Item::Table(t));
        }
        child(agents, id, new_table)
    }

    fn existing_agent(&mut self, id: &str) -> Option<&mut dyn TableLike> {
        existing(existing(self.doc.as_table_mut(), "agents")?, id)
    }

    fn set_mcp_mode(&mut self, agent: &str, server: &str, mode: Effect) -> bool {
        let mcp = child(self.agent(agent), "mcp", new_dotted);
        set_str(child(mcp, server, new_inline), "mode", mode.as_str())
    }

    pub fn grant_shared(&mut self, g: &Grant) -> bool {
        match g {
            Grant::Skill(n) => push_unique(self.shared(), "skills", n),
            Grant::McpServer(n) => push_unique(self.shared(), "mcp", n),
        }
    }

    pub fn grant_agent(&mut self, agent: &str, g: &Grant) -> bool {
        match g {
            Grant::Skill(n) => {
                push_unique(child(self.agent(agent), "skills", new_inline), "allow", n)
            }
            Grant::McpServer(n) => self.set_mcp_mode(agent, n, Effect::Allow),
        }
    }

    pub fn deny_agent(&mut self, agent: &str, g: &Grant) -> bool {
        match g {
            Grant::Skill(n) => {
                push_unique(child(self.agent(agent), "skills", new_inline), "deny", n)
            }
            Grant::McpServer(n) => self.set_mcp_mode(agent, n, Effect::Deny),
        }
    }

    /// `None` = ถอนจาก `[shared]`; `Some(a)` = ถอน allow ของ agent นั้น
    pub fn revoke(&mut self, agent: Option<&str>, g: &Grant) -> bool {
        match (agent, g) {
            (None, Grant::Skill(n)) => existing(self.doc.as_table_mut(), "shared")
                .is_some_and(|t| remove_value(t, "skills", n)),
            (None, Grant::McpServer(n)) => existing(self.doc.as_table_mut(), "shared")
                .is_some_and(|t| remove_value(t, "mcp", n)),
            (Some(a), Grant::Skill(n)) => self
                .existing_agent(a)
                .and_then(|t| existing(t, "skills"))
                .is_some_and(|t| remove_value(t, "allow", n)),
            (Some(a), Grant::McpServer(n)) => {
                let Some(mcp) = self.existing_agent(a).and_then(|t| existing(t, "mcp")) else {
                    return false;
                };
                // ถอนเฉพาะส่วนที่ "ให้" (mode allow, allow_tools) — deny_tools ต้องอยู่ต่อ
                let changed = existing(mcp, n).is_some_and(|t| {
                    let mode_allow = t.get("mode").and_then(Item::as_str) == Some("allow");
                    let a = mode_allow && t.remove("mode").is_some();
                    let b = t.remove("allow_tools").is_some();
                    a || b
                });
                if existing(mcp, n).is_some_and(|t| t.is_empty()) {
                    mcp.remove(n);
                }
                changed
            }
        }
    }

    /// ตัด `"*"` ออกจาก `skills.allow` และ `mcp.*.allow_tools` ของ agent — คืนรายการที่ตัด
    pub fn remove_agent_wildcards(&mut self, agent: &str) -> Vec<String> {
        let mut removed = Vec::new();
        let Some(a) = self.existing_agent(agent) else {
            return removed;
        };
        if let Some(skills) = existing(a, "skills") {
            if remove_value(skills, "allow", "*") {
                removed.push("skills.allow \"*\"".to_string());
            }
        }
        if let Some(mcp) = self.existing_agent(agent).and_then(|a| existing(a, "mcp")) {
            let servers: Vec<String> = mcp.iter().map(|(k, _)| k.to_string()).collect();
            for s in servers {
                if let Some(t) = existing(mcp, &s) {
                    if remove_value(t, "allow_tools", "*") {
                        removed.push(format!("mcp.{s}.allow_tools \"*\""));
                    }
                }
            }
        }
        removed
    }

    pub fn set_default(&mut self, agent: &str, effect: Effect) -> bool {
        set_str(self.agent(agent), "default", effect.as_str())
    }

    /// ตรวจว่ายัง parse ได้ → สำรองครั้งแรก → เขียน tmp แล้ว rename
    pub fn save(&self) -> Result<(), PolicyError> {
        let text = self.to_text();
        PolicyEngine::from_toml(&text).map_err(|e| PolicyError::Invalid(e.to_string()))?;
        let sibling = |suffix: &str| {
            let mut s = self.path.as_os_str().to_owned();
            s.push(suffix);
            PathBuf::from(s)
        };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bak = sibling(".mali-gate.bak");
        if self.path.is_file() && !bak.exists() {
            std::fs::copy(&self.path, &bak)?;
        }
        let tmp = sibling(".mali-gate.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Effect, PolicyEngine, Verdict};

    fn ed(text: &str) -> PolicyEditor {
        PolicyEditor::from_str(Path::new("unused.toml"), text).unwrap()
    }

    fn engine(e: &PolicyEditor) -> PolicyEngine {
        PolicyEngine::from_toml(&e.to_text()).unwrap()
    }

    #[test]
    fn grant_shared_creates_section_and_keeps_comments() {
        let mut e = ed("# keep me\n[agents.a]\ndefault = \"ask\"\n");
        assert!(e.grant_shared(&Grant::Skill("s1".into())));
        assert!(e.to_text().contains("# keep me"));
        assert_eq!(engine(&e).evaluate_skill("a", "s1"), Verdict::Allow);
    }

    #[test]
    fn grant_agent_updates_inline_table() {
        let mut e = ed("[agents.cursor]\nskills = { allow = [\"*\"], deny = [] }\n");
        assert!(e.grant_agent("cursor", &Grant::Skill("x".into())));
        let text = e.to_text();
        assert_eq!(text.matches("skills").count(), 1, "{text}");
        let doc: toml::Value = toml::from_str(&text).unwrap();
        let allow = doc["agents"]["cursor"]["skills"]["allow"]
            .as_array()
            .unwrap();
        assert_eq!(allow.len(), 2);
        assert_eq!(allow[1].as_str(), Some("x"));
    }

    #[test]
    fn grant_agent_creates_missing_agent_with_ask_default() {
        let mut e = ed("");
        assert!(e.grant_agent("new", &Grant::McpServer("gh".into())));
        let p = engine(&e);
        assert_eq!(p.agent_default("new"), Effect::Ask);
        assert_eq!(p.evaluate_mcp_server("new", "gh"), Verdict::Allow);
        let text = e.to_text();
        assert!(text.contains("[agents.new]"), "{text}");
        assert!(text.contains("mcp.gh = { mode = \"allow\" }"), "{text}");
    }

    #[test]
    fn duplicate_grant_is_noop() {
        let mut e = ed("");
        assert!(e.grant_shared(&Grant::Skill("s1".into())));
        assert!(!e.grant_shared(&Grant::Skill("s1".into())));
        assert_eq!(e.to_text().matches("s1").count(), 1);
        assert!(e.grant_agent("a", &Grant::Skill("k".into())));
        assert!(!e.grant_agent("a", &Grant::Skill("k".into())));
    }

    #[test]
    fn revoke_and_deny() {
        let mut e = ed("[agents.a]\ndefault = \"allow\"\n");
        e.grant_shared(&Grant::Skill("s".into()));
        e.grant_shared(&Grant::McpServer("gh".into()));
        assert!(e.revoke(None, &Grant::Skill("s".into())));
        assert!(!e.revoke(None, &Grant::Skill("s".into())));
        assert!(e.deny_agent("a", &Grant::Skill("s".into())));
        assert!(e.deny_agent("a", &Grant::McpServer("gh".into())));
        let p = engine(&e);
        assert_eq!(p.evaluate_skill("a", "s"), Verdict::Deny);
        assert_eq!(p.evaluate_mcp_server("a", "gh"), Verdict::Deny);
        assert!(e.grant_agent("a", &Grant::Skill("t".into())));
        assert!(e.revoke(Some("a"), &Grant::Skill("t".into())));
        // revoke ถอนแค่สิ่งที่ "ให้" — deny ที่ผู้ใช้ตั้งไว้ต้องอยู่ต่อ
        assert!(!e.revoke(Some("a"), &Grant::McpServer("gh".into())));
        assert_eq!(engine(&e).evaluate_mcp_server("a", "gh"), Verdict::Deny);
    }

    #[test]
    fn set_default_changes_effect() {
        let mut e = ed("[agents.a]\ndefault = \"allow\"\n");
        assert!(e.set_default("a", Effect::Ask));
        assert!(!e.set_default("a", Effect::Ask));
        assert_eq!(engine(&e).agent_default("a"), Effect::Ask);
    }

    #[test]
    fn save_is_atomic_and_backs_up_once() {
        let dir = std::env::temp_dir().join(format!("mg-editor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.toml");
        std::fs::write(&path, "# original\n").unwrap();
        let mut e = PolicyEditor::open(&path).unwrap();
        e.grant_shared(&Grant::Skill("a".into()));
        e.save().unwrap();
        let mut e = PolicyEditor::open(&path).unwrap();
        e.grant_shared(&Grant::Skill("b".into()));
        e.save().unwrap();
        let bak = std::fs::read_to_string(dir.join("policy.toml.mali-gate.bak")).unwrap();
        assert_eq!(bak, "# original\n");
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(now.contains("\"a\"") && now.contains("\"b\"") && now.contains("# original"));
        assert!(!dir.join("policy.toml.mali-gate.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn revoke_agent_server_keeps_deny_tools() {
        let mut e = ed("[shared]\nmcp = [\"gh\"]\n[agents.a]\ndefault = \"ask\"\n\
             mcp.gh = { allow_tools = [\"x\"], deny_tools = [\"delete_*\"], mode = \"allow\" }\n");
        assert!(e.revoke(Some("a"), &Grant::McpServer("gh".into())));
        assert_eq!(
            engine(&e).evaluate_mcp_tool("a", "gh", "delete_repo"),
            Verdict::Deny
        );
        let mut e2 = ed("[agents.a]\nmcp.gh = { mode = \"allow\" }\n");
        assert!(e2.revoke(Some("a"), &Grant::McpServer("gh".into())));
        assert!(!e2.to_text().contains("gh"), "{}", e2.to_text());
    }

    #[test]
    fn remove_agent_wildcards_strips_star() {
        let mut e = ed(
            "[agents.a]\ndefault = \"allow\"\nskills = { allow = [\"*\", \"k\"], deny = [] }\n\
             mcp.gh = { allow_tools = [\"*\"] }\n",
        );
        let removed = e.remove_agent_wildcards("a");
        assert_eq!(removed.len(), 2, "{removed:?}");
        let p = engine(&e);
        assert_eq!(p.evaluate_skill("a", "k"), Verdict::Allow);
        assert!(!e.to_text().contains("\"*\""), "{}", e.to_text());
    }
}
