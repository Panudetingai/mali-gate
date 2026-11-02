use std::thread;
use std::time::{Duration, Instant};

use hub_registry::{install_skill, parse_source, preview_source};

use crate::session::HubSession;

const INSTALL_WAIT: Duration = Duration::from_secs(120);

pub fn run_install_with_approval(hub: &HubSession, source_raw: &str) -> Result<String, String> {
    let parsed = parse_source(source_raw).map_err(|e| e.to_string())?;
    let (name, desc, preview, pin) =
        preview_source(&parsed, &hub.registry_cfg.install).map_err(|e| e.to_string())?;
    let id = hub
        .pending_installs
        .lock()
        .map_err(|_| "pending install lock poisoned".to_string())?
        .insert(
            &hub.agent_id,
            source_raw,
            &name,
            &desc,
            &preview,
            &pin.pinned_hash,
        )
        .map_err(|e| e.to_string())?;
    eprintln!("Mali Gate: install request #{id} — `{name}` from `{source_raw}`");
    eprintln!("  Preview (first lines):");
    for line in preview.lines().take(8) {
        eprintln!("    {line}");
    }
    eprintln!("  Run: mali_gate install approve {id} --yes");
    let deadline = Instant::now() + INSTALL_WAIT;
    while Instant::now() < deadline {
        let status = hub
            .pending_installs
            .lock()
            .map_err(|_| "lock".to_string())?
            .poll_resolved(id)
            .map_err(|e| e.to_string())?;
        if status.as_deref() == Some("approved") {
            return finish_install(hub, &parsed, &name, id);
        }
        if status.as_deref() == Some("denied") {
            return Err("install denied by user".into());
        }
        thread::sleep(Duration::from_millis(400));
    }
    let _ = hub
        .pending_installs
        .lock()
        .map_err(|_| "lock".to_string())?
        .resolve(id, "denied");
    Err("install approval timed out (120s)".into())
}

fn finish_install(
    hub: &HubSession,
    parsed: &hub_registry::InstallSource,
    name: &str,
    pending_id: i64,
) -> Result<String, String> {
    let dest = hub_core::skills_dir().join(name);
    let (installed_name, pin) = if dest.is_dir() {
        (
            name.to_string(),
            hub_registry::PinRecord {
                source: String::new(),
                pinned_hash: "already-installed".into(),
                installed_at: String::new(),
            },
        )
    } else {
        install_skill(parsed, &hub.registry_cfg.install, true).map_err(|e| e.to_string())?
    };
    hub.reindex_skills()?;
    serde_json::to_string_pretty(&serde_json::json!({
        "installed": installed_name,
        "name": name,
        "pinned_hash": pin.pinned_hash,
        "pending_id": pending_id,
    }))
    .map_err(|e| e.to_string())
}
