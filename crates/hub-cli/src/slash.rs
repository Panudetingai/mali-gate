//! คำสั่ง `/…` ใน Mali Gate shell — เมนู, prefix match, parse

use std::io::{self, Write};

use crossterm::cursor;
use crossterm::queue;
use crossterm::terminal::{Clear, ClearType};

use crate::{Palette, RULE, ReplCmd};

/// คำสั่ง slash หนึ่งรายการ (ชื่อย่อได้หลายแบบ)
struct SlashDef {
    names: &'static [&'static str],
    about: &'static str,
    usage: Option<&'static str>,
}

const COMMANDS: &[SlashDef] = &[
    SlashDef {
        names: &["help", "?"],
        about: "show command list",
        usage: None,
    },
    SlashDef {
        names: &["list", "ls"],
        about: "list indexed skills",
        usage: None,
    },
    SlashDef {
        names: &["refresh", "reindex"],
        about: "re-index the skills folder",
        usage: None,
    },
    SlashDef {
        names: &["search", "find", "s"],
        about: "full-text search",
        usage: Some("<query>"),
    },
    SlashDef {
        names: &["mcp", "serve"],
        about: "MCP / serve status",
        usage: None,
    },
    SlashDef {
        names: &["init", "setup"],
        about: "scan & configure Cursor / Claude MCP",
        usage: None,
    },
    SlashDef {
        names: &["logs", "log"],
        about: "show recent agent events",
        usage: None,
    },
    SlashDef {
        names: &["banner"],
        about: "show startup banner",
        usage: None,
    },
    SlashDef {
        names: &["clear", "cls"],
        about: "clear the screen",
        usage: None,
    },
    SlashDef {
        names: &["exit", "quit", "q"],
        about: "leave the shell",
        usage: None,
    },
];

fn primary_name(def: &SlashDef) -> &'static str {
    def.names[0]
}

fn display_cmd(def: &SlashDef) -> String {
    match def.usage {
        Some(arg) => format!("/{} {arg}", primary_name(def)),
        None => format!("/{}", primary_name(def)),
    }
}

fn filtered_commands(prefix: Option<&str>) -> Vec<(usize, &'static SlashDef)> {
    let p = prefix.unwrap_or("").to_ascii_lowercase();
    COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, def)| {
            p.is_empty()
                || def
                    .names
                    .iter()
                    .any(|n| n.starts_with(p.as_str()) || p.starts_with(n))
        })
        .collect()
}

/// ผลเมื่อผู้ใช้เลือกคำสั่งจาก select
pub enum CommandPick {
    /// ส่งบรรทัดนี้เข้า parser ทันที
    Run(String),
    /// ต้องพิมพ์ต่อ (เช่น query ของ /search)
    ContinueTyping(String),
}

fn pick_command(def: &SlashDef) -> CommandPick {
    if def.usage.is_some() {
        CommandPick::ContinueTyping(format!("/{} ", primary_name(def)))
    } else {
        CommandPick::Run(display_cmd(def))
    }
}

/// จำนวนรายการหลังกรอง (select UI)
pub fn filtered_count(prefix: Option<&str>) -> usize {
    filtered_commands(prefix).len()
}

/// เลือกคำสั่งตาม index ในรายการที่กรองแล้ว
pub fn pick_filtered(prefix: Option<&str>, index: usize) -> Option<CommandPick> {
    filtered_commands(prefix)
        .into_iter()
        .nth(index)
        .map(|(_, def)| pick_command(def))
}

/// ชื่อคำสั่งหลักสำหรับ Tab complete
pub fn tab_complete_name(prefix: Option<&str>, index: usize) -> Option<String> {
    filtered_commands(prefix)
        .into_iter()
        .nth(index)
        .map(|(_, def)| primary_name(def).to_string())
}

fn truncate_plain(text: &str, max_cols: usize) -> String {
    if text.chars().count() <= max_cols {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_cols.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// วาด select list ใต้ input — ข้อความ plain (ไม่พึ่ง ANSI) ให้มองเห็นบน Windows VT
pub fn draw_select_menu(
    _paint: &Palette,
    prefix: Option<&str>,
    selected: usize,
    input_row: u16,
) -> io::Result<(u16, bool)> {
    let defs: Vec<&SlashDef> = filtered_commands(prefix)
        .into_iter()
        .map(|(_, d)| d)
        .collect();
    let width = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(100);
    let max_cols = width.saturating_sub(2);

    let mut lines: Vec<String> = Vec::new();
    lines.push(truncate_plain(
        "  ↑↓ move · Enter pick · type to filter · Esc back",
        max_cols,
    ));
    lines.push(truncate_plain(
        "  ────────────────────────────────────────",
        max_cols,
    ));

    if defs.is_empty() {
        lines.push(truncate_plain(
            &format!("  (no match for {:?})", prefix.unwrap_or("")),
            max_cols,
        ));
    } else {
        let sel = selected.min(defs.len().saturating_sub(1));
        for (i, def) in defs.iter().enumerate() {
            let cmd = display_cmd(def);
            let body = truncate_plain(
                &format!("{cmd}  —  {}", def.about),
                max_cols.saturating_sub(4),
            );
            let line = if i == sel {
                format!("> {body}")
            } else {
                format!("  {body}")
            };
            lines.push(truncate_plain(&line, max_cols));
        }
    }

    let n = lines.len() as u16;
    // เหนือ prompt ถ้าพอที่ — ไม่งั้นวาดใต้ (กัน saturating_sub ทับแถว 0)
    let draw_above = n > 0 && n <= input_row;
    let mut stdout = io::stdout();
    for (i, line) in lines.iter().enumerate() {
        let row = if draw_above {
            input_row - n + i as u16
        } else {
            input_row.saturating_add(1 + i as u16)
        };
        queue!(
            stdout,
            cursor::MoveTo(0, row),
            Clear(ClearType::CurrentLine)
        )?;
        print!("{line}");
    }
    stdout.flush()?;
    Ok((n, draw_above))
}

/// แสดงเมนูตัวเลือก — `prefix` กรองชื่อที่ขึ้นต้นตรง (เช่น `/s` → search, serve, setup)
pub fn print_command_menu(paint: &Palette, prefix: Option<&str>) {
    let rows = filtered_commands(prefix);

    if rows.is_empty() {
        println!(
            "  {} no commands match {:?} — try {}",
            paint.soft("hint:"),
            prefix.unwrap_or(""),
            paint.heading("/help")
        );
        return;
    }

    let title = if prefix.unwrap_or("").is_empty() {
        "pick a command (number or name after /)"
    } else {
        "matching commands"
    };
    println!("  {}", paint.heading(title));
    println!("  {}", paint.quiet(RULE));
    for (i, def) in &rows {
        let aliases = def.names.join(", ");
        println!(
            "  {}. {}  {}  ({})",
            paint.quiet(&((i + 1).to_string())),
            paint.heading(&display_cmd(def)),
            paint.soft(def.about),
            paint.quiet(&aliases)
        );
    }
    println!(
        "  {}",
        paint.soft("Enter 1–N, a name (e.g. list), or a full line like /search pdf")
    );
}

/// อ่านบรรทัดถัดจาก `/` — ตัวเลข, ชื่อ, หรือ `/command …`
pub fn read_menu_pick(paint: &Palette, prefix: Option<&str>) -> io::Result<Option<String>> {
    print!("{}", paint.heading("mali_gate> /"));
    io::stdout().flush()?;
    let mut line = String::new();
    let n = io::stdin().read_line(&mut line)?;
    if n == 0 {
        return Ok(None);
    }
    let pick = line.trim();
    if pick.is_empty() {
        return Ok(None);
    }
    if pick.starts_with('/') {
        return Ok(Some(pick.to_string()));
    }
    if let Ok(num) = pick.parse::<usize>() {
        let p = prefix.unwrap_or("").to_ascii_lowercase();
        let filtered: Vec<_> = COMMANDS
            .iter()
            .filter(|def| {
                p.is_empty()
                    || def
                        .names
                        .iter()
                        .any(|name| name.starts_with(p.as_str()) || p.starts_with(name))
            })
            .collect();
        if (1..=filtered.len()).contains(&num) {
            let def = filtered[num - 1];
            return Ok(Some(display_cmd(def)));
        }
    }
    Ok(Some(format!("/{pick}")))
}

pub fn print_repl_help(paint: &Palette) {
    println!("  {}", paint.heading("slash commands"));
    println!("  {}", paint.quiet(RULE));
    for def in COMMANDS {
        let aliases = def
            .names
            .iter()
            .map(|n| format!("/{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {}  {}  ({})",
            paint.heading(&display_cmd(def)),
            paint.soft(def.about),
            paint.quiet(&aliases)
        );
    }
    println!();
    println!(
        "  {} type {} for an interactive picker",
        paint.soft("Tip:"),
        paint.heading("/")
    );
}

fn defs_matching_head(head: &str) -> Vec<&'static SlashDef> {
    let h = head.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|def| def.names.iter().any(|&n| n == h))
        .collect()
}

fn defs_prefix_head(head: &str) -> Vec<&'static SlashDef> {
    let h = head.to_ascii_lowercase();
    if h.is_empty() {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .filter(|def| def.names.iter().any(|n| n.starts_with(h.as_str())))
        .collect()
}

fn map_def_to_cmd(def: &SlashDef, tail: &str) -> ReplCmd {
    let primary = primary_name(def);
    match primary {
        "help" | "?" => ReplCmd::Help,
        "banner" => ReplCmd::Banner,
        "list" | "ls" => ReplCmd::List,
        "refresh" | "reindex" => ReplCmd::Refresh,
        "search" | "find" | "s" => {
            if tail.is_empty() {
                ReplCmd::Usage("usage: /search <query>")
            } else {
                ReplCmd::Search {
                    query: tail.to_string(),
                }
            }
        }
        "mcp" | "serve" => ReplCmd::Mcp,
        "init" | "setup" => ReplCmd::Init,
        "logs" | "log" => ReplCmd::Logs,
        "clear" | "cls" => ReplCmd::Clear,
        "exit" | "quit" | "q" => ReplCmd::Exit,
        _ => ReplCmd::Unknown(primary.to_string()),
    }
}

/// แปลงบรรทัดที่ผู้ใช้พิมพ์ — ต้องขึ้นต้นด้วย `/`
pub fn parse_repl_line(line: &str) -> ReplCmd {
    let line = line.trim();
    if line.is_empty() {
        return ReplCmd::Noop;
    }
    if !line.starts_with('/') {
        return ReplCmd::Usage("commands start with / — type / for menu");
    }
    let rest = line[1..].trim();
    let (head, tail) = rest
        .split_once(|c: char| c.is_whitespace())
        .map_or((rest, ""), |(h, t)| (h, t.trim()));

    if head.is_empty() {
        return ReplCmd::CommandMenu { prefix: None };
    }

    let exact = defs_matching_head(head);
    if exact.len() == 1 {
        return map_def_to_cmd(exact[0], tail);
    }
    if exact.len() > 1 {
        return ReplCmd::CommandMenu {
            prefix: Some(head.to_string()),
        };
    }

    let pref = defs_prefix_head(head);
    match pref.len() {
        0 => ReplCmd::Unknown(head.to_string()),
        1 => map_def_to_cmd(pref[0], tail),
        _ => ReplCmd::CommandMenu {
            prefix: Some(head.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_slash_opens_menu() {
        assert!(matches!(
            parse_repl_line("/"),
            ReplCmd::CommandMenu { prefix: None }
        ));
    }

    #[test]
    fn prefix_resolves_unique_command() {
        assert!(matches!(parse_repl_line("/ban"), ReplCmd::Banner));
        assert!(matches!(parse_repl_line("/li"), ReplCmd::List));
    }

    #[test]
    fn ambiguous_prefix_opens_menu() {
        assert!(matches!(
            parse_repl_line("/se"),
            ReplCmd::CommandMenu { .. }
        ));
    }
}
