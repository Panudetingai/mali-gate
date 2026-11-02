//! `mali_gate install` — CLI ติดตั้ง skill (allowlist + pin)

use hub_core::{config_path, db_path, skills_dir};
use hub_registry::index::SkillIndex;
use hub_registry::{PendingInstallStore, RegistryConfigFile, install_skill, parse_source};

use crate::Palette;

pub fn run_install(source: &str, force: bool, paint: &Palette) -> anyhow::Result<()> {
    let cfg = RegistryConfigFile::load_from_disk();
    let parsed = parse_source(source).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (name, pin) =
        install_skill(&parsed, &cfg.install, force).map_err(|e| anyhow::anyhow!("{e}"))?;
    reindex_one(paint)?;
    println!("  {}", paint.heading(&format!("installed skill `{name}`")));
    println!(
        "  {}",
        paint.quiet(&format!(
            "pin: {} · source: {}",
            pin.pinned_hash, pin.source
        ))
    );
    println!(
        "  {}",
        paint.quiet(&format!("path: {}", skills_dir().join(&name).display()))
    );
    Ok(())
}

pub fn run_install_pending(paint: &Palette) -> anyhow::Result<()> {
    let store = PendingInstallStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rows = store.list_open().map_err(|e| anyhow::anyhow!("{e}"))?;
    if rows.is_empty() {
        println!("  {}", paint.soft("no pending install requests"));
        return Ok(());
    }
    println!("  {}", paint.heading("pending installs"));
    for row in rows {
        println!(
            "  {}  {}  {}",
            paint.heading(&row.id.to_string()),
            paint.heading(&row.skill_name),
            paint.quiet(&row.source)
        );
        println!("      {}", paint.soft(&row.description));
        println!(
            "      {}",
            paint.quiet(&format!("pin: {} · agent: {}", row.pinned_hash, row.agent))
        );
        let preview: String = row.preview_md.chars().take(200).collect();
        println!("      {}", paint.quiet(&preview));
        println!(
            "      {}",
            paint.soft(&format!(
                "approve: mali_gate install approve {} --yes",
                row.id
            ))
        );
    }
    Ok(())
}

pub fn run_install_approve(id: i64, yes: bool, paint: &Palette) -> anyhow::Result<()> {
    let store = PendingInstallStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let row = store.get_open(id).map_err(|e| anyhow::anyhow!("{e}"))?;
    if !yes {
        store
            .resolve(id, "denied")
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("  {}", paint.heading(&format!("install #{id} denied")));
        return Ok(());
    }
    let cfg = RegistryConfigFile::load_from_disk();
    let parsed = parse_source(&row.source).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (name, pin) =
        install_skill(&parsed, &cfg.install, true).map_err(|e| anyhow::anyhow!("{e}"))?;
    store
        .resolve(id, "approved")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    reindex_one(paint)?;
    println!(
        "  {}",
        paint.heading(&format!("installed `{name}` from pending #{id}"))
    );
    println!(
        "  {}",
        paint.quiet(&format!(
            "pin: {} (preview was {})",
            pin.pinned_hash, row.pinned_hash
        ))
    );
    let _ = config_path();
    Ok(())
}

fn reindex_one(paint: &Palette) -> anyhow::Result<()> {
    let index = SkillIndex::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let n = index
        .index_dir(&skills_dir())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("  {}", paint.quiet(&format!("indexed {n} skills")));
    Ok(())
}
