//! `mali_gate stats` — สรุปการใช้งานต่อ agent

use hub_core::db_path;
use hub_log::Logger;

use crate::Palette;

pub fn run_stats(paint: &Palette) -> anyhow::Result<()> {
    let logger = Logger::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rows = logger
        .stats_by_agent()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if rows.is_empty() {
        println!(
            "  {}",
            paint.soft("no events yet — use an agent with `mali_gate serve`")
        );
        return Ok(());
    }
    println!("  {}", paint.heading("usage by agent"));
    println!("  {}", paint.quiet(crate::RULE));
    println!(
        "  {}  {}  {}",
        paint.quiet("agent"),
        paint.quiet("events"),
        paint.quiet("last event"),
    );
    for row in rows {
        println!(
            "  {}  {}  {}",
            paint.heading(&row.agent),
            paint.quiet(&row.events.to_string()),
            paint.quiet(&row.last_ts),
        );
    }
    Ok(())
}
