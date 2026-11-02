//! ชนิดข้อมูลกลาง, error, config และเส้นทางไฟล์ของ Mali Gate
//!
//! ข้อมูลผู้ใช้อยู่ที่ `~/.mali_gate/` (`skills/`, `policy.toml`, `mali_gate.db`, `config.toml`)
//! ห้ามใส่ logic เชื่อมเครือข่ายหรือสิทธิ์ไว้ใน crate นี้ — ให้เป็นชั้นรากที่ทุก crate พึ่ง

use std::path::{Path, PathBuf};

/// โฟลเดอร์ข้อมูลผู้ใช้จริง (`~/.mali_gate`) — ไม่อ่าน `MALI_GATE_HOME` / `SKILLHUB_HOME`
pub fn user_data_root() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .unwrap_or_default();
    let mali = PathBuf::from(&home).join(".mali_gate");
    if mali.is_dir() {
        return mali;
    }
    let legacy = PathBuf::from(home).join(".skillhub");
    if legacy.is_dir() && !mali.exists() {
        return legacy;
    }
    mali
}

/// คืนโฟลเดอร์ root ของข้อมูลผู้ใช้ (`~/.mali_gate` โดยค่าเริ่มต้น)
pub fn data_root() -> PathBuf {
    if let Some(from_env) = std::env::var_os("MALI_GATE_HOME") {
        return PathBuf::from(from_env);
    }
    // ชื่อ env เก่า — อ่านได้จนกว่าจะเลิกใช้
    if let Some(from_env) = std::env::var_os("SKILLHUB_HOME") {
        return PathBuf::from(from_env);
    }
    user_data_root()
}

/// หา `probe-agents.toml` สำหรับ example probe (รองรับ `MALI_GATE_HOME` ชี้ test dir)
pub fn probe_agents_config_path() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var_os("MALI_GATE_PROBE_FILE") {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }
    let active = data_root().join("probe-agents.toml");
    if active.is_file() {
        return Some(active);
    }
    let user = user_data_root().join("probe-agents.toml");
    if user.is_file() {
        return Some(user);
    }
    None
}

/// พาธฐานข้อมูล index + log (SQLite)
pub fn db_path() -> PathBuf {
    let root = data_root();
    let primary = root.join("mali_gate.db");
    if primary.is_file() {
        return primary;
    }
    let legacy = root.join("hub.db");
    if legacy.is_file() {
        return legacy;
    }
    primary
}

/// โฟลเดอร์คลัง skill ที่ agent โหลดผ่าน hub (ไม่ใช่ `.cursor/skills/`)
pub fn skills_dir() -> PathBuf {
    data_root().join("skills")
}

/// policy TOML (`~/.mali_gate/policy.toml`)
pub fn policy_path() -> PathBuf {
    data_root().join("policy.toml")
}

/// connector / MCP server config (`~/.mali_gate/config.toml`)
pub fn config_path() -> PathBuf {
    data_root().join("config.toml")
}

/// ตรวจว่า `candidate` อยู่ภายใต้ `root` หลัง canonicalize (กัน path traversal)
pub fn path_within_root(root: &Path, candidate: &Path) -> Result<PathBuf, PathError> {
    if candidate.is_absolute() {
        return Err(PathError::EscapesRoot);
    }
    for comp in candidate.components() {
        if comp == std::path::Component::ParentDir {
            return Err(PathError::EscapesRoot);
        }
    }
    let joined = root.join(candidate);
    let root_canon = root.canonicalize().map_err(PathError::Io)?;
    let file_canon = joined.canonicalize().map_err(PathError::Io)?;
    if !file_canon.starts_with(&root_canon) {
        return Err(PathError::EscapesRoot);
    }
    Ok(file_canon)
}

#[derive(thiserror::Error, Debug)]
pub enum PathError {
    #[error("path escapes allowed directory")]
    EscapesRoot,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env_var<T>(name: &str, value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let saved = std::env::var_os(name);
        unsafe {
            if let Some(v) = value {
                std::env::set_var(name, v);
            } else {
                std::env::remove_var(name);
            }
        }
        let out = f();
        unsafe {
            match saved {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        out
    }

    #[test]
    fn data_root_uses_mali_gate_home_when_set() {
        let p = with_env_var("MALI_GATE_HOME", Some("D:\\some-test-root"), data_root);
        assert!(
            p.ends_with("some-test-root"),
            "MALI_GATE_HOME ต้องชนะ, ได้ {p:?}"
        );
    }

    #[test]
    fn data_root_falls_back_to_mali_gate_dot_dir() {
        let p = with_env_var("MALI_GATE_HOME", None, || {
            with_env_var("SKILLHUB_HOME", None, data_root)
        });
        assert!(
            p.ends_with(".mali_gate"),
            "fallback ต้องเป็น ~/.mali_gate, ได้ {p:?}"
        );
    }
}
