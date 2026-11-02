use std::thread;
use std::time::{Duration, Instant};

use hub_log::{Decision, EventKind};
use hub_policy::{ApprovalOutcome, Verdict};
use serde_json::Value;

use crate::session::HubSession;

const ASK_TIMEOUT: Duration = Duration::from_secs(120);

pub struct GateOutcome {
    pub allowed: bool,
    pub decision: Decision,
    pub error: Option<String>,
}

impl GateOutcome {
    pub fn allow() -> Self {
        Self {
            allowed: true,
            decision: Decision::Allow,
            error: None,
        }
    }

    pub fn deny(msg: impl Into<String>) -> Self {
        Self {
            allowed: false,
            decision: Decision::Deny,
            error: Some(msg.into()),
        }
    }
}

pub fn check_skill(session: &HubSession, skill_name: &str, tool: &str) -> GateOutcome {
    if !skill_exists(session, skill_name) {
        return GateOutcome::deny(format!("skill not found: {skill_name}"));
    }
    let policy = session.policy();
    let verdict = if session.strict_tokens || policy.is_configured() {
        policy.evaluate_skill(&session.agent_id, skill_name)
    } else {
        Verdict::Allow
    };
    apply_verdict(
        session,
        verdict,
        EventKind::SkillLoad,
        skill_name,
        tool,
        serde_json::json!({ "name": skill_name }),
    )
}

pub fn check_meta(
    session: &HubSession,
    tool: &str,
    target: &str,
    args: Value,
    kind: EventKind,
) -> GateOutcome {
    if tool == "connect" && !server_exists(session, target) {
        return GateOutcome::deny(format!("unknown mcp server: {target} (see mcp_list)"));
    }
    let policy = session.policy();
    let verdict = if session.strict_tokens || policy.is_configured() {
        policy.evaluate_meta_tool(&session.agent_id, tool, Some(target))
    } else {
        Verdict::Allow
    };
    apply_verdict(session, verdict, kind, target, tool, args)
}

pub fn check_mcp_call(session: &HubSession, server: &str, tool: &str, args: Value) -> GateOutcome {
    if !server_exists(session, server) {
        return GateOutcome::deny(format!("unknown mcp server: {server} (see mcp_list)"));
    }
    let policy = session.policy();
    let verdict = if session.strict_tokens || policy.is_configured() {
        policy.evaluate_mcp_tool(&session.agent_id, server, tool)
    } else {
        Verdict::Allow
    };
    apply_verdict(session, verdict, EventKind::Call, server, tool, args)
}

/// ของที่ไม่มีอยู่จริง (รวมชื่อแบบ `*`) ปฏิเสธทันที — ไม่สร้างคำขออนุมัติให้ผู้ใช้กดพลาด
fn skill_exists(session: &HubSession, name: &str) -> bool {
    session
        .with_index(|i| i.get_by_name(name).map_err(|e| e.to_string()))
        .is_ok_and(|hit| hit.is_some())
}

fn server_exists(session: &HubSession, name: &str) -> bool {
    session.connectors.servers().iter().any(|(n, _)| n == name)
}

fn apply_verdict(
    session: &HubSession,
    verdict: Verdict,
    kind: EventKind,
    target: &str,
    tool: &str,
    args: Value,
) -> GateOutcome {
    match verdict {
        Verdict::Allow => GateOutcome::allow(),
        Verdict::Deny => {
            let msg = format!("policy denied: agent={} tool={tool}", session.agent_id);
            GateOutcome::deny(msg)
        }
        Verdict::Ask => match wait_for_approval(session, kind, target, tool, &args) {
            ApprovalOutcome::Approved => GateOutcome {
                allowed: true,
                decision: Decision::AskApproved,
                error: None,
            },
            ApprovalOutcome::Denied => GateOutcome {
                allowed: false,
                decision: Decision::AskDenied,
                error: Some(format!("approval denied or timed out for {tool}")),
            },
        },
    }
}

fn wait_for_approval(
    session: &HubSession,
    kind: EventKind,
    target: &str,
    tool: &str,
    args: &Value,
) -> ApprovalOutcome {
    let id = match session.approvals.lock().map_err(|_| ()).and_then(|store| {
        store
            .insert(&session.agent_id, kind.as_str(), target, tool, args)
            .map_err(|_| ())
    }) {
        Ok(id) => id,
        Err(()) => return ApprovalOutcome::Denied,
    };
    eprintln!(
        "Mali Gate: approval #{id} — agent `{}` wants `{tool}` on `{target}`",
        session.agent_id
    );
    eprintln!("  Run: mali_gate approve resolve {id} --shared | --agent | --once | --no");
    let deadline = Instant::now() + ASK_TIMEOUT;
    while Instant::now() < deadline {
        let polled = session
            .approvals
            .lock()
            .ok()
            .and_then(|store| store.poll_resolved(id).ok())
            .flatten();
        if let Some(outcome) = polled {
            return outcome;
        }
        thread::sleep(Duration::from_millis(400));
    }
    if let Ok(store) = session.approvals.lock() {
        let _ = store.resolve(id, ApprovalOutcome::Denied);
    }
    ApprovalOutcome::Denied
}
