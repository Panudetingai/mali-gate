use std::io::stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use hub_log::{AgentUsageStats, Decision, EventKind, EventRow, LogQuery, Logger};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Cell, HighlightSpacing, Padding, Paragraph, Row, Sparkline, Table, TableState,
    Wrap,
};

const POLL_MS: u64 = 400;

const AGENT_COLORS: [Color; 6] = [
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::LightRed,
];

const KINDS: [EventKind; 9] = [
    EventKind::SkillSearch,
    EventKind::SkillLoad,
    EventKind::SkillFile,
    EventKind::Install,
    EventKind::Connect,
    EventKind::Call,
    EventKind::Release,
    EventKind::PolicyDenied,
    EventKind::Approval,
];

const DECISIONS: [Decision; 4] = [
    Decision::Allow,
    Decision::Deny,
    Decision::AskApproved,
    Decision::AskDenied,
];

/// ตัวเลือกเปิด TUI log viewer
pub struct LogViewerConfig {
    pub db_path: PathBuf,
    pub query: LogQuery,
    pub follow: bool,
    pub empty_hint: Option<String>,
}

struct App {
    db_path: PathBuf,
    empty_hint: Option<String>,
    rows: Vec<EventRow>,
    selected: usize,
    query: LogQuery,
    follow: bool,
    agents: Vec<String>,
    usage: Vec<AgentUsageStats>,
    activity: Vec<u64>,
    agent_idx: usize,
    kind_idx: usize,
    decision_idx: usize,
    status: String,
    table_state: TableState,
}

impl App {
    fn new(cfg: LogViewerConfig) -> anyhow::Result<Self> {
        let logger = Logger::open(&cfg.db_path).context("open log db")?;
        let usage = logger.agent_usage_stats().context("usage stats")?;
        let activity = logger.activity_buckets(32).context("activity")?;
        let agents: Vec<String> = usage.iter().map(|s| s.agent.clone()).collect();
        let rows = logger.query(&cfg.query).context("load events")?;
        let mut table_state = TableState::default();
        table_state.select(if rows.is_empty() { None } else { Some(0) });
        let follow = cfg.follow;
        Ok(Self {
            db_path: cfg.db_path.clone(),
            empty_hint: cfg.empty_hint,
            rows,
            selected: 0,
            query: cfg.query,
            follow,
            agents,
            usage,
            activity,
            agent_idx: 0,
            kind_idx: 0,
            decision_idx: 0,
            status: String::new(),
            table_state,
        })
    }

    fn refresh_dashboard(&mut self) -> anyhow::Result<()> {
        let logger = Logger::open(&self.db_path).context("open log db")?;
        self.usage = logger.agent_usage_stats().context("usage stats")?;
        self.activity = logger.activity_buckets(32).context("activity")?;
        if self.agent_idx == 0 {
            self.agents = self.usage.iter().map(|s| s.agent.clone()).collect();
        }
        Ok(())
    }

    fn sync_filter_query(&mut self) {
        self.query.agent = if self.agent_idx == 0 {
            None
        } else {
            Some(self.agents[self.agent_idx - 1].clone())
        };
        self.query.kind = if self.kind_idx == 0 {
            None
        } else {
            Some(KINDS[self.kind_idx - 1].clone())
        };
        self.query.decision = if self.decision_idx == 0 {
            None
        } else {
            Some(DECISIONS[self.decision_idx - 1].clone())
        };
    }

    /// ล้างข้อความแจ้งเตือนชั่วคราว (footer กลับไปแสดงปุ่มลัด)
    fn refresh_status(&mut self) {
        self.status.clear();
    }

    fn reload(&mut self) -> anyhow::Result<()> {
        let logger = Logger::open(&self.db_path).context("open log db")?;
        self.rows = logger.query(&self.query).context("query events")?;
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
        self.table_state.select(if self.rows.is_empty() {
            None
        } else {
            Some(self.selected)
        });
        self.refresh_dashboard()?;
        Ok(())
    }

    fn apply_filters(&mut self) -> anyhow::Result<()> {
        self.sync_filter_query();
        self.reload()?;
        self.refresh_status();
        Ok(())
    }

    fn reset_filters(&mut self) -> anyhow::Result<()> {
        self.agent_idx = 0;
        self.kind_idx = 0;
        self.decision_idx = 0;
        self.apply_filters()
    }

    fn cycle_agent(&mut self) -> anyhow::Result<()> {
        let max = self.agents.len();
        self.agent_idx = if max == 0 {
            0
        } else {
            (self.agent_idx + 1) % (max + 1)
        };
        self.apply_filters()
    }

    fn cycle_kind(&mut self) -> anyhow::Result<()> {
        self.kind_idx = (self.kind_idx + 1) % (KINDS.len() + 1);
        self.apply_filters()
    }

    fn cycle_decision(&mut self) -> anyhow::Result<()> {
        self.decision_idx = (self.decision_idx + 1) % (DECISIONS.len() + 1);
        self.apply_filters()
    }

    fn poll_new(&mut self) -> anyhow::Result<()> {
        if self.follow && self.rows.is_empty() {
            self.reload()?;
            return Ok(());
        }
        if !self.follow {
            return Ok(());
        }
        let logger = Logger::open(&self.db_path).context("open log db")?;
        let max_id = self.rows.first().map(|r| r.id).unwrap_or(0);
        let mut q = self.query.clone();
        q.after_id = Some(max_id);
        q.limit = 200;
        let fresh = logger.query(&q).context("poll events")?;
        if !fresh.is_empty() {
            for row in fresh.into_iter().rev() {
                if self.rows.iter().any(|e| e.id == row.id) {
                    continue;
                }
                self.rows.insert(0, row);
            }
            while self.rows.len() > self.query.limit {
                self.rows.pop();
            }
        }
        self.refresh_dashboard()?;
        Ok(())
    }

    fn move_sel(&mut self, delta: i32) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as i32;
        let next = (self.selected as i32 + delta).clamp(0, n - 1) as usize;
        self.selected = next;
        self.table_state.select(Some(next));
    }

    fn total_est_tokens(&self) -> i64 {
        self.usage.iter().map(|u| u.est_tokens).sum()
    }
}

// ───────────────────────────── palette ─────────────────────────────

const ACCENT: Color = Color::Rgb(180, 140, 255);
const BORDER: Color = Color::Rgb(60, 62, 82);
const DIM: Color = Color::Rgb(110, 112, 132);
const TEXT: Color = Color::Rgb(215, 215, 225);
const GOOD: Color = Color::Rgb(120, 220, 140);
const BAD: Color = Color::Rgb(240, 100, 100);
const WARN: Color = Color::Rgb(240, 200, 90);
const INFO: Color = Color::Rgb(100, 200, 255);
const TOKENS: Color = Color::Rgb(240, 200, 120);
const SELECTED_BG: Color = Color::Rgb(45, 50, 72);

/// จอกว้างพอ → detail อยู่ขวา; แคบ → detail อยู่ใต้ตาราง
const WIDE_MIN: u16 = 110;
const DETAIL_WIDTH: u16 = 46;
const AGENT_COL_MIN: usize = 5;
const AGENT_COL_MAX: usize = 20;

fn help_line(follow: bool) -> String {
    let tail = if follow { "● live" } else { "○ paused" };
    format!(" q quit · j/k move · a/t/d filter · r reset · f follow · {tail}")
}

fn agent_color(name: &str) -> Color {
    let h = name.bytes().fold(0u64, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u64::from(b))
    });
    AGENT_COLORS[(h as usize) % AGENT_COLORS.len()]
}

fn panel_block(title: String, accent: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .padding(Padding::horizontal(1))
        .title(Line::from(Span::styled(
            format!(" {title} "),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )))
}

fn decision_style(decision: Option<&Decision>) -> Style {
    match decision {
        Some(Decision::Allow) | Some(Decision::AskApproved) => Style::default().fg(GOOD),
        Some(Decision::Deny) | Some(Decision::AskDenied) => Style::default().fg(BAD),
        None => Style::default().fg(DIM),
    }
}

/// สัญลักษณ์ + คำสั้นของ decision สำหรับตาราง
fn decision_label(decision: Option<&Decision>) -> &'static str {
    match decision {
        Some(Decision::Allow) => "✓ allow",
        Some(Decision::Deny) => "✗ deny",
        Some(Decision::AskApproved) => "✓ ask",
        Some(Decision::AskDenied) => "✗ ask",
        None => "·",
    }
}

fn kind_style(kind: &EventKind) -> Style {
    match kind {
        EventKind::PolicyDenied => Style::default().fg(BAD),
        EventKind::Approval | EventKind::Install => Style::default().fg(WARN),
        EventKind::Call | EventKind::Connect | EventKind::Release => Style::default().fg(INFO),
        _ => Style::default().fg(TEXT),
    }
}

/// ชื่อ kind แบบสั้นในตาราง (ค่าเต็มอยู่ใน detail)
fn short_kind(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::SkillSearch => "search",
        EventKind::SkillLoad => "load",
        EventKind::SkillFile => "file",
        EventKind::Install => "install",
        EventKind::Connect => "connect",
        EventKind::Call => "call",
        EventKind::Release => "release",
        EventKind::PolicyDenied => "denied",
        EventKind::Approval => "approval",
    }
}

/// target ในตาราง — `call` รวม server กับ tool เป็นคอลัมน์เดียว
fn target_label(row: &EventRow) -> String {
    if row.kind == EventKind::Call && !row.tool.is_empty() {
        format!("{} › {}", row.target, row.tool)
    } else if row.target.is_empty() {
        "·".into()
    } else {
        row.target.clone()
    }
}

/// ความกว้างคอลัมน์ agent = ชื่อยาวสุด (จำกัด min/max)
fn agent_col_width<'a>(names: impl Iterator<Item = &'a str>) -> usize {
    names
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(AGENT_COL_MIN, AGENT_COL_MAX)
}

fn event_arg_tokens(args: &serde_json::Value) -> u64 {
    for key in [
        "total_tokens",
        "tokens_used",
        "input_tokens",
        "output_tokens",
    ] {
        if let Some(n) = args.get(key).and_then(|v| v.as_u64()) {
            return n;
        }
    }
    (args.to_string().len() as u64).max(1) / 4
}

/// `2026-11-02T06:48:30.6Z` → (`11-02`, `06:48:30`)
fn split_ts(ts: &str) -> (String, String) {
    let date = ts.get(5..10).unwrap_or("").to_string();
    let time = ts.get(11..19).unwrap_or(ts).to_string();
    (date, time)
}

/// `2026-11-02 06:48:30`
fn full_ts(ts: &str) -> String {
    ts.get(..19)
        .map(|s| s.replace('T', " "))
        .unwrap_or_else(|| ts.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!(
        "{}…",
        s.chars().take(max.saturating_sub(1)).collect::<String>()
    )
}

/// ตัดด้านซ้ายให้เหลือ `max` ตัวอักษร (เก็บท้าย path ที่สำคัญไว้)
fn truncate_left(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let tail: String = s.chars().skip(n - max.saturating_sub(1)).collect();
    format!("…{tail}")
}

fn format_num(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

// ───────────────────────────── header ─────────────────────────────

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel_block("Mali Gate".into(), ACCENT);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let spark_w = if inner.width >= 90 { 24 } else { 0 };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(spark_w)])
        .split(inner);

    let total: i64 = app.usage.iter().map(|u| u.events).sum();
    let mut spans = vec![
        if app.follow {
            Span::styled(
                "● LIVE",
                Style::default().fg(GOOD).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("○ PAUSED", Style::default().fg(DIM))
        },
        Span::styled(format!("  {total} events"), Style::default().fg(TEXT)),
        Span::styled("  │ ", Style::default().fg(BORDER)),
    ];
    if app.usage.is_empty() {
        spans.push(Span::styled("no agents yet", Style::default().fg(DIM)));
    }
    for (i, u) in app.usage.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(DIM)));
        }
        spans.push(Span::styled(
            u.agent.clone(),
            Style::default()
                .fg(agent_color(&u.agent))
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {}", u.events),
            Style::default().fg(TEXT),
        ));
    }
    let filters = active_filters(app);
    if !filters.is_empty() {
        spans.push(Span::styled("  │ ", Style::default().fg(BORDER)));
        spans.push(Span::styled(
            format!("filter {filters}"),
            Style::default().fg(WARN),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), cols[0]);

    if spark_w > 0 {
        let spark = Sparkline::default()
            .style(Style::default().fg(Color::Rgb(120, 220, 180)))
            .data(app.activity.iter().copied());
        frame.render_widget(spark, cols[1]);
    }
}

fn active_filters(app: &App) -> String {
    let mut parts = Vec::new();
    if let Some(a) = &app.query.agent {
        parts.push(format!("agent={a}"));
    }
    if let Some(k) = &app.query.kind {
        parts.push(format!("kind={}", short_kind(k)));
    }
    if let Some(d) = &app.query.decision {
        parts.push(format!("decision={}", d.as_str()));
    }
    parts.join(" ")
}

// ───────────────────────────── events table ─────────────────────────────

fn render_event_table(frame: &mut Frame, area: Rect, app: &mut App) {
    let agent_w = agent_col_width(app.rows.iter().map(|r| r.agent.as_str()));
    let id_w = app
        .rows
        .iter()
        .map(|r| r.id.to_string().len())
        .max()
        .unwrap_or(1)
        .max(1);

    let mut prev_date = String::new();
    let rows: Vec<Row> = app
        .rows
        .iter()
        .map(|r| {
            let (date, time) = split_ts(&r.ts);
            // แสดงวันที่เฉพาะแถวแรกของแต่ละวัน
            let date_cell = if date != prev_date {
                prev_date = date.clone();
                date
            } else {
                String::new()
            };
            let ms = r.duration_ms.map(|n| n.to_string()).unwrap_or_default();
            let ms_style = match r.duration_ms {
                Some(n) if n >= 1000 => Style::default().fg(WARN),
                _ => Style::default().fg(DIM),
            };
            let status_err = r.result_status.as_deref() == Some("error") || r.error.is_some();
            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{:>id_w$}", r.id),
                    Style::default().fg(DIM),
                )),
                Cell::from(Span::styled(date_cell, Style::default().fg(DIM))),
                Cell::from(Span::styled(
                    time,
                    Style::default().fg(Color::Rgb(150, 150, 170)),
                )),
                Cell::from(Span::styled(
                    truncate(&r.agent, AGENT_COL_MAX),
                    Style::default()
                        .fg(agent_color(&r.agent))
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(Span::styled(short_kind(&r.kind), kind_style(&r.kind))),
                Cell::from(Span::styled(
                    target_label(r),
                    if status_err {
                        Style::default().fg(BAD)
                    } else {
                        Style::default().fg(TEXT)
                    },
                )),
                Cell::from(Span::styled(
                    decision_label(r.decision.as_ref()),
                    decision_style(r.decision.as_ref()),
                )),
                Cell::from(Line::from(Span::styled(ms, ms_style)).alignment(Alignment::Right)),
            ])
        })
        .collect();

    let header_style = Style::default().fg(DIM).add_modifier(Modifier::BOLD);
    let header = Row::new(
        [
            "#", "date", "time", "agent", "kind", "target", "decision", "ms",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, h)| {
            let line = Line::from(Span::styled(h, header_style));
            Cell::from(if i == 0 || i == 7 {
                line.alignment(Alignment::Right)
            } else {
                line
            })
        }),
    )
    .bottom_margin(0);

    let widths = [
        Constraint::Length(id_w as u16),
        Constraint::Length(5),
        Constraint::Length(8),
        Constraint::Length(agent_w as u16),
        Constraint::Length(8),
        Constraint::Min(10),
        Constraint::Length(8),
        Constraint::Length(6),
    ];
    let title = if app.rows.is_empty() {
        "Events".to_string()
    } else {
        format!("Events {}/{}", app.selected + 1, app.rows.len())
    };
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(2)
        .block(panel_block(title, INFO))
        .row_highlight_style(
            Style::default()
                .bg(SELECTED_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .highlight_spacing(HighlightSpacing::Always);
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

// ───────────────────────────── detail ─────────────────────────────

fn kv(key: &str, value: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{key:<8} "), Style::default().fg(DIM))];
    spans.extend(value);
    Line::from(spans)
}

fn detail_lines(
    row: Option<&EventRow>,
    empty_hint: Option<&str>,
    total_tokens: i64,
) -> Vec<Line<'static>> {
    let Some(r) = row else {
        return match empty_hint {
            Some(hint) => hint.lines().map(|l| Line::from(l.to_string())).collect(),
            None => vec![Line::from(Span::styled(
                "Select an event or wait for traffic",
                Style::default().fg(DIM),
            ))],
        };
    };
    let tok = event_arg_tokens(&r.args);
    let mut out = vec![
        Line::from(vec![
            Span::styled(
                format!("#{}", r.id),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {}", full_ts(&r.ts)), Style::default().fg(DIM)),
        ]),
        Line::from(""),
        kv(
            "agent",
            vec![Span::styled(
                r.agent.clone(),
                Style::default()
                    .fg(agent_color(&r.agent))
                    .add_modifier(Modifier::BOLD),
            )],
        ),
        kv(
            "session",
            vec![Span::styled(
                r.session_id.clone(),
                Style::default().fg(TEXT),
            )],
        ),
        kv(
            "kind",
            vec![Span::styled(r.kind.as_str(), kind_style(&r.kind))],
        ),
        kv(
            "target",
            vec![Span::styled(
                if r.target.is_empty() {
                    "·".into()
                } else {
                    r.target.clone()
                },
                Style::default().fg(TEXT),
            )],
        ),
    ];
    if !r.tool.is_empty() && r.tool != r.kind.as_str() {
        out.push(kv(
            "tool",
            vec![Span::styled(r.tool.clone(), Style::default().fg(TEXT))],
        ));
    }
    let status = r.result_status.clone().unwrap_or_else(|| "—".into());
    let status_style = match status.as_str() {
        "ok" => Style::default().fg(GOOD),
        "error" => Style::default().fg(BAD),
        _ => Style::default().fg(DIM),
    };
    out.push(kv(
        "result",
        vec![
            Span::styled(
                r.decision.as_ref().map(|d| d.as_str()).unwrap_or("—"),
                decision_style(r.decision.as_ref()),
            ),
            Span::styled(" · ", Style::default().fg(DIM)),
            Span::styled(status, status_style),
            Span::styled(
                r.duration_ms
                    .map(|n| format!(" · {n} ms"))
                    .unwrap_or_default(),
                Style::default().fg(DIM),
            ),
        ],
    ));
    out.push(kv(
        "tokens",
        vec![
            Span::styled(
                format!("~{}", format_num(tok as i64)),
                Style::default().fg(TOKENS),
            ),
            Span::styled(
                format!("  (all agents ~{})", format_num(total_tokens)),
                Style::default().fg(DIM),
            ),
        ],
    ));
    if let Some(err) = &r.error {
        out.push(kv(
            "error",
            vec![Span::styled(err.clone(), Style::default().fg(BAD))],
        ));
    }
    out.push(Line::from(""));
    out.push(Line::from(Span::styled(
        "── arguments ──",
        Style::default().fg(INFO).add_modifier(Modifier::BOLD),
    )));
    let args = serde_json::to_string_pretty(&r.args).unwrap_or_else(|_| r.args.to_string());
    let total = args.lines().count();
    for line in args.lines().take(20) {
        out.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::Rgb(200, 200, 210)),
        )));
    }
    if total > 20 {
        out.push(Line::from(Span::styled(
            format!("… {} more lines", total - 20),
            Style::default().fg(DIM),
        )));
    }
    out
}

// ───────────────────────────── footer + frame ─────────────────────────────

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let left = if app.status.is_empty() {
        Span::styled(help_line(app.follow), Style::default().fg(DIM))
    } else {
        Span::styled(format!(" {}", app.status), Style::default().fg(WARN))
    };
    let left_w = left.width() as u16;
    let room = area.width.saturating_sub(left_w + 2) as usize;
    let db = truncate_left(&app.db_path.display().to_string(), room);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(left_w),
            Constraint::Length(db.chars().count() as u16 + 1),
        ])
        .split(area);
    frame.render_widget(Paragraph::new(Line::from(left)), cols[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(db, Style::default().fg(BORDER))))
            .alignment(Alignment::Right),
        cols[1],
    );
}

fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, root[0], app);

    let wide = area.width >= WIDE_MIN;
    let body = if wide {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(DETAIL_WIDTH)])
            .split(root[1])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(12)])
            .split(root[1])
    };
    render_event_table(frame, body[0], app);

    let hint = if app.rows.is_empty() {
        app.empty_hint.as_deref()
    } else {
        None
    };
    let row = app.rows.get(app.selected);
    let title = row
        .map(|r| format!("Detail #{}", r.id))
        .unwrap_or_else(|| "Detail".into());
    let detail = Paragraph::new(detail_lines(row, hint, app.total_est_tokens()))
        .block(panel_block(title, ACCENT))
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, body[1]);

    render_footer(frame, root[2], app);
}

fn handle_key(app: &mut App, code: KeyCode, _modifiers: KeyModifiers) -> anyhow::Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Char('j') | KeyCode::Down => app.move_sel(1),
        KeyCode::Char('k') | KeyCode::Up => app.move_sel(-1),
        KeyCode::Char('a') => app.cycle_agent()?,
        KeyCode::Char('t') => app.cycle_kind()?,
        KeyCode::Char('d') => app.cycle_decision()?,
        KeyCode::Char('r') => app.reset_filters()?,
        KeyCode::Char('f') | KeyCode::Char('F') => {
            app.follow = !app.follow;
            app.refresh_status();
        }
        _ => {}
    }
    Ok(false)
}

/// คืนสภาพ terminal เสมอ (ทั้งตอน return ปกติ, error `?` และ panic)
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode().context("raw mode")?;
        if let Err(err) = stdout().execute(EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(err).context("alt screen");
        }
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
    }
}

/// เปิด TUI — ต้องรันบน terminal จริง
pub fn run_log_viewer(cfg: LogViewerConfig) -> anyhow::Result<()> {
    // โหลดข้อมูลก่อนเข้า raw mode: ถ้า DB เปิดไม่ได้ error จะแสดงบน terminal ปกติ
    let mut app = App::new(cfg)?;
    app.refresh_status();

    let _guard = TerminalGuard::enter()?;
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
        prev_hook(info);
    }));
    let result = event_loop(&mut app);
    let _ = std::panic::take_hook();
    result
}

fn event_loop(app: &mut App) -> anyhow::Result<()> {
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).context("terminal")?;

    let mut last_poll = Instant::now();
    let tick = Duration::from_millis(POLL_MS);
    loop {
        terminal.draw(|f| draw(f, app)).context("draw")?;

        let poll_wait = if app.follow {
            tick.saturating_sub(last_poll.elapsed())
        } else {
            Duration::from_millis(250)
        };

        if event::poll(poll_wait).context("poll input")? {
            // event อื่น (เช่น resize) — วาดใหม่รอบถัดไปเอง
            if let Event::Key(key) = event::read().context("read key")? {
                if key.kind == KeyEventKind::Press && handle_key(app, key.code, key.modifiers)? {
                    return Ok(());
                }
            }
        }

        if app.follow && last_poll.elapsed() >= tick {
            // DB อาจถูก `serve` ล็อกชั่วคราว — แสดงใน status แทนการปิด TUI ทิ้ง
            if let Err(err) = app.poll_new() {
                app.status = format!("poll error (retrying): {err:#}");
            } else if app.status.starts_with("poll error") {
                app.refresh_status();
            }
            last_poll = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn row(id: i64, ts: &str, agent: &str, kind: EventKind, target: &str, tool: &str) -> EventRow {
        EventRow {
            id,
            ts: ts.into(),
            agent: agent.into(),
            session_id: "pid-1".into(),
            kind,
            target: target.into(),
            tool: tool.into(),
            args: serde_json::json!({"query": "skill"}),
            decision: Some(Decision::Allow),
            result_status: Some("ok".into()),
            duration_ms: Some(3),
            error: None,
        }
    }

    fn app() -> App {
        let rows = vec![
            row(
                3,
                "2026-11-02T07:40:32.3Z",
                "claude-code",
                EventKind::Call,
                "github",
                "search_issues",
            ),
            row(
                2,
                "2026-11-02T07:40:32.1Z",
                "cursor",
                EventKind::SkillLoad,
                "pdf",
                "skills_load",
            ),
            row(
                1,
                "2026-11-01T23:59:59.0Z",
                "codex",
                EventKind::SkillSearch,
                "skill",
                "skills_search",
            ),
        ];
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        App {
            db_path: PathBuf::from("/home/someone/.mali_gate/mali_gate.db"),
            empty_hint: None,
            rows,
            selected: 0,
            query: LogQuery {
                limit: 100,
                agent: None,
                kind: None,
                decision: None,
                after_id: None,
            },
            follow: true,
            agents: vec![],
            usage: vec![AgentUsageStats {
                agent: "claude-code".into(),
                events: 14,
                est_tokens: 84,
                last_ts: String::new(),
            }],
            activity: vec![1, 3, 2],
            agent_idx: 0,
            kind_idx: 0,
            decision_idx: 0,
            status: String::new(),
            table_state,
        }
    }

    fn render(width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut app = app();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn wide_layout_shows_full_names_headers_and_detail() {
        let out = render(160, 30);
        assert!(out.contains("● LIVE"), "{out}");
        assert!(out.contains("claude-code 14"), "{out}");
        for h in ["agent", "kind", "target", "decision"] {
            assert!(out.contains(h), "missing header {h}\n{out}");
        }
        assert!(out.contains("github › search_issues"), "{out}");
        assert!(!out.contains("claude-co…"), "{out}");
        assert!(out.contains("Detail #3"), "{out}");
        assert!(out.contains("mali_gate.db"), "{out}");
        // วันที่แสดงเฉพาะแถวแรกของแต่ละวัน
        let cursor_row = out.lines().find(|l| l.contains("cursor ")).unwrap();
        assert!(!cursor_row.contains("11-02"), "{out}");
        assert!(out.contains("11-01"), "{out}");
    }

    #[test]
    fn narrow_layout_stacks_detail_below_and_keeps_footer() {
        let out = render(80, 40);
        assert!(out.contains("claude-code"), "{out}");
        assert!(out.contains("Detail #3"), "{out}");
        assert!(out.contains("q quit"), "{out}");
        let lines: Vec<&str> = out.lines().collect();
        let events_line = lines.iter().position(|l| l.contains("Events")).unwrap();
        let detail_line = lines.iter().position(|l| l.contains("Detail #3")).unwrap();
        assert!(detail_line > events_line, "{out}");
    }

    #[test]
    fn helpers_shorten_and_size() {
        assert_eq!(short_kind(&EventKind::SkillSearch), "search");
        assert_eq!(short_kind(&EventKind::PolicyDenied), "denied");
        assert_eq!(agent_col_width(["cursor", "claude-code"].into_iter()), 11);
        assert_eq!(agent_col_width(["a"].into_iter()), AGENT_COL_MIN);
        assert_eq!(
            agent_col_width(std::iter::once("x".repeat(40).as_str())),
            AGENT_COL_MAX
        );
        assert_eq!(truncate_left("abcdef", 4), "…def");
        assert_eq!(
            split_ts("2026-11-02T06:48:30.6Z"),
            ("11-02".into(), "06:48:30".into())
        );
    }
}
