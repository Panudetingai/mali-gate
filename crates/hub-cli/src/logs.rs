//! `mali_gate logs` — plain text หรือ TUI (`--follow`)

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

use hub_core::{db_path, user_data_root};
use hub_log::{Decision, EventKind, EventRow, LogQuery, Logger};
use hub_tui::{LogViewerConfig, run_log_viewer};

use crate::Palette;

/// ตัวเลือกคำสั่ง `logs`
pub struct LogsOptions {
    pub limit: usize,
    pub agent: Option<String>,
    pub kind: Option<EventKind>,
    pub decision: Option<Decision>,
    pub follow: bool,
    pub export: Option<PathBuf>,
    /// บังคับ plain text แม้มี `--follow`
    pub plain: bool,
}

pub fn run_logs(opts: LogsOptions, paint: &Palette) -> anyhow::Result<()> {
    let logger = Logger::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let query = LogQuery {
        limit: opts.limit,
        agent: opts.agent,
        kind: opts.kind,
        decision: opts.decision,
        after_id: None,
    };

    if let Some(path) = &opts.export {
        let file = std::fs::File::create(path)
            .map_err(|e| anyhow::anyhow!("create export file {}: {e}", path.display()))?;
        let n = logger
            .export_jsonl(&query, file)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        println!(
            "  {}",
            paint.heading(&format!("exported {n} event(s) to {}", path.display()))
        );
        return Ok(());
    }

    let tty = io::stdout().is_terminal();
    if opts.follow && !opts.plain {
        if !tty {
            anyhow::bail!("`logs --follow` needs a terminal; use `--plain` for streaming text");
        }
        let active_db = db_path();
        run_log_viewer(LogViewerConfig {
            db_path: active_db.clone(),
            query,
            follow: true,
            empty_hint: empty_db_hint(&active_db),
        })?;
        return Ok(());
    }

    if opts.follow && opts.plain {
        run_plain_follow(&logger, query, paint)?;
        return Ok(());
    }

    let rows = logger.query(&query).map_err(|e| anyhow::anyhow!("{e}"))?;
    print_plain(&query, rows, paint)
}

fn empty_db_hint(active_db: &std::path::Path) -> Option<String> {
    let user_db = user_data_root().join("mali_gate.db");
    if active_db == user_db.as_path() || !user_db.is_file() {
        return None;
    }
    let active = Logger::open(active_db).ok()?;
    let has_local = !active
        .query(&LogQuery {
            limit: 1,
            ..LogQuery::default()
        })
        .ok()?
        .is_empty();
    if has_local {
        return None;
    }
    let user = Logger::open(&user_db).ok()?;
    let n = user.max_event_id().ok()?;
    if n == 0 {
        return None;
    }
    Some(format!(
        "DB นี้ว่าง: {}\n\nมี {n} event อยู่ที่:\n{}\n\nใน PowerShell รัน:\n  Remove-Item Env:MALI_GATE_HOME -ErrorAction SilentlyContinue\nแล้วเปิด logs --follow ใหม่",
        active_db.display(),
        user_db.display()
    ))
}

fn print_plain(query: &LogQuery, rows: Vec<EventRow>, paint: &Palette) -> anyhow::Result<()> {
    if rows.is_empty() {
        let active = db_path();
        println!(
            "  {}",
            paint.soft("no events yet — use an agent with `mali_gate serve`, then run logs again")
        );
        println!("  {}", paint.quiet(&format!("db: {}", active.display())));
        if let Some(hint) = empty_db_hint(&active) {
            println!();
            for line in hint.lines() {
                println!("  {}", paint.heading(line));
            }
        }
        return Ok(());
    }
    print_section(query.limit, rows.len(), paint);
    for row in rows {
        print_row(&row, paint);
    }
    Ok(())
}

#[allow(clippy::never_loop)] // intentional: runs until the process is interrupted
fn run_plain_follow(logger: &Logger, mut query: LogQuery, paint: &Palette) -> anyhow::Result<()> {
    let mut last_id = logger.max_event_id().map_err(|e| anyhow::anyhow!("{e}"))?;
    query.after_id = None;
    let rows = logger.query(&query).map_err(|e| anyhow::anyhow!("{e}"))?;
    print_plain(&query, rows, paint)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        query.after_id = Some(last_id);
        query.limit = 100;
        let fresh = logger.query(&query).map_err(|e| anyhow::anyhow!("{e}"))?;
        if fresh.is_empty() {
            continue;
        }
        for row in fresh.iter().rev() {
            print_row(row, paint);
            let _ = out.flush();
            if row.id > last_id {
                last_id = row.id;
            }
        }
    }
}

fn print_section(limit: usize, shown: usize, paint: &Palette) {
    println!(
        "  {}",
        paint.heading(&format!("events (showing {shown} of last {limit})"))
    );
    println!("  {}", paint.quiet(crate::RULE));
}

fn print_row(row: &EventRow, paint: &Palette) {
    let decision = row.decision.as_ref().map(|d| d.as_str()).unwrap_or("-");
    let status = row.result_status.as_deref().unwrap_or("-");
    let ms = row
        .duration_ms
        .map(|n| format!("{n}ms"))
        .unwrap_or_else(|| "-".into());
    println!(
        "  {}  {}  {}  {}  {}  {}  {}",
        paint.quiet(&row.ts),
        paint.heading(&row.agent),
        paint.heading(row.kind.as_str()),
        paint.heading(&row.tool),
        paint.quiet(decision),
        status_label(status, paint),
        paint.quiet(&ms),
    );
    if !row.target.is_empty() {
        println!("      target: {}", paint.quiet(&row.target));
    }
    if row.args.as_object().is_some_and(|m| !m.is_empty()) {
        let args = row.args.to_string();
        let args = if args.len() > 120 {
            format!("{}…", &args[..120])
        } else {
            args
        };
        println!("      args:   {}", paint.quiet(&args));
    }
    if let Some(err) = &row.error {
        println!("      error:  {}", paint.heading(err));
    }
    println!();
}

fn status_label(status: &str, paint: &Palette) -> colored::ColoredString {
    if status == "ok" || status == "error" {
        paint.heading(status)
    } else {
        paint.quiet(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hub_log::{Decision, Event, EventKind, Logger};
    use serde_json::json;

    #[test]
    fn run_logs_shows_written_event() {
        let path = std::env::temp_dir().join(format!(
            "mali-gate-logs-test-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let logger = Logger::open(&path).unwrap();
        logger
            .write(&Event {
                agent: "test-agent".into(),
                session_id: "s1".into(),
                kind: EventKind::SkillLoad,
                target: "demo-skill".into(),
                tool: "skills_load".into(),
                args: json!({ "name": "demo-skill" }),
                decision: Some(Decision::Allow),
                result_status: Some("ok".into()),
                duration_ms: Some(3),
                error: None,
            })
            .unwrap();

        let home = path
            .parent()
            .unwrap()
            .join(format!("mali-gate-home-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let db_in_home = home.join("mali_gate.db");
        std::fs::copy(&path, &db_in_home).unwrap();
        unsafe {
            std::env::set_var("MALI_GATE_HOME", &home);
        }

        let paint = Palette { color: false };
        run_logs(
            LogsOptions {
                limit: 10,
                agent: None,
                kind: None,
                decision: None,
                follow: false,
                export: None,
                plain: false,
            },
            &paint,
        )
        .expect("logs command");

        unsafe {
            std::env::remove_var("MALI_GATE_HOME");
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
