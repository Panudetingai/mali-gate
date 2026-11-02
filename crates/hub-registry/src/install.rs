//! ติดตั้ง skill จาก path หรือ git URL — validate, allowlist, pin hash

use std::path::{Path, PathBuf};
use std::process::Command;

use hub_core::skills_dir;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::InstallSection;
use crate::skill::{self, ParseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallSource {
    LocalPath(PathBuf),
    /// `subdir` = โฟลเดอร์ skill ใน repo (เช่น monorepo ที่ SKILL.md ไม่ได้อยู่ root)
    GitUrl {
        repo: String,
        subdir: Option<String>,
    },
}

#[derive(thiserror::Error, Debug)]
pub enum InstallError {
    #[error("invalid source: {0}")]
    InvalidSource(String),
    #[error("source not allowed by policy: {0}")]
    NotAllowed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("skill parse error: {0}")]
    Parse(#[from] ParseError),
    #[error("git failed: {0}")]
    Git(String),
    #[error("skill already installed: {0} (use --force to replace)")]
    AlreadyExists(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinRecord {
    pub source: String,
    pub pinned_hash: String,
    pub installed_at: String,
}

const ORIGIN_FILE: &str = ".mali-gate-origin.json";

pub fn parse_source(raw: &str) -> Result<InstallSource, InstallError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(InstallError::InvalidSource("empty source".into()));
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("git@")
        || trimmed.starts_with("ssh://")
    {
        if trimmed.contains("/blob/") {
            return Err(InstallError::InvalidSource(
                "this is a GitHub web link, not a git clone URL — use \
                 https://github.com/OWNER/REPO.git#path/to/skill/folder \
                 (e.g. …/ponytail.git#skills/ponytail)"
                    .into(),
            ));
        }
        let (repo, subdir) = match trimmed.split_once('#') {
            Some((repo, sub)) => {
                let sub = sub.trim().trim_matches('/');
                if sub.is_empty() {
                    (repo.trim(), None)
                } else {
                    (repo.trim(), Some(sub.replace('\\', "/")))
                }
            }
            None => (trimmed, None),
        };
        return Ok(InstallSource::GitUrl {
            repo: repo.to_string(),
            subdir,
        });
    }
    Ok(InstallSource::LocalPath(PathBuf::from(trimmed)))
}

pub fn install_skill(
    source: &InstallSource,
    policy: &InstallSection,
    force: bool,
) -> Result<(String, PinRecord), InstallError> {
    check_allowed(source, policy)?;
    let fetched = fetch_to_staging(source)?;
    let skill_md = fetched.skill_root.join("SKILL.md");
    let manifest = skill::parse_skill_file(&skill_md)?;
    let pin = compute_pin(source, &fetched)?;
    let dest = skills_dir().join(&manifest.name);
    if dest.exists() && !force {
        return Err(InstallError::AlreadyExists(manifest.name.clone()));
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    copy_dir_recursive(&fetched.skill_root, &dest)?;
    write_pin(&dest, &pin)?;
    Ok((manifest.name, pin))
}

pub fn preview_source(
    source: &InstallSource,
    policy: &InstallSection,
) -> Result<(String, String, String, PinRecord), InstallError> {
    check_allowed(source, policy)?;
    let fetched = fetch_to_staging(source)?;
    let skill_md_path = fetched.skill_root.join("SKILL.md");
    let manifest = skill::parse_skill_file(&skill_md_path)?;
    let body = skill::read_skill_md(&fetched.skill_root)?;
    let excerpt: String = body.chars().take(800).collect();
    let pin = compute_pin(source, &fetched)?;
    if let Some(git_root) = fetched.git_root {
        let _ = std::fs::remove_dir_all(git_root);
    } else {
        let _ = std::fs::remove_dir_all(fetched.skill_root);
    }
    Ok((manifest.name, manifest.description, excerpt, pin))
}

fn check_allowed(source: &InstallSource, policy: &InstallSection) -> Result<(), InstallError> {
    match source {
        InstallSource::LocalPath(path) => {
            if !policy.allow_local_paths {
                return Err(InstallError::NotAllowed("local paths disabled".into()));
            }
            if !policy.allow_path_prefixes.is_empty() {
                let canon = path.canonicalize().map_err(InstallError::Io)?;
                let ok = policy.allow_path_prefixes.iter().any(|prefix| {
                    PathBuf::from(prefix)
                        .canonicalize()
                        .ok()
                        .is_some_and(|p| canon.starts_with(&p))
                });
                if !ok {
                    return Err(InstallError::NotAllowed(format!(
                        "path not under allow_path_prefixes: {}",
                        canon.display()
                    )));
                }
            }
            Ok(())
        }
        InstallSource::GitUrl { repo, .. } => {
            let host = git_host(repo).ok_or_else(|| {
                InstallError::NotAllowed(format!("cannot parse git host from {repo}"))
            })?;
            if !policy
                .allow_git_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(&host))
            {
                return Err(InstallError::NotAllowed(format!(
                    "git host `{host}` not in allow_git_hosts"
                )));
            }
            Ok(())
        }
    }
}

fn git_host(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("git@") {
        let host = rest.split(':').next()?;
        return Some(host.to_string());
    }
    let url = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"))?;
    let host = url.split('/').next()?.split('@').next_back()?;
    Some(host.to_string())
}

struct FetchedStaging {
    skill_root: PathBuf,
    /// โฟลเดอร์ที่มี `.git` (สำหรับ rev-parse); local = None
    git_root: Option<PathBuf>,
}

fn fetch_to_staging(source: &InstallSource) -> Result<FetchedStaging, InstallError> {
    let base = hub_core::data_root().join("staging");
    std::fs::create_dir_all(&base)?;
    let dir = base.join(format!(
        "install-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    match source {
        InstallSource::LocalPath(path) => {
            std::fs::create_dir_all(&dir)?;
            let src = path.canonicalize().map_err(InstallError::Io)?;
            if !src.is_dir() {
                return Err(InstallError::InvalidSource(
                    "local source must be a directory".into(),
                ));
            }
            copy_dir_recursive(&src, &dir)?;
            Ok(FetchedStaging {
                skill_root: dir,
                git_root: None,
            })
        }
        InstallSource::GitUrl { repo, subdir } => {
            let status = Command::new("git")
                .args(["clone", "--depth", "1", repo, &dir.to_string_lossy()])
                .status()
                .map_err(InstallError::Io)?;
            if !status.success() {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(InstallError::Git(format!("clone failed for {repo}")));
            }
            let skill_root = match subdir {
                Some(sub) => dir.join(sub),
                None => dir.clone(),
            };
            if !skill_root.join("SKILL.md").is_file() {
                let _ = std::fs::remove_dir_all(&dir);
                let hint = subdir.as_deref().unwrap_or("");
                let msg = if hint.is_empty() {
                    "SKILL.md missing at repo root — if the skill lives in a subfolder, \
                     append #path/to/folder (e.g. …/ponytail.git#skills/ponytail)"
                        .to_string()
                } else {
                    format!("SKILL.md missing under `{hint}` after clone")
                };
                return Err(InstallError::InvalidSource(msg));
            }
            Ok(FetchedStaging {
                skill_root,
                git_root: Some(dir),
            })
        }
    }
}

fn compute_pin(
    source: &InstallSource,
    fetched: &FetchedStaging,
) -> Result<PinRecord, InstallError> {
    let pinned_hash = match source {
        InstallSource::GitUrl { .. } => {
            let git_root = fetched
                .git_root
                .as_ref()
                .ok_or_else(|| InstallError::Git("missing git root".into()))?;
            let out = Command::new("git")
                .args(["-C", &git_root.to_string_lossy(), "rev-parse", "HEAD"])
                .output()
                .map_err(InstallError::Io)?;
            if !out.status.success() {
                return Err(InstallError::Git("rev-parse failed".into()));
            }
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
        InstallSource::LocalPath(_) => hash_dir(&fetched.skill_root)?,
    };
    let source_text = match source {
        InstallSource::LocalPath(p) => p.to_string_lossy().into_owned(),
        InstallSource::GitUrl { repo, subdir } => match subdir {
            Some(sub) => format!("{repo}#{sub}"),
            None => repo.clone(),
        },
    };
    Ok(PinRecord {
        source: source_text,
        pinned_hash,
        installed_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown".into()),
    })
}

/// SHA-256 ของไฟล์ทั้งหมดใต้โฟลเดอร์ (relative path + bytes) — ใช้เทียบ skill ซ้ำ
pub fn content_hash_dir(dir: &Path) -> Result<String, InstallError> {
    hash_dir(dir)
}

fn hash_dir(dir: &Path) -> Result<String, InstallError> {
    let mut paths = Vec::new();
    collect_files(dir, dir, &mut paths)?;
    paths.sort();
    let mut hasher = Sha256::new();
    for rel in paths {
        hasher.update(rel.as_bytes());
        let full = dir.join(&rel);
        let bytes = std::fs::read(&full)?;
        hasher.update(&bytes);
    }
    Ok(hex_encode(hasher.finalize().as_slice()))
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), InstallError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| InstallError::InvalidSource("path".into()))?;
            let mut s = rel.to_string_lossy().into_owned();
            if cfg!(windows) {
                s = s.replace('\\', "/");
            }
            out.push(s);
        }
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), InstallError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_file() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn write_pin(dest: &Path, pin: &PinRecord) -> Result<(), InstallError> {
    let text = serde_json::to_string_pretty(pin)?;
    std::fs::write(dest.join(ORIGIN_FILE), text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_git_subdir_and_blob_hint() {
        match parse_source("https://github.com/x/y.git#skills/foo").unwrap() {
            InstallSource::GitUrl { repo, subdir } => {
                assert_eq!(repo, "https://github.com/x/y.git");
                assert_eq!(subdir.as_deref(), Some("skills/foo"));
            }
            _ => panic!("expected git url"),
        }
        assert!(parse_source("https://github.com/x/y/blob/main/SKILL.md").is_err());
    }

    #[test]
    fn parse_git_and_path() {
        assert!(matches!(
            parse_source("https://github.com/x/y.git").unwrap(),
            InstallSource::GitUrl { .. }
        ));
        assert!(matches!(
            parse_source(r"D:\mali_gate\tests\fixtures\pdf-master").unwrap(),
            InstallSource::LocalPath(_)
        ));
    }

    #[test]
    fn install_local_fixture() {
        let home =
            std::env::temp_dir().join(format!("mali-gate-install-test-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        unsafe {
            std::env::set_var("MALI_GATE_HOME", &home);
        }
        let policy = InstallSection::default();
        let src = InstallSource::LocalPath(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/pdf-summary"),
        );
        let (name, pin) = install_skill(&src, &policy, true).expect("install");
        assert_eq!(name, "pdf-summary");
        assert!(!pin.pinned_hash.is_empty());
        unsafe {
            std::env::remove_var("MALI_GATE_HOME");
        }
        let _ = std::fs::remove_dir_all(home);
    }
}
