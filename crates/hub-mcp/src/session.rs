use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use hub_connect::{ConnectorManager, HubConfig};
use hub_core::{config_path, db_path, policy_path, skills_dir};
use hub_log::Logger;
use hub_policy::{AgentStore, ApprovalStore, PolicyEngine, PolicyError};
use hub_registry::index::SkillIndex;
use hub_registry::{PendingInstallStore, RegistryConfigFile};

use crate::log_hook::EventLog;

/// policy.toml ที่โหลดใหม่เมื่อไฟล์เปลี่ยน (เช็ก mtime + ขนาดก่อนตัดสินทุกครั้ง)
/// ไฟล์พัง → ใช้ฉบับล่าสุดที่ดีต่อ และเตือนครั้งเดียวต่อการเปลี่ยนแปลง
pub struct PolicyCache {
    path: PathBuf,
    state: Mutex<(Option<FileStamp>, Arc<PolicyEngine>)>,
}

/// (mtime, ขนาดไฟล์) — เปลี่ยนเมื่อไหร่โหลดใหม่
type FileStamp = (SystemTime, u64);

impl PolicyCache {
    /// โหลดครั้งแรก — ไฟล์พังตอนเริ่ม = error (fail closed, ไม่เปิดโหมดไม่ตรวจสิทธิ์)
    pub fn new(path: PathBuf) -> Result<Self, String> {
        let engine = match std::fs::read_to_string(&path) {
            Ok(text) => PolicyEngine::from_toml(&text)
                .map_err(|e| format!("invalid policy {}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PolicyEngine::empty(),
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let stamp = file_stamp(&path);
        Ok(Self {
            path,
            state: Mutex::new((stamp, Arc::new(engine))),
        })
    }

    /// ไฟล์เปลี่ยน → โหลดใหม่; พัง/หาย/อ่านไม่ได้ชั่วคราว → ใช้ฉบับล่าสุดที่ดีต่อ
    pub fn get(&self) -> Arc<PolicyEngine> {
        let stamp = file_stamp(&self.path);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // อ่านไม่ได้ชั่วคราว (antivirus/rename) → ไม่จำ stamp จะได้ลองใหม่รอบหน้า
        if stamp.is_some() && state.0 != stamp {
            if let Ok(text) = std::fs::read_to_string(&self.path) {
                state.0 = stamp;
                match PolicyEngine::from_toml(&text) {
                    Ok(engine) => state.1 = Arc::new(engine),
                    Err(e) => eprintln!(
                        "Mali Gate: {} is invalid — keeping previous policy ({e})",
                        self.path.display()
                    ),
                }
            }
        }
        state.1.clone()
    }
}

fn file_stamp(path: &std::path::Path) -> Option<FileStamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

pub struct HubSession {
    pub agent_id: String,
    pub index: Arc<Mutex<SkillIndex>>,
    pub events: EventLog,
    policy: PolicyCache,
    pub approvals: Arc<Mutex<ApprovalStore>>,
    pub connectors: Arc<ConnectorManager>,
    pub registry_cfg: RegistryConfigFile,
    pub pending_installs: Arc<Mutex<PendingInstallStore>>,
    pub strict_tokens: bool,
}

impl HubSession {
    pub fn open() -> Result<Self, String> {
        let db = db_path();
        let index = SkillIndex::open(&db).map_err(|e| e.to_string())?;
        let root = skills_dir();
        if root.is_dir() {
            let _ = index.index_dir(&root);
        }
        let logger = Arc::new(Mutex::new(Logger::open(&db).map_err(|e| e.to_string())?));
        let agents = AgentStore::open(&db).map_err(|e| e.to_string())?;
        let strict_tokens = agents.count().map_err(|e| e.to_string())? > 0;
        let agent_id = resolve_agent_id(&agents, strict_tokens)?;
        let approvals = Arc::new(Mutex::new(
            ApprovalStore::open(&db).map_err(|e| e.to_string())?,
        ));
        hub_policy::ensure_default_policy(&policy_path())
            .map_err(|e: PolicyError| e.to_string())?;
        let policy = PolicyCache::new(policy_path())?;
        HubConfig::ensure_sample(&config_path()).map_err(|e| e.to_string())?;
        let hub_config = HubConfig::load_from_disk().map_err(|e| e.to_string())?;
        let registry_cfg = hub_registry::RegistryConfigFile::load_from_disk();
        let connectors = Arc::new(ConnectorManager::new(hub_config));
        let pending_installs = Arc::new(Mutex::new(
            PendingInstallStore::open(&db).map_err(|e| e.to_string())?,
        ));
        let events = EventLog::open(logger, &agent_id);
        Ok(Self {
            agent_id,
            index: Arc::new(Mutex::new(index)),
            events,
            policy,
            approvals,
            connectors,
            registry_cfg,
            pending_installs,
            strict_tokens,
        })
    }

    /// policy ปัจจุบัน (โหลดใหม่ถ้าไฟล์เปลี่ยน)
    pub fn policy(&self) -> Arc<PolicyEngine> {
        self.policy.get()
    }

    pub fn reindex_skills(&self) -> Result<usize, String> {
        let root = skills_dir();
        let guard = self
            .index
            .lock()
            .map_err(|_| "skill index lock poisoned".to_string())?;
        guard.index_dir(&root).map_err(|e| e.to_string())
    }

    pub fn with_index<R>(
        &self,
        f: impl FnOnce(&SkillIndex) -> Result<R, String>,
    ) -> Result<R, String> {
        let guard = self
            .index
            .lock()
            .map_err(|_| "skill index lock poisoned".to_string())?;
        f(&guard)
    }
}

fn resolve_agent_id(agents: &AgentStore, strict: bool) -> Result<String, String> {
    let token = std::env::var("MALI_GATE_TOKEN")
        .or_else(|_| std::env::var("SKILLHUB_TOKEN"))
        .ok();
    if strict {
        let raw = token.ok_or_else(|| {
            "MALI_GATE_TOKEN required — run `mali_gate agent add <id>` and set token in MCP env"
                .to_string()
        })?;
        return agents
            .resolve_token(&raw)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "invalid or revoked MALI_GATE_TOKEN".to_string());
    }
    if let Some(raw) = token {
        if let Ok(Some(id)) = agents.resolve_token(&raw) {
            return Ok(id);
        }
    }
    Ok(std::env::var("MALI_GATE_AGENT")
        .or_else(|_| std::env::var("SKILLHUB_AGENT"))
        .unwrap_or_else(|_| "mcp".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hub_policy::Verdict;

    #[test]
    fn reload_keeps_last_good_policy() {
        let dir = std::env::temp_dir().join(format!("mg-policy-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.toml");
        std::fs::write(&path, "[agents.a]\ndefault = \"deny\"\n").unwrap();
        let cache = PolicyCache::new(path.clone()).unwrap();
        assert_eq!(cache.get().evaluate_skill("a", "x"), Verdict::Deny);

        std::fs::write(
            &path,
            "[agents.a]\ndefault = \"deny\"\nskills = { allow = [\"x\"] }\n",
        )
        .unwrap();
        assert_eq!(cache.get().evaluate_skill("a", "x"), Verdict::Allow);

        std::fs::write(&path, "[[[ broken").unwrap();
        assert_eq!(cache.get().evaluate_skill("a", "x"), Verdict::Allow);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_policy_at_startup_is_an_error() {
        let dir = std::env::temp_dir().join(format!("mg-policy-start-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("policy.toml");
        std::fs::write(&path, "[[[ broken").unwrap();
        assert!(PolicyCache::new(path.clone()).is_err());
        std::fs::write(&path, "[agents.a]\nskills = { allow = [\"x\"] }\n").unwrap();
        let cache = PolicyCache::new(path.clone()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(cache.get().evaluate_skill("a", "x"), Verdict::Allow);
        assert!(cache.get().is_configured());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
