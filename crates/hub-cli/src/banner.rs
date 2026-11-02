//! หน้าต้อนรับของ REPL: ดอกมะลิเล็ก + การ์ดสถานะ hub (ค่าสด) + คำสั่งถัดไป
//!
//! ทุกค่าสถานะอ่านแบบ "ไม่ล้ม": อ่านไม่ได้ → แสดง `—` แทน

use std::path::Path;

use hub_core::{config_path, data_root, db_path};
use hub_log::Logger;
use hub_policy::{AgentStore, ApprovalStore};
use hub_registry::PendingInstallStore;
use hub_registry::index::SkillIndex;

use crate::Palette;

/// ดอกมะลิเล็ก — `@` คือเกสร (สีเด่น) ส่วนอื่นเป็นกลีบ (สีจาง)
pub(crate) const FLOWER: &str = r"
     .-.
  .-(   )-.
 (   \ /   )
  >-( @ )-<
 (   / \   )
  '-(   )-'
     '-'
";

/// คอลัมน์ที่การ์ดสถานะเริ่ม (ดอกไม้กว้างสุด 13 ช่อง + ช่องว่าง)
const CARD_COL: usize = 17;
/// จอแคบกว่านี้ → การ์ดย้ายไปใต้ดอกไม้
pub(crate) const WIDE_MIN: usize = 64;
const LABEL_W: usize = 10;

/// สถานะ hub ณ ตอนเปิด REPL — `None` = อ่านไม่ได้
#[derive(Debug, Default, Clone)]
pub(crate) struct HubStatus {
    pub agents: Option<Vec<String>>,
    pub active_24h: Option<usize>,
    pub skills: Option<usize>,
    pub pending_installs: Option<usize>,
    pub approvals: Option<usize>,
    pub mcp_servers: Option<usize>,
}

impl HubStatus {
    pub(crate) fn collect() -> Self {
        let db = db_path();
        let db_ok = db.is_file();
        Self {
            agents: db_ok
                .then(|| AgentStore::open(&db).ok()?.list().ok())
                .flatten()
                .map(|rows| rows.into_iter().map(|r| r.id).collect()),
            active_24h: db_ok.then(|| active_agents_24h(&db)).flatten(),
            skills: db_ok
                .then(|| SkillIndex::open(&db).ok()?.list_all().ok())
                .flatten()
                .map(|v| v.len()),
            pending_installs: db_ok
                .then(|| PendingInstallStore::open(&db).ok()?.list_open().ok())
                .flatten()
                .map(|v| v.len()),
            approvals: db_ok
                .then(|| ApprovalStore::open(&db).ok()?.list_open().ok())
                .flatten()
                .map(|v| v.len()),
            mcp_servers: count_mcp_servers(&config_path()),
        }
    }
}

/// agent ที่มี event ใน 24 ชม. ล่าสุด (เทียบ timestamp RFC3339 UTC แบบ string)
fn active_agents_24h(db: &Path) -> Option<usize> {
    let cutoff = time::OffsetDateTime::now_utc() - time::Duration::hours(24);
    let cutoff = cutoff
        .format(&time::format_description::well_known::Rfc3339)
        .ok()?;
    let cutoff = cutoff.get(..19)?.to_string();
    let stats = Logger::open(db).ok()?.agent_usage_stats().ok()?;
    Some(
        stats
            .iter()
            .filter(|s| s.last_ts.get(..19).is_some_and(|t| t >= cutoff.as_str()))
            .count(),
    )
}

/// จำนวน `[mcp.servers.*]` ใน config.toml (ไม่มีไฟล์ = 0)
fn count_mcp_servers(path: &Path) -> Option<usize> {
    if !path.is_file() {
        return Some(0);
    }
    let text = std::fs::read_to_string(path).ok()?;
    let doc: toml::Value = toml::from_str(&text).ok()?;
    Some(
        doc.get("mcp")
            .and_then(|m| m.get("servers"))
            .and_then(|s| s.as_table())
            .map_or(0, |t| t.len()),
    )
}

fn num(v: Option<usize>) -> String {
    v.map_or_else(|| "—".into(), |n| n.to_string())
}

fn tilde(path: &Path) -> String {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    match home.and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

/// ตัดรายชื่อ agent ให้พอดี `max` ตัวอักษร
fn agent_names(names: &[String], max: usize) -> String {
    let mut out = String::new();
    for (i, n) in names.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        let rest = names.len() - i;
        if out.chars().count() + sep.len() + n.chars().count() + 5 > max && rest > 0 && i > 0 {
            out.push_str(", …");
            return out;
        }
        out.push_str(sep);
        out.push_str(n);
    }
    out
}

/// บรรทัดของการ์ดสถานะ (มีสีตาม palette)
/// `avail` = ความกว้างที่การ์ดใช้ได้ (หลังหักดอกไม้/ระยะขอบแล้ว)
fn card_lines(st: &HubStatus, paint: &Palette, avail: usize) -> Vec<String> {
    let label = |s: &str| paint.quiet(&format!("{s:<LABEL_W$}")).to_string();
    let agents_n = st.agents.as_ref().map(Vec::len);
    let mut agents = format!(
        "{}{} registered",
        label("agents"),
        paint.heading(&num(agents_n))
    );
    if let Some(active) = st.active_24h {
        agents.push_str(&paint.soft(&format!(" · {active} active 24h")).to_string());
    }

    let mut lines = vec![
        paint
            .flower(&format!("Mali Gate v{}", env!("CARGO_PKG_VERSION")))
            .to_string(),
        paint.soft("skill hub + MCP gateway").to_string(),
        String::new(),
        agents,
    ];
    if let Some(names) = st.agents.as_ref().filter(|n| !n.is_empty()) {
        let room = avail.saturating_sub(LABEL_W).max(20);
        lines.push(format!(
            "{}{}",
            " ".repeat(LABEL_W),
            paint.quiet(&agent_names(names, room))
        ));
    }
    lines.push(format!(
        "{}{} indexed{}",
        label("skills"),
        paint.heading(&num(st.skills)),
        paint.soft(&format!(" · {} pending installs", num(st.pending_installs)))
    ));
    let waiting = st.approvals.unwrap_or(0) > 0;
    let approvals = if waiting {
        paint.flower(&format!("{} waiting", num(st.approvals)))
    } else {
        paint.heading(&format!("{} waiting", num(st.approvals)))
    };
    lines.push(format!(
        "{}{}{}",
        label("approvals"),
        approvals,
        paint.soft(&format!(" · mcp servers {}", num(st.mcp_servers)))
    ));
    lines.push(format!(
        "{}{}",
        label("data"),
        paint.quiet(&tilde(&data_root()))
    ));
    lines
}

/// คำแนะนำถัดไปตามสถานะ (เช่น ยังไม่มี agent → /init)
fn next_steps(st: &HubStatus, paint: &Palette) -> Vec<String> {
    let mut out = Vec::new();
    if st.approvals.unwrap_or(0) > 0 {
        out.push(format!(
            "{} {}",
            paint.flower("!"),
            paint.soft("requests are waiting — run `mali_gate approve list`")
        ));
    }
    if st.pending_installs.unwrap_or(0) > 0 {
        out.push(format!(
            "{} {}",
            paint.flower("!"),
            paint.soft("skill installs are waiting — run `mali_gate install pending`")
        ));
    }
    if st.agents.as_ref().is_some_and(Vec::is_empty) {
        out.push(format!(
            "{} {}",
            paint.flower("→"),
            paint.soft("no agents yet — /init wires your AI CLIs")
        ));
    }
    if st.skills == Some(0) {
        out.push(format!(
            "{} {}",
            paint.flower("→"),
            paint.soft("skill index is empty — /refresh")
        ));
    }
    out
}

/// สร้างหน้าต้อนรับทั้งหมด (แยกจากการพิมพ์ เพื่อเทสต์ได้)
pub(crate) fn render(st: &HubStatus, paint: &Palette, width: usize) -> Vec<String> {
    let flower: Vec<&str> = FLOWER
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .map(str::trim_end)
        .collect();
    let paint_flower = |row: &str| -> String {
        match row.split_once('@') {
            Some((a, b)) => format!("{}{}{}", paint.soft(a), paint.flower("@"), paint.soft(b)),
            None => paint.soft(row).to_string(),
        }
    };
    let wide = width >= WIDE_MIN;
    let avail = if wide {
        width.saturating_sub(CARD_COL + 4)
    } else {
        width.saturating_sub(4)
    };
    let card = card_lines(st, paint, avail);
    let mut out = Vec::new();
    if wide {
        for i in 0..flower.len().max(card.len()) {
            let art = flower.get(i).copied().unwrap_or("");
            let pad = CARD_COL.saturating_sub(art.chars().count());
            let text = card.get(i).map(String::as_str).unwrap_or("");
            out.push(
                format!("  {}{}{text}", paint_flower(art), " ".repeat(pad))
                    .trim_end()
                    .to_string(),
            );
        }
    } else {
        out.extend(flower.iter().map(|r| format!("  {}", paint_flower(r))));
        out.push(String::new());
        out.extend(
            card.into_iter()
                .map(|l| format!("  {l}").trim_end().to_string()),
        );
    }
    let steps = next_steps(st, paint);
    if !steps.is_empty() {
        out.push(String::new());
        out.extend(steps.into_iter().map(|l| format!("  {l}")));
    }
    out.push(String::new());
    let hints = [
        ("/help", "commands"),
        ("/logs", "live log"),
        ("/init", "wire AI CLIs"),
        ("/exit", "leave"),
    ];
    let item = |(cmd, what): (&str, &str)| format!("{} {}", paint.heading(cmd), paint.quiet(what));
    if wide {
        out.push(format!(
            "  {}",
            hints.into_iter().map(item).collect::<Vec<_>>().join("  ")
        ));
    } else {
        // จอแคบ: 2 คำสั่งต่อบรรทัด
        for pair in hints.chunks(2) {
            out.push(format!(
                "  {}",
                pair.iter()
                    .copied()
                    .map(item)
                    .collect::<Vec<_>>()
                    .join("  ")
            ));
        }
    }
    out
}

pub(crate) fn print_banner(paint: &Palette) {
    let st = HubStatus::collect();
    for line in render(&st, paint, crate::terminal_width()) {
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Palette {
        Palette { color: false }
    }

    fn status() -> HubStatus {
        HubStatus {
            agents: Some(vec![
                "claude-code".into(),
                "cursor".into(),
                "opencode".into(),
            ]),
            active_24h: Some(2),
            skills: Some(5),
            pending_installs: Some(0),
            approvals: Some(0),
            mcp_servers: Some(1),
        }
    }

    #[test]
    fn flower_is_small_and_has_stamen() {
        let rows: Vec<&str> = FLOWER.lines().filter(|l| !l.trim().is_empty()).collect();
        assert!(rows.len() <= 8);
        assert!(FLOWER.contains('@'));
        assert!(rows.iter().all(|r| r.chars().count() < CARD_COL));
    }

    #[test]
    fn wide_puts_card_beside_flower() {
        let out = render(&status(), &plain(), 120);
        let title = out.iter().find(|l| l.contains("Mali Gate v")).unwrap();
        assert!(
            title.contains(".-."),
            "title should sit beside the flower: {title}"
        );
        let joined = out.join("\n");
        assert!(joined.contains("3 registered · 2 active 24h"), "{joined}");
        assert!(joined.contains("claude-code, cursor, opencode"), "{joined}");
        assert!(
            joined.contains("5 indexed · 0 pending installs"),
            "{joined}"
        );
        assert!(joined.contains("0 waiting · mcp servers 1"), "{joined}");
        assert!(joined.contains("/init"), "{joined}");
        assert!(
            !joined.contains("Cursor connects via"),
            "old tip must be gone"
        );
    }

    #[test]
    fn narrow_stacks_card_below() {
        let out = render(&status(), &plain(), 50);
        let title = out.iter().position(|l| l.contains("Mali Gate v")).unwrap();
        let stamen = out.iter().position(|l| l.contains('@')).unwrap();
        assert!(title > stamen);
        assert!(out.iter().all(|l| l.chars().count() <= 60), "{out:#?}");
    }

    #[test]
    fn unknown_values_show_dash_and_hints_appear() {
        let st = HubStatus {
            agents: Some(vec![]),
            skills: Some(0),
            approvals: Some(2),
            ..HubStatus::default()
        };
        let joined = render(&st, &plain(), 120).join("\n");
        assert!(joined.contains("— pending installs"), "{joined}");
        assert!(joined.contains("mcp servers —"), "{joined}");
        assert!(joined.contains("approve list"), "{joined}");
        assert!(joined.contains("/init wires"), "{joined}");
        assert!(joined.contains("/refresh"), "{joined}");
    }

    #[test]
    fn agent_names_truncate() {
        let names: Vec<String> = (0..10).map(|i| format!("agent-{i}")).collect();
        let s = agent_names(&names, 30);
        assert!(s.ends_with('…'), "{s}");
        assert!(s.chars().count() <= 34, "{s}");
    }

    /// ดูหน้าต้อนรับจากข้อมูลจริงในเครื่อง: `cargo test -p hub-cli --bin mali_gate preview_banner -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn preview_banner() {
        for w in [100, 50] {
            println!("--- width {w} ---");
            for line in render(&HubStatus::collect(), &plain(), w) {
                println!("{line}");
            }
        }
    }
}
