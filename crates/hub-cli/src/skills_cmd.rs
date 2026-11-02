//! `mali_gate skills` — sync, remove, show

use hub_core::{db_path, skills_dir};
use hub_registry::index::SkillIndex;
use hub_registry::{
    CleanOptions, SyncOptions, clean_cli_skills, default_sync_roots, remove_skill, sync_skills,
};

use crate::{Palette, print_hit, print_section};

pub fn run_skills_show(name: &str, paint: &Palette) -> anyhow::Result<()> {
    let index = SkillIndex::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let hit = index
        .get_by_name(name)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("skill `{name}` not in index — try `skills refresh`"))?;
    print_section("skill", 1, paint);
    print_hit(&hit, paint);
    let origin = hit.dir.join(".mali-gate-origin.json");
    if origin.is_file() {
        println!(
            "      {}",
            paint.quiet(&format!("install pin: {}", origin.display()))
        );
    }
    Ok(())
}

pub fn run_skills_remove(name: &str, yes: bool, paint: &Palette) -> anyhow::Result<()> {
    if !yes {
        anyhow::bail!("refusing to remove `{name}` without `--yes`");
    }
    remove_skill(name).map_err(|e| anyhow::anyhow!("{e}"))?;
    let index = SkillIndex::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let _ = index.delete_by_name(name);
    println!(
        "  {}",
        paint.heading(&format!("removed `{name}` from {}", skills_dir().display()))
    );
    Ok(())
}

pub fn run_skills_clean(
    dry_run: bool,
    by_name: bool,
    yes: bool,
    paint: &Palette,
) -> anyhow::Result<()> {
    if by_name && !dry_run && !yes {
        anyhow::bail!("`--by-name` removes CLI copies even when content differs — add `--yes`");
    }
    let roots = default_sync_roots();
    println!("  {}", paint.heading("skills clean (AI CLI folders)"));
    for r in &roots {
        if r.is_dir() {
            println!("  {}", paint.quiet(&format!("scan: {}", r.display())));
        }
    }
    let report = clean_cli_skills(&CleanOptions {
        dry_run,
        by_name,
        roots: Vec::new(),
    })
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    if dry_run {
        println!("  {}", paint.soft("(dry run — no files deleted)"));
    }
    if !report.removed.is_empty() {
        println!(
            "  {}",
            paint.heading(&format!("removed from CLI ({})", report.removed.len()))
        );
        for e in &report.removed {
            println!(
                "      {} ← {}",
                e.name,
                paint.quiet(&e.source.display().to_string())
            );
        }
    }
    if !report.kept_different.is_empty() {
        println!("  {}", paint.heading("kept (content differs from library)"));
        for c in &report.kept_different {
            println!(
                "      {} — {} ({})",
                c.name,
                paint.quiet(&c.source.display().to_string()),
                c.reason
            );
        }
    }
    if !report.not_in_library.is_empty() {
        println!(
            "  {}",
            paint.soft(&format!(
                "not in mali_gate library ({}) — left on disk",
                report.not_in_library.len()
            ))
        );
        for n in &report.not_in_library {
            println!(
                "      {} ← {}",
                n.folder,
                paint.quiet(&n.source.display().to_string())
            );
        }
    }
    for err in &report.parse_errors {
        eprintln!("  parse error: {err}");
    }
    if report.removed.is_empty()
        && report.kept_different.is_empty()
        && report.not_in_library.is_empty()
    {
        println!("  {}", paint.soft("nothing to clean"));
    }
    Ok(())
}

pub fn run_skills_sync(
    dry_run: bool,
    force: bool,
    clean_sources: bool,
    paint: &Palette,
) -> anyhow::Result<()> {
    let roots = default_sync_roots();
    println!("  {}", paint.heading("skills sync"));
    for r in &roots {
        if r.is_dir() {
            println!("  {}", paint.quiet(&format!("scan: {}", r.display())));
        }
    }
    let report = sync_skills(&SyncOptions {
        dry_run,
        force,
        clean_sources,
        roots: Vec::new(),
    })
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    if dry_run {
        println!("  {}", paint.soft("(dry run — no files changed)"));
    }
    if !report.imported.is_empty() {
        println!(
            "  {}",
            paint.heading(&format!("imported ({})", report.imported.len()))
        );
        for n in &report.imported {
            println!("      {n}");
        }
    }
    if !report.skipped_identical.is_empty() {
        println!(
            "  {}",
            paint.soft(&format!(
                "already in library ({})",
                report.skipped_identical.len()
            ))
        );
        for s in &report.skipped_identical {
            println!(
                "      {} ← {}",
                s.name,
                paint.quiet(&s.source.display().to_string())
            );
        }
    }
    if !report.conflicts.is_empty() {
        println!("  {}", paint.heading("conflicts"));
        for c in &report.conflicts {
            println!(
                "      {} — {} ({})",
                c.name,
                paint.quiet(&c.source.display().to_string()),
                c.reason
            );
        }
    }
    if !report.cleaned_sources.is_empty() {
        println!(
            "  {}",
            paint.heading(&format!(
                "removed duplicate sources ({})",
                report.cleaned_sources.len()
            ))
        );
        for p in &report.cleaned_sources {
            println!("      {}", paint.quiet(&p.display().to_string()));
        }
    }
    for err in &report.parse_errors {
        eprintln!("  parse error: {err}");
    }

    if !dry_run && (!report.imported.is_empty() || !report.cleaned_sources.is_empty()) {
        refresh_skills_index(paint)?;
    } else if report.imported.is_empty()
        && report.skipped_identical.is_empty()
        && report.conflicts.is_empty()
    {
        println!("  {}", paint.soft("nothing to sync"));
    }
    Ok(())
}

fn refresh_skills_index(paint: &Palette) -> anyhow::Result<()> {
    let index = SkillIndex::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let n = index
        .index_dir(&skills_dir())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("  {}", paint.heading(&format!("indexed {n} skills")));
    Ok(())
}
