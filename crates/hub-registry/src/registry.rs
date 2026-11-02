//! index skill ภายนอก (JSON) สำหรับแนะนำเมื่อค้นในเครื่องไม่เจอ

use serde::Deserialize;
use serde_json::Value;

use crate::config::RegistrySection;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RegistryEntry {
    pub name: String,
    pub description: String,
    pub source: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    skills: Vec<RegistryEntry>,
}

#[derive(thiserror::Error, Debug)]
pub enum RegistryError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("fetch: {0}")]
    Fetch(String),
}

pub fn load_entries(section: &RegistrySection) -> Result<Vec<RegistryEntry>, RegistryError> {
    let mut all = Vec::new();
    for file in &section.files {
        let text = std::fs::read_to_string(file)?;
        all.extend(parse_registry_text(&text)?);
    }
    for url in &section.urls {
        all.extend(fetch_registry_url(url)?);
    }
    Ok(all)
}

fn parse_registry_text(text: &str) -> Result<Vec<RegistryEntry>, RegistryError> {
    let trimmed = text.trim();
    if trimmed.starts_with('[') {
        return Ok(serde_json::from_str(trimmed)?);
    }
    let file: RegistryFile = serde_json::from_str(trimmed)?;
    Ok(file.skills)
}

fn fetch_registry_url(url: &str) -> Result<Vec<RegistryEntry>, RegistryError> {
    let body = ureq::get(url)
        .call()
        .map_err(|e| RegistryError::Fetch(e.to_string()))?
        .into_body()
        .read_to_string()
        .map_err(|e| RegistryError::Fetch(e.to_string()))?;
    parse_registry_text(&body)
}

pub fn search_registry(entries: &[RegistryEntry], query: &str) -> Vec<RegistryEntry> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    entries
        .iter()
        .filter(|e| {
            e.name.to_ascii_lowercase().contains(&q)
                || e.description.to_ascii_lowercase().contains(&q)
                || e.tags.iter().any(|t| t.to_ascii_lowercase().contains(&q))
        })
        .cloned()
        .collect()
}

/// รวม local hits กับ registry suggestions สำหรับ agent
pub fn merge_search_json(
    local_json: &str,
    registry_hits: &[RegistryEntry],
) -> Result<String, RegistryError> {
    let local: Value = serde_json::from_str(local_json).unwrap_or(Value::Array(vec![]));
    let suggestions: Vec<Value> = registry_hits
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": e.name,
                "description": e.description,
                "source": e.source,
                "installable": true,
                "location": "registry",
            })
        })
        .collect();
    let out = serde_json::json!({
        "local": local,
        "registry_suggestions": suggestions,
    });
    Ok(serde_json::to_string_pretty(&out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_by_name() {
        let entries = vec![RegistryEntry {
            name: "pdf-tools".into(),
            description: "work with pdf".into(),
            source: "file:///tmp/x".into(),
            tags: vec![],
        }];
        let hits = search_registry(&entries, "pdf");
        assert_eq!(hits.len(), 1);
    }
}
