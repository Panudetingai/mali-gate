// Binary `mali_gate` — CLI หลักของ Mali Gate
//
// Commands (phase 1):
//   mali_gate                  Interactive shell (slash commands, stays open)
//   mali_gate skills refresh   Re-index from the user's skills folder
//   mali_gate skills list      List all indexed skills
//   mali_gate skills search    Full-text search (Thai + English)
//   mali_gate serve            MCP server for Cursor / agents (stdio)
//   mali_gate logs             Recent agent tool events from the log DB
// Data path: `MALI_GATE_HOME` or `~/.mali_gate` (see hub-core::data_root)

use std::io::{self, IsTerminal};

use clap::{CommandFactory, Parser, Subcommand};
use colored::{ColoredString, Colorize};
use crossterm::terminal::{Clear, ClearType};
use crossterm::{cursor, execute, terminal};
use hub_core::{db_path, skills_dir};
use hub_log::{Decision, EventKind};
use hub_registry::index::{SearchHit, SkillIndex};

use banner::print_banner;
use hub_policy::GrantScope;

mod agent;
mod approve;
mod banner;
mod init;
mod install_cmd;
mod logs;
mod repl_input;
mod skills_cmd;
mod slash;
mod stats;

/// A quiet separator line between sections.
pub(crate) const RULE: &str = "──────────────────────────────────────────";

/// Choose colours only when stdout is a real terminal;
/// keep output plain when it is piped (safe for scripts).
pub(crate) struct Palette {
    color: bool,
}

impl Palette {
    /// Detect from whether stdout is a TTY.
    fn from_stdout() -> Self {
        Self {
            color: std::io::stdout().is_terminal(),
        }
    }

    /// Bold green — for skill names and emphasised hints.
    fn heading(&self, text: &str) -> ColoredString {
        if self.color {
            text.bold().green()
        } else {
            text.normal()
        }
    }

    /// Dim grey — for separators and file paths so they sit quietly.
    fn quiet(&self, text: &str) -> ColoredString {
        if self.color {
            text.dimmed()
        } else {
            text.normal()
        }
    }

    /// Bright yellow — the jasmine banner so the logo reads as a flower.
    fn flower(&self, text: &str) -> ColoredString {
        if self.color {
            text.bold().yellow()
        } else {
            text.normal()
        }
    }

    /// Dim yellow — the flower's outline and the banner's small print.
    fn soft(&self, text: &str) -> ColoredString {
        if self.color {
            text.yellow().dimmed()
        } else {
            text.normal()
        }
    }
}

#[derive(Parser)]
#[command(
    name = "mali_gate",
    version,
    about = "Mali Gate — skill library + MCP gateway for AI agents"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

/// คำสั่งในโหมด interactive (`mali_gate` ไม่มี subcommand)
#[derive(Debug)]
enum ReplCmd {
    Noop,
    Help,
    Banner,
    List,
    Refresh,
    Search {
        query: String,
    },
    Mcp,
    Init,
    Logs,
    /// พิมพ์ `/` อย่างเดียว หรือ prefix คลุมเครือ — แสดงเมนูให้เลือก
    CommandMenu {
        prefix: Option<String>,
    },
    Clear,
    Exit,
    Unknown(String),
    Usage(&'static str),
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the MCP server for agents (ready in task 5)
    Serve,
    /// Scan installed AI CLIs, wire `mali-gate` MCP into each, verify and report
    Init {
        /// Show what would change without writing anything
        #[arg(long)]
        dry_run: bool,
        /// Skip the live MCP handshake test
        #[arg(long)]
        no_verify: bool,
        /// Do not add ~/.mali_gate/bin to the user PATH
        #[arg(long)]
        no_path: bool,
        /// (legacy) same as plain `init`
        #[arg(long, hide = true)]
        all: bool,
    },
    /// Skill library under ~/.mali_gate/skills (index, sync, clean CLI duplicates)
    #[command(
        about = "Manage the Mali Gate skill library",
        long_about = "Index, search, install, sync, and clean skills stored in \
~/.mali_gate/skills. Agents should load skills via MCP (skills_search / skills_load), \
not by scanning AI CLI skill folders.\n\n\
Run `mali_gate skills clean --help` for removing duplicate copies from Cursor, Claude, etc."
    )]
    Skills {
        #[command(subcommand)]
        action: SkillsCmd,
    },
    /// Show recent hub events (agent tool calls)
    Logs {
        /// Maximum number of events to show (newest first)
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        decision: Option<String>,
        /// Live TUI (ratatui); use `--plain` for text stream
        #[arg(long)]
        follow: bool,
        /// Export matching events as JSONL
        #[arg(long)]
        export: Option<std::path::PathBuf>,
        #[arg(long)]
        plain: bool,
    },
    /// Summarize event counts per agent
    Stats,
    /// Manage agent tokens (MCP auth)
    Agent {
        #[command(subcommand)]
        action: AgentCmd,
    },
    /// Resolve policy \"ask\" prompts
    Approve {
        #[command(subcommand)]
        action: ApproveCmd,
    },
    /// Install skills into the library (allowlist + pin)
    Install {
        #[command(subcommand)]
        action: InstallCmd,
    },
}

#[derive(Subcommand)]
enum InstallCmd {
    /// Install from git URL or local path
    Run {
        source: String,
        #[arg(long)]
        force: bool,
    },
    Pending,
    Approve {
        id: i64,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        no: bool,
    },
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Create agent id + token (printed once)
    Add {
        id: String,
        #[arg(long)]
        label: Option<String>,
    },
    List,
    /// Switch an agent to lazy mode: keep what it already used (shared), ask for anything new
    Lazy {
        id: String,
        /// Show what would change without writing
        #[arg(long)]
        dry_run: bool,
    },
    Revoke {
        id: String,
    },
}

#[derive(Subcommand)]
enum ApproveCmd {
    List,
    /// Answer a pending request (default --shared: every agent gets it)
    Resolve {
        id: i64,
        /// Grant to every agent and remember (default)
        #[arg(long, group = "scope", alias = "yes")]
        shared: bool,
        /// Grant only to the requesting agent and remember
        #[arg(long, group = "scope")]
        agent: bool,
        /// Allow this one time only
        #[arg(long, group = "scope")]
        once: bool,
        /// Refuse this time
        #[arg(long, group = "scope")]
        no: bool,
        /// Refuse and remember (deny for this agent)
        #[arg(long, group = "scope")]
        deny: bool,
    },
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// Re-index every skill folder under the user's skills dir
    Refresh,
    /// List all skills in the index
    List,
    /// Full-text search (Thai + English)
    Search { query: String },
    /// Allow a skill for every agent (shared) or one agent
    Grant {
        name: String,
        #[arg(long)]
        agent: Option<String>,
    },
    /// Remove a skill grant from shared or one agent
    Revoke {
        name: String,
        #[arg(long)]
        agent: Option<String>,
    },
    #[command(
        about = "Remove duplicate skill folders from AI CLI directories",
        long_about = "Delete local skill copies that AI CLIs keep outside Mali Gate when the \
same skill is already in ~/.mali_gate/skills.\n\n\
Folders scanned (each child directory that contains SKILL.md):\n  \
~/.cursor/skills\n  \
~/.claude/skills  (folder named `synced` is skipped)\n  \
~/.codex/skills\n  \
~/.gemini/skills\n\n\
Matching uses the skill `name` from SKILL.md frontmatter against the library folder name.\n\n\
Default behavior: remove a CLI copy only when all files match the library copy (content hash). \
Skills that exist only on the CLI are left untouched (use `skills sync` to import them). \
Skills in the library with different content are kept on the CLI and reported as conflicts.\n\n\
After cleaning, use skills through Mali Gate MCP — not the old CLI paths.\n\n\
Examples:\n  \
mali_gate skills clean --dry-run\n  \
mali_gate skills clean\n  \
mali_gate skills clean --by-name --yes"
    )]
    Clean {
        #[arg(
            long,
            help = "Print what would be deleted without removing anything",
            long_help = "Lists CLI skill folders that would be removed. \
No files are deleted. Use this before the first real clean."
        )]
        dry_run: bool,
        #[arg(
            long,
            help = "Remove when the skill name exists in the library, even if files differ",
            long_help = "Normally a CLI copy is removed only when every file matches \
~/.mali_gate/skills/<name>. With --by-name, any CLI copy whose SKILL.md name is already \
in the library is removed, even if you edited one side. Requires --yes (unless --dry-run)."
        )]
        by_name: bool,
        #[arg(
            long,
            help = "Confirm destructive actions (--by-name)",
            long_help = "Required together with --by-name when not using --dry-run, \
because removing a newer CLI copy cannot be undone."
        )]
        yes: bool,
    },
    /// Import skills from ~/.cursor/skills, ~/.claude/skills, etc. into the library
    Sync {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        force: bool,
        /// Remove source folders when they match the library copy (dedupe on disk)
        #[arg(long)]
        clean_sources: bool,
    },
    /// Remove a skill folder from ~/.mali_gate/skills
    Remove {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Show one indexed skill (path + description)
    Show { name: String },
}

/// Width of the terminal in columns; 100 when unknown (pipes, tests).
fn terminal_width() -> usize {
    terminal::size().map(|(w, _)| w as usize).unwrap_or(100)
}

/// A section heading such as `skills (5)`.
fn print_section(title: &str, count: usize, paint: &Palette) {
    println!("  {}", paint.heading(&format!("{title} ({count})")));
    println!("  {}", paint.quiet(RULE));
}

/// Show one skill in a three-line block: name / description / path.
fn print_hit(hit: &SearchHit, paint: &Palette) {
    println!("  {}", paint.heading(&hit.name));
    for line in hit.description.lines() {
        println!("      {line}");
    }
    println!(
        "      {}",
        paint.quiet(&format!("path: {}", hit.dir.display()))
    );
    println!();
}

/// Show one skill as a numbered result: `1. name`.
struct HubSession {
    index: SkillIndex,
    paint: Palette,
}

fn open_session() -> anyhow::Result<HubSession> {
    let paint = Palette::from_stdout();
    let index = SkillIndex::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(HubSession { index, paint })
}

fn skills_refresh(session: &HubSession) -> anyhow::Result<()> {
    let root = skills_dir();
    let n = session
        .index
        .index_dir(&root)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    println!(
        "  {}",
        session.paint.heading(&format!("indexed {n} skills"))
    );
    println!(
        "  {}",
        session
            .paint
            .quiet(&format!("folder:   {}", root.display()))
    );
    Ok(())
}

fn skills_list(session: &HubSession) -> anyhow::Result<()> {
    let all = session
        .index
        .list_all()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if all.is_empty() {
        println!(
            "  {} — run {} or {} first.",
            session.paint.heading("index is empty"),
            session.paint.heading("/refresh"),
            session.paint.heading("mali_gate skills refresh")
        );
    } else {
        print_section("skills", all.len(), &session.paint);
        for hit in &all {
            print_hit(hit, &session.paint);
        }
    }
    Ok(())
}

fn skills_search(session: &HubSession, query: &str) -> anyhow::Result<()> {
    let hits = session
        .index
        .search(query)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if hits.is_empty() {
        println!(
            "  no skills matched {}",
            session.paint.heading(&format!("\"{query}\""))
        );
    } else {
        print_section("matches", hits.len(), &session.paint);
        for (i, hit) in hits.iter().enumerate() {
            print_ranked_hit(i + 1, hit, &session.paint);
        }
    }
    Ok(())
}

fn print_mcp_status(paint: &Palette) {
    println!("  {}", paint.heading("MCP: mali_gate serve (stdio, rmcp)"));
    println!(
        "  {}",
        paint.soft("Tools: skills_search, skills_load, skills_read_file")
    );
    println!(
        "  {}",
        paint.quiet(
            "In another terminal: mali_gate serve — or enable mali-gate in .cursor/mcp.json"
        )
    );
}

fn clear_terminal() -> anyhow::Result<()> {
    if io::stdout().is_terminal() {
        execute!(io::stdout(), Clear(ClearType::All), cursor::MoveTo(0, 0))?;
    }
    Ok(())
}

fn run_repl() -> anyhow::Result<()> {
    let session = open_session()?;
    print_banner(&session.paint);
    loop {
        let Some(line) = repl_input::read_line(&session.paint)? else {
            println!();
            break;
        };

        let mut cmd = slash::parse_repl_line(&line);
        if let ReplCmd::CommandMenu { ref prefix } = cmd {
            slash::print_command_menu(&session.paint, prefix.as_deref());
            match slash::read_menu_pick(&session.paint, prefix.as_deref())? {
                Some(next) => cmd = slash::parse_repl_line(&next),
                None => continue,
            }
        }

        match cmd {
            ReplCmd::Noop => {}
            ReplCmd::CommandMenu { .. } => {}
            ReplCmd::Help => {
                slash::print_repl_help(&session.paint);
            }
            ReplCmd::Banner => print_banner(&session.paint),
            ReplCmd::List => {
                if let Err(err) = skills_list(&session) {
                    eprintln!("error: {err}");
                }
            }
            ReplCmd::Refresh => {
                if let Err(err) = skills_refresh(&session) {
                    eprintln!("error: {err}");
                }
            }
            ReplCmd::Search { query } => {
                if let Err(err) = skills_search(&session, &query) {
                    eprintln!("error: {err}");
                }
            }
            ReplCmd::Mcp => print_mcp_status(&session.paint),
            ReplCmd::Init => {
                if let Err(err) = init::run_init(&init::InitOptions::default(), &session.paint) {
                    eprintln!("error: {err}");
                }
            }
            ReplCmd::Logs => {
                if let Err(err) = logs::run_logs(
                    logs::LogsOptions {
                        limit: 50,
                        agent: None,
                        kind: None,
                        decision: None,
                        follow: false,
                        export: None,
                        plain: false,
                    },
                    &session.paint,
                ) {
                    eprintln!("error: {err}");
                }
            }
            ReplCmd::Clear => clear_terminal()?,
            ReplCmd::Exit => break,
            ReplCmd::Unknown(name) => {
                eprintln!(
                    "error: unknown command /{} — try {}",
                    name,
                    session.paint.heading("/")
                );
                slash::print_command_menu(&session.paint, Some(name.as_str()));
            }
            ReplCmd::Usage(msg) => {
                eprintln!("error: {msg}");
            }
        }
    }
    Ok(())
}

fn print_ranked_hit(num: usize, hit: &SearchHit, paint: &Palette) {
    println!(
        "  {}. {}",
        paint.quiet(&num.to_string()),
        paint.heading(&hit.name)
    );
    for line in hit.description.lines() {
        println!("      {line}");
    }
    println!(
        "      {}",
        paint.quiet(&format!("path: {}", hit.dir.display()))
    );
    println!();
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let cmd = cli.cmd.ok_or_else(|| {
        anyhow::anyhow!("missing subcommand — run `mali_gate` for interactive mode")
    })?;
    match cmd {
        Cmd::Serve => {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .block_on(hub_mcp::serve_stdio())?;
        }
        Cmd::Init {
            dry_run,
            no_verify,
            no_path,
            all: _,
        } => {
            let paint = Palette::from_stdout();
            init::run_init(
                &init::InitOptions {
                    dry_run,
                    skip_verify: no_verify,
                    no_path,
                },
                &paint,
            )?;
        }
        Cmd::Skills {
            action: SkillsCmd::Grant { name, agent },
        } => approve::run_grant(&name, agent.as_deref(), true, &Palette::from_stdout())?,
        Cmd::Skills {
            action: SkillsCmd::Revoke { name, agent },
        } => approve::run_grant(&name, agent.as_deref(), false, &Palette::from_stdout())?,
        Cmd::Skills {
            action:
                SkillsCmd::Clean {
                    dry_run,
                    by_name,
                    yes,
                },
        } => skills_cmd::run_skills_clean(dry_run, by_name, yes, &Palette::from_stdout())?,
        Cmd::Skills {
            action:
                SkillsCmd::Sync {
                    dry_run,
                    force,
                    clean_sources,
                },
        } => skills_cmd::run_skills_sync(dry_run, force, clean_sources, &Palette::from_stdout())?,
        Cmd::Skills {
            action: SkillsCmd::Remove { name, yes },
        } => skills_cmd::run_skills_remove(&name, yes, &Palette::from_stdout())?,
        Cmd::Skills {
            action: SkillsCmd::Show { name },
        } => {
            let paint = Palette::from_stdout();
            print_banner(&paint);
            skills_cmd::run_skills_show(&name, &paint)?;
        }
        Cmd::Skills { action } => {
            let session = open_session()?;
            print_banner(&session.paint);
            match action {
                SkillsCmd::Refresh => skills_refresh(&session)?,
                SkillsCmd::List => skills_list(&session)?,
                SkillsCmd::Search { query } => skills_search(&session, &query)?,
                SkillsCmd::Grant { .. }
                | SkillsCmd::Revoke { .. }
                | SkillsCmd::Clean { .. }
                | SkillsCmd::Sync { .. }
                | SkillsCmd::Remove { .. }
                | SkillsCmd::Show { .. } => {}
            }
        }
        Cmd::Logs {
            limit,
            agent,
            kind,
            decision,
            follow,
            export,
            plain,
        } => {
            let paint = Palette::from_stdout();
            let kind = parse_optional_kind(kind.as_deref())?;
            let decision = parse_optional_decision(decision.as_deref())?;
            logs::run_logs(
                logs::LogsOptions {
                    limit,
                    agent,
                    kind,
                    decision,
                    follow,
                    export,
                    plain,
                },
                &paint,
            )?;
        }
        Cmd::Stats => {
            let paint = Palette::from_stdout();
            stats::run_stats(&paint)?;
        }
        Cmd::Agent { action } => {
            let paint = Palette::from_stdout();
            match action {
                AgentCmd::Add { id, label } => {
                    agent::run_agent_add(&id, label.as_deref(), &paint)?;
                }
                AgentCmd::List => agent::run_agent_list(&paint)?,
                AgentCmd::Lazy { id, dry_run } => agent::run_agent_lazy(&id, dry_run, &paint)?,
                AgentCmd::Revoke { id } => agent::run_agent_revoke(&id, &paint)?,
            }
        }
        Cmd::Approve { action } => {
            let paint = Palette::from_stdout();
            match action {
                ApproveCmd::List => approve::run_approve_list(&paint)?,
                ApproveCmd::Resolve {
                    id,
                    shared: _,
                    agent,
                    once,
                    no,
                    deny,
                } => {
                    let scope = match (agent, once, no, deny) {
                        (true, ..) => GrantScope::Agent,
                        (_, true, ..) => GrantScope::Once,
                        (_, _, true, _) => GrantScope::Deny,
                        (.., true) => GrantScope::Deny,
                        _ => GrantScope::Shared,
                    };
                    // --no = ปฏิเสธครั้งนี้ (ไม่จำ); --deny = ปฏิเสธและจำ
                    approve::run_approve(id, scope, !no, &paint)?;
                }
            }
        }
        Cmd::Install { action } => {
            let paint = Palette::from_stdout();
            match action {
                InstallCmd::Run { source, force } => {
                    install_cmd::run_install(&source, force, &paint)?;
                }
                InstallCmd::Pending => install_cmd::run_install_pending(&paint)?,
                InstallCmd::Approve { id, yes, no } => {
                    if yes == no {
                        anyhow::bail!("use --yes or --no");
                    }
                    install_cmd::run_install_approve(id, yes, &paint)?;
                }
            }
        }
    }
    Ok(())
}

fn parse_optional_kind(text: Option<&str>) -> anyhow::Result<Option<EventKind>> {
    match text {
        None => Ok(None),
        Some(s) => EventKind::parse_str(s)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("unknown event kind: {s}")),
    }
}

fn parse_optional_decision(text: Option<&str>) -> anyhow::Result<Option<Decision>> {
    match text {
        None => Ok(None),
        Some(s) => Decision::parse_str(s)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("unknown decision: {s}")),
    }
}

fn main() {
    let cli = Cli::parse();
    let result = if cli.cmd.is_none() {
        if io::stdin().is_terminal() && io::stdout().is_terminal() {
            run_repl()
        } else {
            Cli::command()
                .print_help()
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
    } else {
        run(cli)
    };
    if let Err(err) = result {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// banner ฝังข้อความเวอร์ชันจาก crate — ต้องไม่ว่าง
    #[test]
    fn version_string_is_available() {
        assert!(!env!("CARGO_PKG_VERSION").is_empty());
    }

    /// เมื่อ stdout ไม่ใช่ terminal (เช่น piped) ต้องไม่มี ANSI escape codes
    #[test]
    fn palette_stays_plain_when_not_a_tty() {
        let paint = Palette { color: false };
        assert_eq!(paint.heading("abc").to_string(), "abc");
        assert_eq!(paint.quiet("X").to_string(), "X");
        assert_eq!(paint.flower("X").to_string(), "X");
        assert_eq!(paint.soft("X").to_string(), "X");
    }

    /// เมื่อเปิดสี ต้องมี ESC (0x1b) ครอบอยู่จริง ทั้งเขียวและเหลือง
    #[test]
    fn palette_wraps_in_ansi_when_colored() {
        colored::control::set_override(true);
        let paint = Palette { color: true };
        assert!(paint.heading("abc").to_string().contains('\x1b'));
        assert!(paint.quiet("abc").to_string().contains('\x1b'));
        assert!(paint.flower("abc").to_string().contains('\x1b'));
        assert!(paint.soft("abc").to_string().contains('\x1b'));
        colored::control::unset_override();
    }

    /// ความกว้าง terminal ที่ได้ต้องอยู่ในช่วงที่สมเหตุสมผล
    #[test]
    fn terminal_width_returns_plausible_value() {
        let width = terminal_width();
        assert!((40..=1000).contains(&width), "กว้างจอผิดปกติ: {width}");
    }

    /// คำสั่ง slash ใน REPL ต้อง parse เป็น ReplCmd ที่ถูกต้อง
    #[test]
    fn repl_parses_slash_commands() {
        use slash::parse_repl_line;
        assert!(matches!(parse_repl_line(""), ReplCmd::Noop));
        assert!(matches!(parse_repl_line("/"), ReplCmd::CommandMenu { .. }));
        assert!(matches!(parse_repl_line("/help"), ReplCmd::Help));
        assert!(matches!(parse_repl_line("/list"), ReplCmd::List));
        assert!(matches!(parse_repl_line("/init"), ReplCmd::Init));
        match parse_repl_line("/search weekly report") {
            ReplCmd::Search { query } => assert_eq!(query, "weekly report".to_string()),
            other => panic!("expected Search, got {other:?}"),
        }
    }
}
