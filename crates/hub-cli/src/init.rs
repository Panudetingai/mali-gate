//! `mali_gate init` — สแกนหา AI CLI ในเครื่อง, ตั้งค่า MCP ให้ต่อ hub, ทดสอบการเชื่อมต่อ แล้วรายงาน
//!
//! ขั้นตอน:
//! 1. ติดตั้ง binary ชุดเสถียรที่ `~/.mali_gate/bin/` (กัน Windows ล็อกไฟล์ใน target/ ตอน cargo build)
//! 2. สแกน client ที่รู้จัก (PATH + โฟลเดอร์ config)
//! 3. ต่อ client: หา/ออก token ต่อ agent, เขียน config ตามรูปแบบของ client, เพิ่ม policy ถ้ายังไม่มี
//! 4. spawn `mali_gate serve` ด้วย token นั้นจริง → initialize + tools/list → รายงานผล
//!
//! token ไม่ถูกพิมพ์ออกจอ — เขียนลงไฟล์ config ของ client เท่านั้น

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use hub_core::{data_root, db_path, policy_path};
use hub_policy::{AgentStore, PolicyEngine, ensure_default_policy};
use serde_json::{Value, json};

use crate::Palette;

const SERVER_NAME: &str = "mali-gate";
const TOKEN_ENV: &str = "MALI_GATE_TOKEN";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(20);

// ───────────────────────────── client catalogue ─────────────────────────────

/// รูปแบบไฟล์ config ของแต่ละ client
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    /// `{ "mcpServers": { name: { command, args, env } } }` — Cursor, Claude Code, Gemini, Windsurf, Qwen
    McpServersJson,
    /// `{ "mcp": { name: { type: "local", command: [..], environment } } }` — OpenCode
    OpenCodeJson,
    /// `[mcp_servers.name]` ใน TOML — Codex CLI
    CodexToml,
}

#[derive(Debug, Clone)]
pub struct ConfigTarget {
    pub path: PathBuf,
    pub format: ConfigFormat,
    pub scope: &'static str,
    /// project scope: แก้เฉพาะไฟล์ที่มี entry `mali-gate` อยู่แล้ว (ไม่สร้างใหม่ ไม่เพิ่มเอง)
    pub only_if_present: bool,
    /// ใส่ `"type": "stdio"` (Claude Code)
    pub stdio_type: bool,
}

#[derive(Debug, Clone)]
pub struct ClientSpec {
    pub agent_id: &'static str,
    pub label: &'static str,
    pub bins: &'static [&'static str],
    /// โฟลเดอร์ใต้ home ที่บอกว่า client นี้เคยติดตั้ง/ใช้งาน
    pub markers: &'static [&'static str],
}

pub const CLIENTS: &[ClientSpec] = &[
    ClientSpec {
        agent_id: "claude-code",
        label: "Claude Code",
        bins: &["claude"],
        markers: &[".claude"],
    },
    ClientSpec {
        agent_id: "codex",
        label: "Codex CLI",
        bins: &["codex"],
        markers: &[".codex"],
    },
    ClientSpec {
        agent_id: "gemini-cli",
        label: "Gemini CLI",
        bins: &["gemini"],
        markers: &[".gemini"],
    },
    ClientSpec {
        agent_id: "cursor",
        label: "Cursor",
        bins: &["cursor", "cursor-agent"],
        markers: &[".cursor"],
    },
    ClientSpec {
        agent_id: "opencode",
        label: "OpenCode",
        bins: &["opencode"],
        markers: &[".config/opencode"],
    },
    ClientSpec {
        agent_id: "windsurf",
        label: "Windsurf",
        bins: &["windsurf"],
        markers: &[".codeium/windsurf"],
    },
    ClientSpec {
        agent_id: "qwen-code",
        label: "Qwen Code",
        bins: &["qwen"],
        markers: &[".qwen"],
    },
];

/// ไฟล์ config ที่ client แต่ละตัวอ่านจริง
pub fn targets_for(agent_id: &str, home: &Path, project: &Path) -> Vec<ConfigTarget> {
    let user = |path: PathBuf, format| ConfigTarget {
        path,
        format,
        scope: "user",
        only_if_present: false,
        stdio_type: false,
    };
    let proj = |path: PathBuf, format| ConfigTarget {
        path,
        format,
        scope: "project",
        only_if_present: true,
        stdio_type: false,
    };
    use ConfigFormat::*;
    match agent_id {
        "claude-code" => vec![
            // user scope ของ Claude Code อยู่ใน ~/.claude.json (ไม่ใช่ ~/.claude/settings.json)
            ConfigTarget {
                stdio_type: true,
                ..user(home.join(".claude.json"), McpServersJson)
            },
            // project .mcp.json มีลำดับเหนือ user — ถ้ามี entry ค้างอยู่ต้องแก้ด้วย ไม่งั้นทับ user scope
            ConfigTarget {
                stdio_type: true,
                ..proj(project.join(".mcp.json"), McpServersJson)
            },
        ],
        "codex" => vec![user(home.join(".codex").join("config.toml"), CodexToml)],
        "gemini-cli" => vec![
            user(home.join(".gemini").join("settings.json"), McpServersJson),
            proj(
                project.join(".gemini").join("settings.json"),
                McpServersJson,
            ),
        ],
        "cursor" => vec![
            user(home.join(".cursor").join("mcp.json"), McpServersJson),
            proj(project.join(".cursor").join("mcp.json"), McpServersJson),
        ],
        "opencode" => vec![
            user(
                home.join(".config").join("opencode").join("opencode.json"),
                OpenCodeJson,
            ),
            proj(project.join("opencode.json"), OpenCodeJson),
        ],
        "windsurf" => vec![user(
            home.join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
            McpServersJson,
        )],
        "qwen-code" => vec![user(
            home.join(".qwen").join("settings.json"),
            McpServersJson,
        )],
        _ => Vec::new(),
    }
}

// ───────────────────────────── detection ─────────────────────────────

#[derive(Debug, Clone)]
pub struct Detection {
    pub spec: &'static ClientSpec,
    pub bin: Option<PathBuf>,
    pub marker: Option<PathBuf>,
}

impl Detection {
    pub fn found(&self) -> bool {
        self.bin.is_some() || self.marker.is_some()
    }
}

pub fn detect_clients(home: &Path) -> Vec<Detection> {
    CLIENTS
        .iter()
        .map(|spec| Detection {
            spec,
            bin: spec.bins.iter().find_map(|b| which(b)),
            marker: spec
                .markers
                .iter()
                .map(|m| home.join(m))
                .find(|p| p.is_dir()),
        })
        .collect()
}

/// หา executable บน PATH (รองรับ PATHEXT บน Windows: .exe/.cmd/.bat/.ps1)
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .chain([".ps1".to_string()])
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ───────────────────────────── binary install ─────────────────────────────

#[derive(Debug)]
pub enum BinaryStatus {
    Installed,
    Updated,
    UpToDate,
    /// รันจากตำแหน่งติดตั้งอยู่แล้ว
    Running,
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "mali_gate.exe"
    } else {
        "mali_gate"
    }
}

pub fn installed_binary_path() -> PathBuf {
    data_root().join("bin").join(exe_name())
}

/// คัดลอก binary ปัจจุบันไป `~/.mali_gate/bin/` — MCP client จะ spawn จากที่นี่
/// (Windows: ไฟล์ที่กำลังรันเขียนทับไม่ได้ แต่ rename ได้ — จึง rename ตัวเก่าออกก่อน)
pub fn install_binary() -> anyhow::Result<(PathBuf, BinaryStatus)> {
    let src = std::env::current_exe()?;
    let dst = installed_binary_path();
    let same = match (src.canonicalize(), dst.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Ok((dst, BinaryStatus::Running));
    }
    let existed = dst.is_file();
    if existed && files_identical(&src, &dst) {
        return Ok((dst, BinaryStatus::UpToDate));
    }
    let dir = dst
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid install path"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{}.new", exe_name()));
    std::fs::copy(&src, &tmp)?;
    swap_in(&tmp, &dst).map_err(|e| anyhow::anyhow!("cannot replace {}: {e}", dst.display()))?;
    Ok((
        dst,
        if existed {
            BinaryStatus::Updated
        } else {
            BinaryStatus::Installed
        },
    ))
}

/// แทนที่ `dst` ด้วย `tmp` แม้ `dst` กำลังรันอยู่ (Windows: rename ได้ แต่เขียนทับไม่ได้)
/// ตัวเก่าย้ายไป `<dst>.old`, `.old1`, … — ชื่อที่ยังล็อกอยู่จากรอบก่อนจะถูกข้าม
fn swap_in(tmp: &Path, dst: &Path) -> io::Result<()> {
    if dst.exists() {
        let mut moved = false;
        for i in 0..32 {
            let mut name = dst.as_os_str().to_owned();
            name.push(if i == 0 {
                ".old".to_string()
            } else {
                format!(".old{i}")
            });
            let old = PathBuf::from(name);
            // ลบได้ = ตัวเก่ารอบก่อนเลิกรันแล้ว; ลบไม่ได้ = ยังรันอยู่ ลองชื่อถัดไป
            if old.exists() && std::fs::remove_file(&old).is_err() {
                continue;
            }
            std::fs::rename(dst, &old)?;
            moved = true;
            break;
        }
        if !moved {
            return Err(io::Error::other("too many old copies still running"));
        }
    }
    std::fs::rename(tmp, dst)
}

fn files_identical(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// ตรวจว่า `dir` อยู่ใน PATH ของ process นี้หรือไม่
fn dir_on_path(dir: &Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let want = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    std::env::split_paths(&path).any(|p| p.canonicalize().unwrap_or(p) == want)
}

/// Windows: เพิ่ม `dir` ใน PATH ของ user (ถาวร) — คืน true ถ้าเพิ่มใหม่
#[cfg(windows)]
fn add_to_user_path(dir: &Path) -> anyhow::Result<bool> {
    let dir_s = dir.display().to_string().replace('\'', "''");
    let script = format!(
        "$d='{dir_s}'; $p=[Environment]::GetEnvironmentVariable('Path','User'); \
         if ($null -eq $p) {{ $p='' }}; \
         if (($p -split ';') -contains $d) {{ 'present' }} else {{ \
           $n = if ($p -eq '') {{ $d }} else {{ $p.TrimEnd(';') + ';' + $d }}; \
           [Environment]::SetEnvironmentVariable('Path',$n,'User'); 'added' }}"
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim() == "added")
}

#[cfg(not(windows))]
fn add_to_user_path(_dir: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("add `~/.mali_gate/bin` to PATH in your shell profile")
}

// ───────────────────────────── config read/merge ─────────────────────────────

fn read_json(path: &Path) -> anyhow::Result<Value> {
    if !path.is_file() {
        return Ok(json!({}));
    }
    let raw = std::fs::read_to_string(path)?;
    let raw = raw.trim_start_matches('\u{feff}');
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(raw)
        .map_err(|e| anyhow::anyhow!("invalid JSON in {}: {e}", path.display()))
}

/// entry `mali-gate` ที่มีอยู่ในไฟล์ (ถ้ามี)
fn existing_entry(target: &ConfigTarget) -> anyhow::Result<Option<Value>> {
    match target.format {
        ConfigFormat::McpServersJson => Ok(read_json(&target.path)?
            .get("mcpServers")
            .and_then(|s| s.get(SERVER_NAME))
            .cloned()),
        ConfigFormat::OpenCodeJson => Ok(read_json(&target.path)?
            .get("mcp")
            .and_then(|s| s.get(SERVER_NAME))
            .cloned()),
        ConfigFormat::CodexToml => {
            if !target.path.is_file() {
                return Ok(None);
            }
            let raw = std::fs::read_to_string(&target.path)?;
            let doc: toml::Value = toml::from_str(&raw)
                .map_err(|e| anyhow::anyhow!("invalid TOML in {}: {e}", target.path.display()))?;
            let entry = doc.get("mcp_servers").and_then(|s| s.get(SERVER_NAME));
            Ok(entry.map(|e| serde_json::to_value(e).unwrap_or(Value::Null)))
        }
    }
}

/// token ที่ตั้งไว้ใน entry เดิม
fn entry_token(entry: &Value) -> Option<String> {
    entry
        .get("env")
        .or_else(|| entry.get("environment"))
        .and_then(|e| e.get(TOKEN_ENV))
        .and_then(|t| t.as_str())
        .map(str::to_string)
}

/// สร้าง entry JSON ใหม่ โดยเก็บ env ตัวอื่นของผู้ใช้ไว้
pub fn build_json_entry(
    format: ConfigFormat,
    stdio_type: bool,
    exe: &Path,
    token: &str,
    previous: Option<&Value>,
) -> Value {
    let env_key = if format == ConfigFormat::OpenCodeJson {
        "environment"
    } else {
        "env"
    };
    let mut env = previous
        .and_then(|p| p.get(env_key))
        .and_then(|e| e.as_object())
        .cloned()
        .unwrap_or_default();
    env.insert(TOKEN_ENV.into(), json!(token));
    let exe_s = exe.display().to_string();
    match format {
        ConfigFormat::OpenCodeJson => json!({
            "type": "local",
            "command": [exe_s, "serve"],
            "environment": env,
            "enabled": true
        }),
        _ => {
            let mut v = json!({
                "command": exe_s,
                "args": ["serve"],
                "env": env
            });
            if stdio_type {
                v["type"] = json!("stdio");
            }
            v
        }
    }
}

/// ใส่ entry ลงใน JSON เดิม (คืน text ใหม่ + มีการเปลี่ยนหรือไม่)
pub fn merge_json(
    existing: &Value,
    top_key: &str,
    entry: &Value,
) -> anyhow::Result<(String, bool)> {
    let mut root = existing.clone();
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("config root must be a JSON object"))?;
    let servers = obj.entry(top_key).or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("`{top_key}` must be a JSON object"))?;
    let changed = servers.get(SERVER_NAME) != Some(entry);
    servers.insert(SERVER_NAME.into(), entry.clone());
    Ok((serde_json::to_string_pretty(&root)? + "\n", changed))
}

/// แทนที่ `[mcp_servers.mali-gate]` (และตารางย่อย) ใน TOML ของ Codex — ส่วนอื่นของไฟล์คงเดิม
pub fn merge_codex_toml(existing: &str, exe: &Path, token: &str) -> anyhow::Result<(String, bool)> {
    let mut kept: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in existing.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            let norm: String = t
                .chars()
                .filter(|c| !matches!(c, ' ' | '\t' | '"' | '\''))
                .collect();
            let head = format!("[mcp_servers.{SERVER_NAME}");
            skipping = norm == format!("{head}]") || norm.starts_with(&format!("{head}."));
        }
        if !skipping {
            kept.push(line);
        }
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    let exe_s = exe.display().to_string();
    // literal string ('...') ไม่ต้อง escape backslash ของ path Windows
    let cmd = if exe_s.contains('\'') {
        toml::Value::String(exe_s.clone()).to_string()
    } else {
        format!("'{exe_s}'")
    };
    out.push_str(&format!(
        "[mcp_servers.{SERVER_NAME}]\ncommand = {cmd}\nargs = [\"serve\"]\n\n\
         [mcp_servers.{SERVER_NAME}.env]\n{TOKEN_ENV} = \"{token}\"\n"
    ));
    let doc: toml::Value = toml::from_str(&out)
        .map_err(|e| anyhow::anyhow!("merged Codex config would be invalid TOML: {e}"))?;
    let ok = doc
        .get("mcp_servers")
        .and_then(|s| s.get(SERVER_NAME))
        .and_then(|e| e.get("command"))
        .and_then(|c| c.as_str())
        == Some(exe_s.as_str());
    if !ok {
        anyhow::bail!("merged Codex config did not round-trip");
    }
    let changed = out.trim_end() != existing.trim_end();
    Ok((out, changed))
}

/// เขียนแบบ atomic + สำรองไฟล์เดิมครั้งแรกเป็น `<file>.mali-gate.bak`
fn write_config(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.is_file() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".mali-gate.bak");
        let bak = PathBuf::from(bak);
        if !bak.exists() {
            std::fs::copy(path, &bak)?;
        }
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".mali-gate.tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn apply_target(target: &ConfigTarget, exe: &Path, token: &str) -> anyhow::Result<bool> {
    match target.format {
        ConfigFormat::CodexToml => {
            let existing = if target.path.is_file() {
                std::fs::read_to_string(&target.path)?
            } else {
                String::new()
            };
            let (text, changed) = merge_codex_toml(&existing, exe, token)?;
            if changed {
                write_config(&target.path, &text)?;
            }
            Ok(changed)
        }
        fmt => {
            let root = read_json(&target.path)?;
            let top = if fmt == ConfigFormat::OpenCodeJson {
                "mcp"
            } else {
                "mcpServers"
            };
            let prev = root.get(top).and_then(|s| s.get(SERVER_NAME)).cloned();
            let entry = build_json_entry(fmt, target.stdio_type, exe, token, prev.as_ref());
            let (text, changed) = merge_json(&root, top, &entry)?;
            if changed {
                write_config(&target.path, &text)?;
            }
            Ok(changed)
        }
    }
}

// ───────────────────────────── tokens + policy ─────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenAction {
    Reused,
    Created,
    Rotated,
}

fn resolve_token(
    store: &AgentStore,
    spec: &ClientSpec,
    found: &[String],
) -> anyhow::Result<(String, TokenAction)> {
    let err = |e: hub_policy::AgentError| anyhow::anyhow!("{e}");
    for tok in found {
        if store.resolve_token(tok).map_err(err)?.as_deref() == Some(spec.agent_id) {
            return Ok((tok.clone(), TokenAction::Reused));
        }
    }
    if store.exists(spec.agent_id).map_err(err)? {
        // DB เก็บแค่ hash — กู้ token เดิมไม่ได้ ต้องออกใหม่
        return Ok((
            store.rotate(spec.agent_id).map_err(err)?,
            TokenAction::Rotated,
        ));
    }
    let (tok, _) = store.add(spec.agent_id, spec.label).map_err(err)?;
    Ok((tok, TokenAction::Created))
}

/// เพิ่ม `[agents.<id>]` ใน policy.toml ถ้ายังไม่มี — คืน true ถ้าเพิ่ม
fn ensure_policy_entry(agent_id: &str) -> anyhow::Result<bool> {
    let path = policy_path();
    ensure_default_policy(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
    let text = std::fs::read_to_string(&path)?;
    let engine = PolicyEngine::from_toml(&text)
        .map_err(|e| anyhow::anyhow!("invalid policy {}: {e}", path.display()))?;
    if engine.agent_ids().any(|id| id == agent_id) {
        return Ok(false);
    }
    let block = format!(
        "\n# added by `mali_gate init` — ปรับสิทธิ์ได้ตามต้องการ\n\
         [agents.{agent_id}]\ndefault = \"ask\"\n"
    );
    let mut f = std::fs::OpenOptions::new().append(true).open(&path)?;
    f.write_all(block.as_bytes())?;
    Ok(true)
}

fn agent_sees_everything(agent_id: &str) -> bool {
    std::fs::read_to_string(policy_path())
        .ok()
        .and_then(|t| PolicyEngine::from_toml(&t).ok())
        .is_some_and(|e| e.sees_everything(agent_id))
}

// ───────────────────────────── verification ─────────────────────────────

#[derive(Debug, Clone)]
pub enum Verify {
    Ok { tools: usize },
    Failed(String),
}

/// spawn `serve` แบบเดียวกับที่ client จะทำ แล้วคุย MCP จริง: initialize → tools/list
pub fn verify_connection(exe: &Path, token: &str) -> Verify {
    let mut child = match Command::new(exe)
        .arg("serve")
        .env(TOKEN_ENV, token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return Verify::Failed(format!("spawn failed: {e}")),
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        return Verify::Failed("no stdio pipes".into());
    };

    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    let (etx, erx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut buf = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            buf.push_str(&line);
            buf.push('\n');
        }
        let _ = etx.send(buf);
    });

    let send = |stdin: &mut std::process::ChildStdin, v: Value| -> io::Result<()> {
        writeln!(stdin, "{v}")?;
        stdin.flush()
    };
    let wait_id = |id: i64| -> Option<Value> {
        let deadline = std::time::Instant::now() + VERIFY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(v) if v.get("id").and_then(Value::as_i64) == Some(id) => return Some(v),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    };

    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"mali_gate-init","version":env!("CARGO_PKG_VERSION")}}});
    let result = (|| {
        send(&mut stdin, init).map_err(|e| format!("write: {e}"))?;
        let resp = wait_id(1).ok_or("no initialize response")?;
        if let Some(err) = resp.get("error") {
            return Err(format!("initialize error: {err}"));
        }
        send(
            &mut stdin,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .map_err(|e| format!("write: {e}"))?;
        send(
            &mut stdin,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        )
        .map_err(|e| format!("write: {e}"))?;
        let resp = wait_id(2).ok_or("no tools/list response")?;
        if let Some(err) = resp.get("error") {
            return Err(format!("tools/list error: {err}"));
        }
        let n = resp
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        Ok(n)
    })();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    match result {
        Ok(tools) => Verify::Ok { tools },
        Err(msg) => {
            let stderr = erx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
            let tail = stderr.lines().rev().find(|l| !l.trim().is_empty());
            Verify::Failed(match tail {
                Some(t) => format!("{msg} — {}", t.replace(token, "mg_***")),
                None => msg,
            })
        }
    }
}

// ───────────────────────────── orchestration ─────────────────────────────

#[derive(Debug, Default)]
pub struct InitOptions {
    pub dry_run: bool,
    pub skip_verify: bool,
    pub no_path: bool,
}

struct ClientReport {
    det: Detection,
    files: Vec<(PathBuf, &'static str, Result<bool, String>)>,
    token: Option<TokenAction>,
    policy_added: bool,
    /// default = "allow" → เห็นทุกอย่าง (แนะนำ `agent lazy`)
    sees_everything: bool,
    verify: Option<Verify>,
    error: Option<String>,
}

fn find_git_root(start: &Path) -> Option<PathBuf> {
    let mut cur = start.to_path_buf();
    loop {
        if cur.join(".git").exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

fn user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// `mali_gate init` และ `/init` ใน REPL
pub fn run_init(opts: &InitOptions, paint: &Palette) -> anyhow::Result<()> {
    let home = user_home().ok_or_else(|| anyhow::anyhow!("cannot find home directory"))?;
    let cwd = std::env::current_dir()?;
    let project = find_git_root(&cwd).unwrap_or(cwd);

    println!(
        "  {}",
        paint.heading("Mali Gate init — scan AI CLIs + wire MCP")
    );
    println!("  {}", paint.quiet(super::RULE));

    // 1. binary
    let exe = if opts.dry_run {
        installed_binary_path()
    } else {
        let (exe, status) = install_binary()?;
        let note = match status {
            BinaryStatus::Installed => "installed",
            BinaryStatus::Updated => "updated",
            BinaryStatus::UpToDate => "up to date",
            BinaryStatus::Running => "in use",
        };
        println!(
            "  {} {} {}",
            paint.soft("binary:"),
            paint.quiet(&exe.display().to_string()),
            paint.heading(&format!("({note})"))
        );
        exe
    };
    let bin_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    if !opts.dry_run && !opts.no_path && !dir_on_path(&bin_dir) {
        match add_to_user_path(&bin_dir) {
            Ok(true) => println!(
                "  {} {}",
                paint.soft("PATH:"),
                paint.heading("added for current user — open a new terminal to use `mali_gate`")
            ),
            Ok(false) => println!(
                "  {} {}",
                paint.soft("PATH:"),
                paint.quiet("already in user PATH (open a new terminal)")
            ),
            Err(e) => println!("  {} {}", paint.soft("PATH:"), paint.quiet(&e.to_string())),
        }
    }

    // 2. scan
    let detections = detect_clients(&home);
    std::fs::create_dir_all(data_root())?;
    let store = if opts.dry_run {
        None
    } else {
        Some(AgentStore::open(&db_path()).map_err(|e| anyhow::anyhow!("{e}"))?)
    };

    let mut reports = Vec::new();
    for det in detections {
        let mut rep = ClientReport {
            det,
            files: Vec::new(),
            token: None,
            policy_added: false,
            sees_everything: false,
            verify: None,
            error: None,
        };
        if !rep.det.found() {
            reports.push(rep);
            continue;
        }
        let targets: Vec<ConfigTarget> = targets_for(rep.det.spec.agent_id, &home, &project)
            .into_iter()
            .filter(|t| !t.only_if_present || matches!(existing_entry(t), Ok(Some(_))))
            .collect();

        // token เดิมจากทุกไฟล์ของ client นี้ (ใช้ร่วมกันทุกไฟล์)
        let mut found_tokens = Vec::new();
        let mut entries: BTreeMap<PathBuf, bool> = BTreeMap::new();
        for t in &targets {
            match existing_entry(t) {
                Ok(e) => {
                    entries.insert(t.path.clone(), e.is_some());
                    if let Some(tok) = e.as_ref().and_then(entry_token) {
                        found_tokens.push(tok);
                    }
                }
                Err(err) => {
                    rep.files
                        .push((t.path.clone(), t.scope, Err(err.to_string())));
                }
            }
        }

        let Some(store) = &store else {
            for t in &targets {
                if entries.contains_key(&t.path) {
                    let has = entries.get(&t.path).copied().unwrap_or(false);
                    rep.files.push((t.path.clone(), t.scope, Ok(!has)));
                }
            }
            reports.push(rep);
            continue;
        };

        let (token, action) = match resolve_token(store, rep.det.spec, &found_tokens) {
            Ok(v) => v,
            Err(e) => {
                rep.error = Some(format!("token: {e}"));
                reports.push(rep);
                continue;
            }
        };
        rep.token = Some(action);
        match ensure_policy_entry(rep.det.spec.agent_id) {
            Ok(added) => {
                rep.policy_added = added;
                rep.sees_everything = agent_sees_everything(rep.det.spec.agent_id);
            }
            Err(e) => rep.error = Some(format!("policy: {e}")),
        }
        for t in &targets {
            if !entries.contains_key(&t.path) {
                continue; // อ่านไม่ได้ — รายงานแล้วข้างบน ไม่เขียนทับ
            }
            let res = apply_target(t, &exe, &token).map_err(|e| e.to_string());
            rep.files.push((t.path.clone(), t.scope, res));
        }
        if !opts.skip_verify {
            rep.verify = Some(verify_connection(&exe, &token));
        }
        reports.push(rep);
    }

    print_report(&reports, &home, opts, paint);
    Ok(())
}

fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn print_report(reports: &[ClientReport], home: &Path, opts: &InitOptions, paint: &Palette) {
    println!();
    let mut ok = 0usize;
    let mut failed = 0usize;
    for r in reports {
        let spec = r.det.spec;
        if !r.det.found() {
            println!(
                "  {} {:<12} {}",
                paint.quiet("·"),
                spec.label,
                paint.quiet("not installed")
            );
            continue;
        }
        let how = match (&r.det.bin, &r.det.marker) {
            (Some(b), _) => tilde(b, home),
            (None, Some(m)) => format!("{} (config only)", tilde(m, home)),
            _ => String::new(),
        };
        let status = match (&r.error, &r.verify) {
            (Some(e), _) => {
                failed += 1;
                paint.heading(&format!("✗ {e}"))
            }
            (None, Some(Verify::Ok { tools })) => {
                ok += 1;
                paint.heading(&format!("✓ connected ({tools} tools)"))
            }
            (None, Some(Verify::Failed(e))) => {
                failed += 1;
                paint.heading(&format!("✗ {e}"))
            }
            (None, None) if opts.dry_run => paint.soft("would configure"),
            (None, None) => paint.soft("configured (not verified)"),
        };
        println!(
            "  {} {:<12} {}  {}",
            paint.heading("●"),
            spec.label,
            paint.quiet(&format!("agent={}", spec.agent_id)),
            status
        );
        println!("      {}", paint.quiet(&format!("found: {how}")));
        if let Some(t) = r.token {
            let t = match t {
                TokenAction::Reused => "token: kept existing",
                TokenAction::Created => "token: new agent created",
                TokenAction::Rotated => "token: re-issued (old one was lost/invalid)",
            };
            println!("      {}", paint.quiet(t));
        }
        if r.policy_added {
            println!(
                "      {}",
                paint.quiet("policy: added [agents] entry to policy.toml (skills allow *)")
            );
        }
        if r.sees_everything {
            println!(
                "      {}",
                paint.heading(&format!(
                    "policy: default = \"allow\" (sees everything) — run `mali_gate agent lazy {}` to switch to lazy",
                    spec.agent_id
                ))
            );
        }
        for (path, scope, res) in &r.files {
            let what = match res {
                Ok(true) if opts.dry_run => "will write".to_string(),
                Ok(false) if opts.dry_run => "already has entry".to_string(),
                Ok(true) => "written".to_string(),
                Ok(false) => "unchanged".to_string(),
                Err(e) => format!("error: {e}"),
            };
            println!(
                "      {} {}",
                paint.soft(&format!("[{scope}] {what}")),
                paint.quiet(&tilde(path, home))
            );
        }
    }
    println!();
    if opts.dry_run {
        println!("  {}", paint.soft("dry run — nothing written"));
    } else {
        println!(
            "  {}",
            paint.heading(&format!("{ok} client(s) connected, {failed} failed"))
        );
        println!(
            "  {}",
            paint.soft("Restart each AI CLI (or /mcp → reconnect) to pick up the new config.")
        );
        println!(
            "  {}",
            paint.quiet("Tokens live in the client config files above — do not commit them.")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_in_works_when_previous_old_copy_is_stuck() {
        let dir = std::env::temp_dir().join(format!("mg-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dst = dir.join("mali_gate.exe");
        let tmp = dir.join("mali_gate.exe.new");
        std::fs::write(&dst, "old build").unwrap();
        std::fs::write(&tmp, "new build").unwrap();
        // ตัวเก่ารอบก่อนยังรันอยู่ = ลบไม่ได้ (จำลองด้วยโฟลเดอร์ที่มีของข้างใน)
        let stuck = dir.join("mali_gate.exe.old");
        std::fs::create_dir_all(stuck.join("x")).unwrap();
        swap_in(&tmp, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "new build");
        assert!(!tmp.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_entry_keeps_user_env_and_sets_token() {
        let prev = json!({"command":"old","env":{"FOO":"1","MALI_GATE_TOKEN":"mg_old"}});
        let e = build_json_entry(
            ConfigFormat::McpServersJson,
            true,
            Path::new("C:\\x\\mali_gate.exe"),
            "mg_new",
            Some(&prev),
        );
        assert_eq!(e["command"], "C:\\x\\mali_gate.exe");
        assert_eq!(e["args"], json!(["serve"]));
        assert_eq!(e["env"]["FOO"], "1");
        assert_eq!(e["env"]["MALI_GATE_TOKEN"], "mg_new");
        assert_eq!(e["type"], "stdio");
        assert_eq!(entry_token(&e).as_deref(), Some("mg_new"));
    }

    #[test]
    fn opencode_entry_shape() {
        let e = build_json_entry(
            ConfigFormat::OpenCodeJson,
            false,
            Path::new("/bin/mali_gate"),
            "mg_t",
            None,
        );
        assert_eq!(e["type"], "local");
        assert_eq!(e["command"], json!(["/bin/mali_gate", "serve"]));
        assert_eq!(e["environment"]["MALI_GATE_TOKEN"], "mg_t");
        assert_eq!(entry_token(&e).as_deref(), Some("mg_t"));
    }

    #[test]
    fn merge_json_preserves_other_servers() {
        let existing = json!({"mcpServers":{"other":{"command":"x"}},"theme":"dark"});
        let entry = json!({"command":"m","args":["serve"]});
        let (out, changed) = merge_json(&existing, "mcpServers", &entry).unwrap();
        assert!(changed);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["mcpServers"]["mali-gate"]["command"], "m");
        assert_eq!(v["theme"], "dark");
        let (_, again) = merge_json(&v, "mcpServers", &entry).unwrap();
        assert!(!again);
    }

    #[test]
    fn codex_toml_replaces_only_our_section() {
        let existing = "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"x\"\n\n\
                        [mcp_servers.\"mali-gate\"]\ncommand = \"old\"\n\n\
                        [mcp_servers.mali-gate.env]\nMALI_GATE_TOKEN = \"mg_old\"\n\n\
                        [profiles.fast]\nmodel = \"o4\"\n";
        let (out, changed) = merge_codex_toml(
            existing,
            Path::new("C:\\Users\\me\\mali_gate.exe"),
            "mg_new",
        )
        .unwrap();
        assert!(changed);
        let doc: toml::Value = toml::from_str(&out).unwrap();
        assert_eq!(doc["model"].as_str(), Some("gpt-5"));
        assert_eq!(doc["mcp_servers"]["other"]["command"].as_str(), Some("x"));
        assert_eq!(doc["profiles"]["fast"]["model"].as_str(), Some("o4"));
        let mg = &doc["mcp_servers"]["mali-gate"];
        assert_eq!(mg["command"].as_str(), Some("C:\\Users\\me\\mali_gate.exe"));
        assert_eq!(mg["env"]["MALI_GATE_TOKEN"].as_str(), Some("mg_new"));
        assert!(!out.contains("mg_old"));
        let (_, again) =
            merge_codex_toml(&out, Path::new("C:\\Users\\me\\mali_gate.exe"), "mg_new").unwrap();
        assert!(!again);
    }

    #[test]
    fn claude_targets_use_claude_json_not_settings() {
        let t = targets_for("claude-code", Path::new("/h"), Path::new("/p"));
        assert_eq!(t[0].path, Path::new("/h").join(".claude.json"));
        assert!(t[0].stdio_type && !t[0].only_if_present);
        assert_eq!(t[1].path, Path::new("/p").join(".mcp.json"));
        assert!(t[1].only_if_present);
    }

    #[test]
    fn every_client_has_targets() {
        for c in CLIENTS {
            assert!(
                !targets_for(c.agent_id, Path::new("/h"), Path::new("/p")).is_empty(),
                "{}",
                c.agent_id
            );
        }
    }
}
