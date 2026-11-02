//! อ่านส่วน `[install]` และ `[registry]` จาก `config.toml`

use std::path::Path;

use hub_core::config_path;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistryConfigFile {
    #[serde(default)]
    pub install: InstallSection,
    #[serde(default)]
    pub registry: RegistrySection,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallSection {
    #[serde(default = "default_true")]
    pub allow_local_paths: bool,
    #[serde(default = "default_git_hosts")]
    pub allow_git_hosts: Vec<String>,
    /// พาธโฟลเดอร์ที่อนุญาต copy จาก local (ว่าง = ทุกพาธถ้า allow_local_paths)
    #[serde(default)]
    pub allow_path_prefixes: Vec<String>,
}

impl Default for InstallSection {
    fn default() -> Self {
        Self {
            allow_local_paths: true,
            allow_git_hosts: default_git_hosts(),
            allow_path_prefixes: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_git_hosts() -> Vec<String> {
    vec![
        "github.com".into(),
        "gitlab.com".into(),
        "bitbucket.org".into(),
    ]
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegistrySection {
    /// ไฟล์ JSON index บนเครื่อง (array ของ skill entries)
    #[serde(default)]
    pub files: Vec<String>,
    /// URL ชี้ไปที่ JSON index (optional — ต้องอยู่ใน allow_git_hosts)
    #[serde(default)]
    pub urls: Vec<String>,
}

impl RegistryConfigFile {
    pub fn load_from_disk() -> Self {
        let path = config_path();
        if !path.is_file() {
            return Self::default();
        }
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn ensure_install_section(path: &Path) -> std::io::Result<()> {
        if path.is_file() {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // ไม่เขียนทับ config ทั้งไฟล์ถ้ามี hub-connect สร้างแล้ว — append note only when missing
        Ok(())
    }
}
