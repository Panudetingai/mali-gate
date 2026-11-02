//! รวม skill จากโฟลเดอร์ที่ AI CLI ใช้ (Cursor, Claude ฯลฯ) เข้า `~/.mali_gate/skills/`

use std::path::{Path, PathBuf};

use hub_core::{data_root, skills_dir};

use crate::install::{InstallError, content_hash_dir};
use crate::skill::{self, ParseError};

const SKIP_DIR_NAMES: &[&str] = &["synced"];

#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    pub dry_run: bool,
    pub force: bool,
    /// ลบโฟลเดอร์ต้นทางเมื่อเนื้อหาตรงกับที่มีในคลังแล้ว (กัน copy ซ้ำในเครื่อง)
    pub clean_sources: bool,
    /// ถ้าว่าง ใช้ `default_sync_roots()`
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub imported: Vec<String>,
    pub skipped_identical: Vec<SkippedIdentical>,
    pub conflicts: Vec<Conflict>,
    pub cleaned_sources: Vec<PathBuf>,
    pub parse_errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SkippedIdentical {
    pub name: String,
    pub source: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub name: String,
    pub source: PathBuf,
    pub reason: &'static str,
}

#[derive(thiserror::Error, Debug)]
pub enum SyncError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("install/hash: {0}")]
    Install(#[from] InstallError),
    #[error("parse: {0}")]
    Parse(#[from] ParseError),
}

/// โฟลเดอร์ที่มักมี skill แบบ `name/SKILL.md` (ไม่รวมคลัง hub)
pub fn default_sync_roots() -> Vec<PathBuf> {
    let home = data_root().parent().map(Path::to_path_buf).or_else(|| {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    });
    let Some(home) = home else {
        return Vec::new();
    };
    vec![
        home.join(".cursor").join("skills"),
        home.join(".claude").join("skills"),
        home.join(".codex").join("skills"),
        home.join(".gemini").join("skills"),
    ]
}

#[derive(Debug, Clone, Default)]
pub struct CleanOptions {
    pub dry_run: bool,
    /// ลบเมื่อมี skill ชื่อเดียวกันในคลัง แม้เนื้อหาต่าง (ต้องยืนยันใน CLI)
    pub by_name: bool,
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanReport {
    pub removed: Vec<CleanedEntry>,
    pub kept_different: Vec<Conflict>,
    pub not_in_library: Vec<NotInLibrary>,
    pub parse_errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CleanedEntry {
    pub name: String,
    pub source: PathBuf,
}

#[derive(Debug, Clone)]
pub struct NotInLibrary {
    pub folder: String,
    pub source: PathBuf,
}

/// ลบ skill ในโฟลเดอร์ AI CLI ที่มีอยู่ในคลัง `~/.mali_gate/skills/` แล้ว
pub fn clean_cli_skills(opts: &CleanOptions) -> Result<CleanReport, SyncError> {
    let library = skills_dir();
    let roots = if opts.roots.is_empty() {
        default_sync_roots()
    } else {
        opts.roots.clone()
    };
    let mut report = CleanReport::default();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for bundle in discover_bundles(&root)? {
            if is_under(&bundle, &library) {
                continue;
            }
            let skill_md = bundle.join("SKILL.md");
            let manifest = match skill::parse_skill_file(&skill_md) {
                Ok(m) => m,
                Err(e) => {
                    report
                        .parse_errors
                        .push(format!("{}: {e}", bundle.display()));
                    continue;
                }
            };
            let dest = library.join(&manifest.name);
            if !dest.is_dir() {
                report.not_in_library.push(NotInLibrary {
                    folder: manifest.name,
                    source: bundle,
                });
                continue;
            }
            let same_content = content_hash_dir(&bundle)? == content_hash_dir(&dest)?;
            if same_content || opts.by_name {
                if opts.dry_run {
                    if !bundle.is_dir() {
                        continue;
                    }
                } else if bundle.is_dir() {
                    std::fs::remove_dir_all(&bundle)?;
                }
                report.removed.push(CleanedEntry {
                    name: manifest.name,
                    source: bundle,
                });
            } else {
                report.kept_different.push(Conflict {
                    name: manifest.name,
                    source: bundle,
                    reason: "library copy differs (use --by-name to remove anyway)",
                });
            }
        }
    }
    Ok(report)
}

pub fn sync_skills(opts: &SyncOptions) -> Result<SyncReport, SyncError> {
    let library = skills_dir();
    let roots = if opts.roots.is_empty() {
        default_sync_roots()
    } else {
        opts.roots.clone()
    };
    let mut report = SyncReport::default();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for bundle in discover_bundles(&root)? {
            if is_under(&bundle, &library) {
                continue;
            }
            let skill_md = bundle.join("SKILL.md");
            let manifest = match skill::parse_skill_file(&skill_md) {
                Ok(m) => m,
                Err(e) => {
                    report
                        .parse_errors
                        .push(format!("{}: {e}", bundle.display()));
                    continue;
                }
            };

            let dest = library.join(&manifest.name);
            let source_hash = content_hash_dir(&bundle)?;
            if dest.is_dir() {
                let dest_hash = content_hash_dir(&dest)?;
                if dest_hash == source_hash {
                    report.skipped_identical.push(SkippedIdentical {
                        name: manifest.name.clone(),
                        source: bundle.clone(),
                    });
                    if opts.clean_sources {
                        remove_tree(&bundle, opts.dry_run, &mut report.cleaned_sources)?;
                    }
                    continue;
                }
                if !opts.force {
                    report.conflicts.push(Conflict {
                        name: manifest.name.clone(),
                        source: bundle.clone(),
                        reason: "library copy differs (use --force to replace)",
                    });
                    continue;
                }
            }

            if !opts.dry_run {
                if dest.exists() {
                    std::fs::remove_dir_all(&dest)?;
                }
                copy_dir_recursive(&bundle, &dest)?;
            }
            report.imported.push(manifest.name);
            if opts.clean_sources {
                remove_tree(&bundle, opts.dry_run, &mut report.cleaned_sources)?;
            }
        }
    }
    Ok(report)
}

fn remove_tree(path: &Path, dry_run: bool, cleaned: &mut Vec<PathBuf>) -> Result<(), SyncError> {
    if dry_run {
        cleaned.push(path.to_path_buf());
        return Ok(());
    }
    if path.is_dir() {
        std::fs::remove_dir_all(path)?;
        cleaned.push(path.to_path_buf());
    }
    Ok(())
}

fn is_under(path: &Path, root: &Path) -> bool {
    path.canonicalize()
        .ok()
        .zip(root.canonicalize().ok())
        .is_some_and(|(p, r)| p.starts_with(&r))
}

fn discover_bundles(root: &Path) -> Result<Vec<PathBuf>, SyncError> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if SKIP_DIR_NAMES
            .iter()
            .any(|skip| name.eq_ignore_ascii_case(skip))
        {
            continue;
        }
        if path.join("SKILL.md").is_file() {
            out.push(path);
        }
    }
    Ok(out)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), SyncError> {
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

/// ลบ skill ออกจากคลัง (โฟลเดอร์ใต้ `skills/`)
pub fn remove_skill(name: &str) -> Result<(), SyncError> {
    let dest = skills_dir().join(name);
    if !dest.is_dir() {
        return Err(SyncError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("skill `{name}` not in library"),
        )));
    }
    std::fs::remove_dir_all(&dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_removes_identical_cli_copy() {
        let home = std::env::temp_dir().join(format!("mali-gate-clean-{}", std::process::id()));
        let external = home.join("cli-skills");
        let lib = home.join("skills");
        let skill_dir = external.join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let body = "---\nname: demo-skill\ndescription: demo\n---\n";
        std::fs::write(skill_dir.join("SKILL.md"), body).unwrap();
        std::fs::create_dir_all(lib.join("demo-skill")).unwrap();
        std::fs::write(lib.join("demo-skill/SKILL.md"), body).unwrap();
        unsafe {
            std::env::set_var("MALI_GATE_HOME", &home);
        }
        let report = clean_cli_skills(&CleanOptions {
            dry_run: false,
            by_name: false,
            roots: vec![external.clone()],
        })
        .expect("clean");
        assert_eq!(report.removed.len(), 1);
        assert!(!skill_dir.exists());
        unsafe {
            std::env::remove_var("MALI_GATE_HOME");
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn sync_imports_from_temp_root() {
        let home = std::env::temp_dir().join(format!("mali-gate-sync-{}", std::process::id()));
        let external = home.join("external");
        std::fs::create_dir_all(&external).unwrap();
        let skill_dir = external.join("demo-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: demo\n---\n",
        )
        .unwrap();
        unsafe {
            std::env::set_var("MALI_GATE_HOME", &home);
        }
        // library path via hub_core
        let lib_skills = home.join("skills");
        std::fs::create_dir_all(&lib_skills).unwrap();

        let report = sync_skills(&SyncOptions {
            dry_run: false,
            force: false,
            clean_sources: false,
            roots: vec![external.clone()],
        })
        .expect("sync");
        assert_eq!(report.imported, vec!["demo-skill".to_string()]);
        assert!(lib_skills.join("demo-skill/SKILL.md").is_file());
        unsafe {
            std::env::remove_var("MALI_GATE_HOME");
        }
        let _ = std::fs::remove_dir_all(home);
    }
}
