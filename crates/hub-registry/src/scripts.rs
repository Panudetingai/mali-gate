//! scripts/ ใน skill — ห้าม agent รันเอง; hub เป็นประตูเดียว (sandbox จริงทำภายหลัง)

use std::path::Path;

use hub_core::path_within_root;

#[derive(thiserror::Error, Debug)]
pub enum ScriptError {
    #[error("path error: {0}")]
    Path(#[from] hub_core::PathError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "execution blocked: skill scripts must run through Mali Gate (OS sandbox not enabled on this platform)"
    )]
    SandboxNotAvailable,
}

pub fn list_scripts(sk_dir: &Path) -> Result<Vec<String>, ScriptError> {
    let scripts = sk_dir.join("scripts");
    if !scripts.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&scripts)? {
        let entry = entry?;
        if entry.path().is_file() {
            let rel = format!("scripts/{}", entry.file_name().to_string_lossy());
            out.push(rel);
        }
    }
    out.sort();
    Ok(out)
}

/// ตรวจ path อยู่ใต้ `scripts/` — ยังไม่ spawn process (เฟส 4 stub)
pub fn validate_script_path(
    sk_dir: &Path,
    relative: &str,
) -> Result<std::path::PathBuf, ScriptError> {
    if !relative.starts_with("scripts/") {
        return Err(ScriptError::Path(hub_core::PathError::EscapesRoot));
    }
    path_within_root(sk_dir, Path::new(relative)).map_err(ScriptError::Path)
}

pub fn run_script_stub(_sk_dir: &Path, _relative: &str) -> Result<(), ScriptError> {
    Err(ScriptError::SandboxNotAvailable)
}
