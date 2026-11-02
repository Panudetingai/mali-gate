//! ตอบคำขออนุมัติ: เขียนสิทธิ์ลง policy ก่อน แล้วค่อย resolve (agent ที่รออยู่จะเห็น policy ใหม่ทันที)

use std::path::Path;

use crate::{ApprovalStore, ApproveError, Grant, GrantScope, PolicyEditor, PolicyError, grant_for};

#[derive(thiserror::Error, Debug)]
pub enum ApplyError {
    #[error(transparent)]
    Approve(#[from] ApproveError),
    #[error(transparent)]
    Policy(#[from] PolicyError),
}

/// ผลการตอบคำขอ
#[derive(Debug, Clone)]
pub struct ApprovalApplied {
    pub agent: String,
    pub grant: Option<Grant>,
    pub policy_changed: bool,
}

/// resolve ก่อน (atomic: ใครมาก่อนได้ — กันแข่งกับ timeout 120s ของ serve) แล้วค่อยเขียน policy
/// agent ที่รออยู่ได้คำตอบจากการ resolve; policy มีผลกับครั้งต่อไป
pub fn apply_approval(
    store: &ApprovalStore,
    policy_path: &Path,
    id: i64,
    scope: GrantScope,
) -> Result<ApprovalApplied, ApplyError> {
    let req = store.get(id)?;
    store.resolve_scoped(id, scope)?;
    let grant = grant_for(&req.kind, &req.target);
    let mut policy_changed = false;
    if let Some(g) = &grant {
        let mut ed = PolicyEditor::open(policy_path)?;
        policy_changed = match scope {
            GrantScope::Shared => ed.grant_shared(g),
            GrantScope::Agent => ed.grant_agent(&req.agent, g),
            GrantScope::Deny => ed.deny_agent(&req.agent, g),
            GrantScope::Once => false,
        };
        if policy_changed {
            ed.save()?;
        }
    }
    Ok(ApprovalApplied {
        agent: req.agent,
        grant,
        policy_changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApprovalOutcome, ApprovalStore, GrantScope, PolicyEngine, Verdict};

    struct Fx {
        dir: std::path::PathBuf,
        store: ApprovalStore,
    }

    fn fx(name: &str) -> Fx {
        let dir = std::env::temp_dir().join(format!(
            "mg-apply-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("policy.toml"), "[agents.a]\ndefault = \"ask\"\n").unwrap();
        let store = ApprovalStore::open(&dir.join("db.sqlite")).unwrap();
        Fx { dir, store }
    }

    impl Fx {
        fn policy(&self) -> std::path::PathBuf {
            self.dir.join("policy.toml")
        }
        fn engine(&self) -> PolicyEngine {
            PolicyEngine::from_toml(&std::fs::read_to_string(self.policy()).unwrap()).unwrap()
        }
        fn ask(&self, kind: &str, target: &str) -> i64 {
            self.store
                .insert("a", kind, target, kind, &serde_json::json!({}))
                .unwrap()
        }
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn approve_shared_writes_policy_then_resolves() {
        let f = fx("shared");
        let id = f.ask("skill_load", "pdf");
        let got = apply_approval(&f.store, &f.policy(), id, GrantScope::Shared).unwrap();
        assert!(got.policy_changed);
        assert_eq!(got.agent, "a");
        assert_eq!(
            f.engine().evaluate_skill("someone-else", "pdf"),
            Verdict::Allow
        );
        assert_eq!(
            f.store.poll_resolved(id).unwrap(),
            Some(ApprovalOutcome::Approved)
        );
    }

    #[test]
    fn approve_agent_for_call_grants_server_to_agent_only() {
        let f = fx("agent");
        let id = f.ask("call", "github");
        apply_approval(&f.store, &f.policy(), id, GrantScope::Agent).unwrap();
        let e = f.engine();
        assert_eq!(
            e.evaluate_mcp_tool("a", "github", "anything"),
            Verdict::Allow
        );
        assert_eq!(e.evaluate_mcp_server("b", "github"), Verdict::Deny);
    }

    #[test]
    fn approve_once_does_not_touch_policy() {
        let f = fx("once");
        let before = std::fs::read(f.policy()).unwrap();
        let id = f.ask("skill_load", "pdf");
        let got = apply_approval(&f.store, &f.policy(), id, GrantScope::Once).unwrap();
        assert!(!got.policy_changed);
        assert_eq!(std::fs::read(f.policy()).unwrap(), before);
        assert_eq!(
            f.store.poll_resolved(id).unwrap(),
            Some(ApprovalOutcome::Approved)
        );
    }

    #[test]
    fn approve_deny_writes_agent_deny() {
        let f = fx("deny");
        let id = f.ask("connect", "postgres");
        apply_approval(&f.store, &f.policy(), id, GrantScope::Deny).unwrap();
        assert_eq!(
            f.engine().evaluate_mcp_server("a", "postgres"),
            Verdict::Deny
        );
        assert_eq!(
            f.store.poll_resolved(id).unwrap(),
            Some(ApprovalOutcome::Denied)
        );
    }

    #[test]
    fn already_resolved_is_an_error_and_policy_untouched() {
        let f = fx("twice");
        let id = f.ask("skill_load", "pdf");
        apply_approval(&f.store, &f.policy(), id, GrantScope::Once).unwrap();
        let before = std::fs::read(f.policy()).unwrap();
        assert!(apply_approval(&f.store, &f.policy(), id, GrantScope::Shared).is_err());
        assert_eq!(std::fs::read(f.policy()).unwrap(), before);
    }

    #[test]
    fn wildcard_request_never_becomes_a_grant() {
        let f = fx("wild");
        let before = std::fs::read(f.policy()).unwrap();
        let id = f.ask("connect", "*");
        let got = apply_approval(&f.store, &f.policy(), id, GrantScope::Shared).unwrap();
        assert!(got.grant.is_none() && !got.policy_changed);
        assert_eq!(std::fs::read(f.policy()).unwrap(), before);
    }

    #[test]
    fn request_is_claimed_before_policy_write() {
        // policy เขียนไม่ได้ (เป็นโฟลเดอร์) → error แต่คำขอต้องถูก resolve แล้ว (ไม่ค้าง/ไม่แข่งกับ timeout)
        let f = fx("order");
        let bad = f.dir.join("policy-dir");
        std::fs::create_dir_all(&bad).unwrap();
        let id = f.ask("skill_load", "pdf");
        assert!(apply_approval(&f.store, &bad, id, GrantScope::Shared).is_err());
        assert_eq!(
            f.store.poll_resolved(id).unwrap(),
            Some(ApprovalOutcome::Approved)
        );
    }
}
