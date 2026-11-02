//! อ่านและตรวจ SKILL.md — YAML frontmatter ขยี้ `name` กับ `description` เท่านั้น
//!
//! รูปแบบที่ยอมรับ:
//! ```text
//! ---
//! name: weekly-report
//! description: สรุปรายงาน...
//! ---
//! (markdown เนื้อหา ด้านหลังอ่านภายหลังเมื่อ agent เรียก skills_load)
//! ```
//! ห้ามมี `unwrap()` เพราะเป็น library code (ข้อตกลงใน CLAUDE.md)

use std::path::{Path, PathBuf};

use hub_core::path_within_root;
use serde::Deserialize;

#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    #[error("failed to read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("missing YAML frontmatter starting with '---' on the first line")]
    MissingFrontmatter,
    #[error("invalid YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("SKILL.md must contain a non-empty name and description in its frontmatter")]
    MissingField,
    #[error("skill file path error: {0}")]
    Path(#[from] hub_core::PathError),
    #[error("skill not found: {0}")]
    NotFound(String),
}

/// ข้อมูลที่จำเป็นสำหรับ list/search/ตรวจสิทธิ์ (ยังไม่โหลดเนื้อหา markdown)
#[derive(Debug, Clone, PartialEq)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    /// โฟลเดอร์ skill ที่ไฟล์นี้อยู่ — ให้ตัวอ่านไฟล์ประกอบใช้เป็นเขตห้ามผ่าน (กัน `../`)
    pub sk_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// อ่านไฟล์ SKILL.md หนึ่งไฟล์ → `SkillManifest`
pub fn parse_skill_file(path: &Path) -> Result<SkillManifest, ParseError> {
    let raw = std::fs::read_to_string(path)?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let fm = parse_frontmatter(raw)?;
    let name = fm.name.unwrap_or_default().trim().to_string();
    let description = fm.description.unwrap_or_default().trim().to_string();
    if name.is_empty() || description.is_empty() {
        return Err(ParseError::MissingField);
    }
    let sk_dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    Ok(SkillManifest {
        name,
        description,
        sk_dir,
    })
}

/// อ่านเนื้อหา SKILL.md ทั้งไฟล์ (frontmatter + body)
pub fn read_skill_md(sk_dir: &Path) -> Result<String, ParseError> {
    let path = sk_dir.join("SKILL.md");
    let raw = std::fs::read_to_string(path)?;
    Ok(raw.strip_prefix('\u{feff}').unwrap_or(&raw).to_string())
}

/// รายการพาธไฟล์ใต้ `references/` (relative จากโฟลเดอร์ skill)
pub fn list_reference_files(sk_dir: &Path) -> Result<Vec<String>, ParseError> {
    let refs = sk_dir.join("references");
    if !refs.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    walk_reference_dir(&refs, &refs, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_reference_dir(
    refs_root: &Path,
    dir: &Path,
    out: &mut Vec<String>,
) -> Result<(), ParseError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_reference_dir(refs_root, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(refs_root)
                .map_err(|_| hub_core::PathError::EscapesRoot)?;
            let mut rel_str = rel.to_string_lossy().into_owned();
            if cfg!(windows) {
                rel_str = rel_str.replace('\\', "/");
            }
            out.push(format!("references/{rel_str}"));
        }
    }
    Ok(())
}

/// อ่านไฟล์ประกอบใน skill — `relative` ต้องอยู่ใต้โฟลเดอร์ skill เท่านั้น
pub fn read_skill_file(sk_dir: &Path, relative: &str) -> Result<String, ParseError> {
    let safe = path_within_root(sk_dir, Path::new(relative))?;
    Ok(std::fs::read_to_string(safe)?)
}

fn parse_frontmatter(raw: &str) -> Result<Frontmatter, ParseError> {
    let mut lines = raw.lines();
    if lines.next().map(|l| l.trim()) != Some("---") {
        return Err(ParseError::MissingFrontmatter);
    }
    let mut fm_text = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        fm_text.push_str(line);
        fm_text.push('\n');
    }
    if !closed {
        return Err(ParseError::MissingFrontmatter);
    }
    let fm: Frontmatter = serde_yaml::from_str(&fm_text)?;
    Ok(fm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_skill_without_frontmatter() {
        let err = parse_frontmatter("# hello\nno markers").unwrap_err();
        assert!(matches!(err, ParseError::MissingFrontmatter));
    }

    #[test]
    fn rejects_unclosed_frontmatter() {
        let err = parse_frontmatter("---\nname: x").unwrap_err();
        assert!(matches!(err, ParseError::MissingFrontmatter));
    }

    #[test]
    fn rejects_missing_fields() {
        let fm = parse_frontmatter("---\ndescription: ok\n---\n").unwrap();
        let name = fm.name.unwrap_or_default().trim().to_string();
        assert!(
            name.is_empty(),
            "เคส name หายต้องทำให้ parse_skill_file ตอบ MissingField (โครงเปล่าที่ทดสอบ)"
        );
    }
}
