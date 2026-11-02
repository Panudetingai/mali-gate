# Mali Gate — Skill Hub + MCP Gateway (Rust CLI)

> ชื่อโปรเจกต์/CLI คือ **Mali Gate** (`mali_gate`); โฟลเดอร์ crate ยังใช้ prefix `hub-`* ตาม workspace เดิม

## โปรเจกต์นี้คืออะไร

ตัวกลางเดียว (CLI/daemon) ที่ AI CLI agent ทุกตัว (Claude Code, Codex, Gemini CLI ฯลฯ) ต่อเข้ามาผ่าน MCP
เพื่อค้นหา/โหลด skills, เรียก MCP/API ภายนอก โดยลงที่เดียว ไม่ต้องลงรายตัว
และทุกการเรียกผ่าน hub ถูกตรวจสิทธิ์และบันทึก log

## เป้าหมาย (ลำดับตามความสำคัญ)

1. ติดตั้ง skills/plugins เข้าคลังกลางได้
2. กำหนดสิทธิ์ต่อ agent ต่อ resource (skill, MCP server, tool) แบบ allow / deny / ask
3. Log ว่า agent ไหนเรียกอะไร เมื่อไหร่ ผลเป็นอย่างไร พร้อม TUI ที่อ่านง่าย
4. Lazy loading: agent ไม่เห็น skills/MCP ทั้งหมด ขอเฉพาะที่ใช้ เชื่อมเป็นงานๆ แล้ว disconnect เมื่อเสร็จ
5. แนะนำ skill ให้ agent และติดตั้งเมื่อหาไม่เจอ (ต้องผ่านการอนุมัติจากผู้ใช้)
6. เชื่อมได้หลายแบบ: MCP stdio, MCP HTTP, REST API (OpenAPI)
7. Dashboard (ทำหลังจากข้อ 1-6 เสร็จเท่านั้น ห้ามเริ่มก่อน)

## หลักการออกแบบที่ต้องไม่ลืม

- Hub คือ "ประตู" ไม่ใช่ "กำแพง": ควบคุมและ log ได้เฉพาะสิ่งที่ผ่าน hub ห้ามเขียนโค้ดหรือเอกสารที่อ้างว่ากัน agent ข้าม hub ได้ 100%
- คลัง skills ต้องไม่อยู่ในที่ที่ agent สแกนเอง (เช่น `~/.claude/skills/`) ไม่งั้น log ไม่เห็น
- Default deny: ไม่มี policy = ปฏิเสธ
- ความลับ (token, API key) เก็บใน OS keychain ห้ามอยู่ใน config/log และห้ามส่งให้ agent
- ติดตั้ง skill จากภายนอก = รับโค้ด/คำสั่งที่ agent จะเชื่อ ต้องผ่าน allowlist แหล่งที่มา + pin hash + ผู้ใช้อนุมัติ
- Log ต้อง redact ข้อมูลอ่อนไหวก่อนเขียนเสมอ
- HTTP transport ผูก `127.0.0.1` เท่านั้น และต้องมี token

## สถาปัตยกรรม

```
Agent (Claude Code / Codex / Gemini) ──MCP──► hub
                                               │
                         Auth (agent token) ─► Policy ─► Logger (SQLite)
                                               │
                              ┌────────────────┴────────────────┐
                        Skill Registry                  Connector Manager
                      (search/load/install)        (lazy connect / release)
                                                   ├─ MCP stdio (spawn)
                                                   ├─ MCP HTTP
                                                   └─ REST (OpenAPI → tool)
```

### Meta-tools ที่ hub เปิดให้ agent (ต้องมีน้อยและคงที่)

| Tool | หน้าที่ |
|---|---|
| `skills_search(query)` | ค้นหา skill: `results` (ใช้ได้) + `requestable` (ขอได้ — hub ถามผู้ใช้ครั้งเดียว) |
| `skills_load(name)` | คืน SKILL.md และรายการไฟล์ใน references/ |
| `skills_read_file(name, path)` | อ่านไฟล์ประกอบ (กัน path traversal) |
| `skills_install(source)` | ขอติดตั้ง skill (ผ่านขั้นอนุมัติเสมอ) |
| `mcp_list(query?)` | รายชื่อ MCP server ที่ agent ใช้ได้/ขอได้ (ไม่ส่งรายชื่อ tool) |
| `connect(server)` | เปิดการเชื่อมต่อ MCP/API ตามงาน — คืนเฉพาะ tool ที่มีสิทธิ์ |
| `call(server, tool, args)` | เรียก tool ผ่าน hub (ตรวจสิทธิ์ + log) |
| `release(server)` | ตัดการเชื่อมต่อเมื่อเสร็จงาน |

- `call(...)` คือทางหลักที่ใช้ได้กับทุก client
- การเปิด tool แบบ dynamic ผ่าน `notifications/tools/list_changed` เป็นทางเสริม (client บางตัวรองรับไม่ครบ ต้องมี fallback)
- Disconnect อัตโนมัติ: agent เรียก `release`, session จบ, หรือ idle เกิน timeout (hub ไม่รู้ว่างานเสร็จเอง)

## โครงสร้าง Workspace

```
mali_gate/   (repo root)
├── Cargo.toml                # workspace
├── CLAUDE.md
├── crates/
│   ├── hub-core/             # types, error, paths (~/.mali_gate)
│   ├── hub-registry/         # skill parsing, index (SQLite FTS5), load/read file
│   ├── hub-policy/           # policy TOML, ตัดสิน allow/deny/ask (stub → เฟส 2)
│   ├── hub-log/              # event schema, redact, SQLite writer (MCP + `mali_gate logs`)
│   ├── hub-connect/          # trait Connector + stdio/http/rest (stub → เฟส 3)
│   ├── hub-mcp/              # MCP server ฝั่ง agent (meta-tools 3 ตัวแรก)
│   ├── hub-tui/              # ratatui log viewer (`logs --follow`)
│   └── hub-cli/              # binary `mali_gate` (clap + REPL)
└── tests/                    # integration tests + fixtures (skills ตัวอย่าง)
```

ข้อมูลผู้ใช้อยู่ที่ `~/.mali_gate/` (`skills/`, `policy.toml`, `mali_gate.db`, `config.toml`)

- env: `MALI_GATE_HOME` (ใหม่), `SKILLHUB_HOME` (legacy อ่านได้)
- DB: `mali_gate.db` โดยค่าเริ่มต้น; ถ้ามีแค่ `hub.db` เก่าจะใช้ต่อ (ตาราง `skills` + `events` อยู่ไฟล์เดียวกันได้)

## Policy (ตัวอย่างรูปแบบ)

```toml
[shared]                       # ทุก agent ได้ (ของที่อนุมัติแบบ --shared มาลงที่นี่)
skills = ["weekly-report"]
mcp = ["github"]

[agents.claude-code]
skills = { allow = ["weekly-report", "pdf-*"], deny = ["deploy-*"] }
mcp.github   = { allow_tools = ["search_*", "get_*"], deny_tools = ["delete_*"] }
mcp.postgres = "ask"

[agents."*"]
default = "deny"
```

- ระบุตัวตน agent ด้วย token แยกต่อ agent (ชื่อใน MCP handshake ใช้เป็นแค่ label ห้ามเชื่อถือเพื่อสิทธิ์)
- ลำดับตัดสิน: deny ของ agent / shared deny_tools → allow ของ agent / `[shared]` → mode → default ของ agent → `"*"` → deny
- agent ใหม่จาก `init` ได้ `default = "ask"` (lazy); agent เดิมที่เป็น `"allow"` เปลี่ยนด้วย `mali_gate agent lazy <id>`
- policy แก้ด้วยโปรแกรมผ่าน `hub_policy::PolicyEditor` เท่านั้น (คงคอมเมนต์, atomic, backup `.mali-gate.bak`); `serve` โหลดใหม่เองเมื่อไฟล์เปลี่ยน

## Log schema (SQLite ตาราง `events`)

`id, ts, agent, session_id, kind (skill_search|skill_load|skill_file|install|connect|call|release|policy_denied|approval), target, tool, args_redacted, decision (allow|deny|ask_approved|ask_denied), result_status, duration_ms, error`

## สถานะล่าสุด (2026-11-02)

**เฟส 1–4 ปิดเกณฑ์หลักแล้ว** — ติดตั้ง skill, registry แนะนำ, `skills_install` + approve

| คำสั่ง / ไฟล์สำคัญ | หมายเหตุ |
|---|---|
| `~/.mali_gate/policy.toml` | default deny, glob skills/MCP tools |
| `~/.mali_gate/config.toml` | MCP servers, `[install]` allowlist, `[registry]` index |
| `mali_gate install run …` / `install pending` / `install approve` | CLI ติดตั้ง + อนุมัติคิวจาก agent |
| MCP `skills_install` / `skills_search` | ค้น registry แนะนำ + ขอติดตั้ง (log `install`) |
| `MALI_GATE_TOKEN` ใน MCP env | บังคับเมื่อมี agent ใน DB |
| `mali_gate agent add/list/revoke` | token hash ใน `mali_gate.db` |
| `mali_gate approve list` / `approve resolve <id> [--shared\|--agent\|--once\|--no\|--deny]` | โหมด `ask` (รอ 120s); ค่าเริ่มต้น `--shared` = ทุก agent + จำ |
| `mali_gate skills grant\|revoke <name> [--agent A]` / `agent lazy <id> [--dry-run]` | จัดสิทธิ์ skill; เปลี่ยน agent เป็น lazy (ย้ายของที่เคยใช้เข้า shared) |
| meta-tools `connect`, `call`, `release` | `hub-connect` + rmcp client stdio |
| `mali_gate init [--dry-run]` | สแกน AI CLI (Claude Code, Codex, Gemini, Cursor, OpenCode, Windsurf, Qwen) → ติดตั้ง binary ที่ `~/.mali_gate/bin/` (MCP spawn จากที่นี่ ไม่ล็อก target/) → ออก token ต่อ agent + เขียน config + policy → ทดสอบ handshake จริงแล้วรายงาน |

**ยังไม่ครบในเฟส 3:** MCP HTTP client, REST/OpenAPI — stub คืน `unsupported`

## แผนงานเป็นเฟส

ทำทีละเฟส จบเฟสต้องผ่านเกณฑ์ "เสร็จ" ก่อนไปต่อ

### เฟส 0 — เตรียมโครงการ (เสร็จแล้ว 2026-11-02)

- [x] สร้าง workspace + crates ว่าง, CI (fmt, clippy, test) — 8 crates stub + .github/workflows/ci.yml
- [x] ตรวจเวอร์ชันและ API ล่าสุดของ `rmcp` และสเปก MCP (เปลี่ยนเร็ว อย่าเดาจากความจำ) — rmcp 3.8.0 / สเปก 2026-07-28 ดู docs/phase-0-rmcp-notes.md
- [x] เตรียม fixture skills 3-5 ตัวสำหรับเทสต์ — tests/fixtures/ (5 ตัว)

- เสร็จเมื่อ: `cargo test` ผ่านบน workspace เปล่า — ✅ ผ่าน (fmt + clippy -D warnings ก็ผ่าน)

### เฟส 1 — MVP: อ่าน skill ผ่าน hub + log (เสร็จเกณฑ์หลัก 2026-11-02)

- [x] parse SKILL.md (YAML frontmatter: name, description) และตรวจความถูกต้อง — `hub-registry`, tests
- [x] index ลง SQLite FTS5, `mali_gate skills refresh|list|search` (trigram + ชั้นสำรอง LIKE — ค้นไทย/อังกฤษ)
- [x] MCP server (stdio) พร้อม `skills_search`, `skills_load`, `skills_read_file` — `hub-mcp`, `mali_gate serve`
- [x] logger เขียน events ลง SQLite — `hub-mcp` เรียก `hub-log` หลังทุก meta-tool (decision=allow ชั่วคราว)
- [x] `mali_gate logs` / `/logs` (ข้อความธรรมดา, `--limit`)
- [ ] integration test flow MCP (แนะนำ แต่ไม่บังคับปิดเฟส)

- เสร็จเมื่อ: Cursor / Claude Code ต่อ hub ได้ ค้นและโหลด skill ได้ และเห็นใน log ว่าเรียกอะไร — **✅**

### เฟส 2 — สิทธิ์ (เสร็จเกณฑ์หลัก 2026-11-02)

- [x] policy TOML + parser + ตัวตัดสิน — `hub-policy` + unit tests
- [x] ออก/ตรวจ token ต่อ agent — `mali_gate agent add/list/revoke`, `MALI_GATE_TOKEN`
- [x] โหมด `ask` — pending ใน DB, `mali_gate approve`, log `ask_approved` / `ask_denied`

- เสร็จเมื่อ: agent ที่ไม่มีสิทธิ์ถูกปฏิเสธและมี `policy_denied` ใน log — **✅**

### เฟส 3 — Connector + lazy connect (stdio ครบ; HTTP/REST ค้าง)

- [x] MCP stdio client (spawn/kill) — `hub-connect`, `ConnectorManager`
- [x] `connect` / `call` / `release` + idle timeout + ปิดเมื่อ `release` / drop / idle

- [~] MCP HTTP แล้วตามด้วย REST — config รองรับ `transport = "http"|"rest"` แต่ยังไม่ implement

- [x] ตรวจสิทธิ์ระดับ tool ก่อน `call`  — `evaluate_mcp_tool`

- เสร็จเมื่อ: เชื่อม MCP จริง 1 ตัว ใช้งาน แล้ว disconnect เองได้ — **✅ stdio** (ตั้ง `[mcp.servers.*]` ใน `config.toml`)

### เฟส 4 — ติดตั้งและแนะนำ skill (เสร็จเกณฑ์หลัก 2026-11-02)

- [x] `mali_gate install run <git-url|path>` — validate SKILL.md, allowlist (`config.toml` `[install]`), pin (`.mali-gate-origin.json`)
- [x] `skills_install` MCP — preview + pending queue, รอ `mali_gate install approve <id> --yes` (120s), log `install`
- [x] `[registry]` ใน config — `files` / `urls`; `skills_search` รวม `registry_suggestions` เมื่อมี index
- [~] scripts ใน skill — ตรวจ path ใต้ `scripts/`; รันจริงยัง stub (`SandboxNotAvailable`)

- เสร็จเมื่อ: agent หา skill ไม่เจอ → เสนอ → ผู้ใช้อนุมัติ → ติดตั้ง → ใช้ได้ ทั้งหมดอยู่ใน log — **✅**

### เฟส 5 — TUI log (เสร็จแล้ว 2026-11-02)

- [x] `mali_gate logs --follow` ด้วย ratatui: แยกสีตาม agent, ฟิลเตอร์ agent/kind/decision (a/t/d), ดูรายละเอียดต่อ event
- [x] สรุปการใช้งานต่อ agent (`mali_gate stats`), export JSONL (`logs --export`)

- เสร็จเมื่อ: ดู log สดและย้อนหลังได้ลื่น — **✅**

### เฟส 6 — Dashboard (หลังเฟส 1-5 จบ)

- axum อ่านจาก `mali_gate.db` เดิม ไม่รื้อ schema

## Stack

tokio, rmcp, clap, serde + toml, rusqlite (FTS5), ratatui + crossterm, axum (เฟส 6), tracing, keyring, notify, thiserror/anyhow

## ข้อตกลงในการเขียนโค้ด

- Rust stable, `cargo fmt` + `cargo clippy -- -D warnings` ต้องผ่านก่อนจบงานทุกครั้ง
- library crates ใช้ `thiserror`, binary ใช้ `anyhow`; ห้าม `unwrap()` ใน non-test code
- ห้าม log ค่าความลับ; ทุกการเขียน log ผ่านฟังก์ชัน redact
- ทุกพาธไฟล์ที่ agent ส่งมาต้อง canonicalize และตรวจว่าอยู่ในโฟลเดอร์ skill (กัน `../`)
- เขียนเทสต์คู่กับฟีเจอร์: unit test สำหรับ policy/parse, integration test สำหรับ MCP flow
- commit เล็กๆ ทีละเรื่อง; แก้ schema ต้องมี migration

## คำสั่งที่ใช้บ่อย

```
cargo build --workspace
cargo test --workspace
cargo fmt --all && cargo clippy --workspace -- -D warnings
cargo run -p hub-cli --bin mali_gate          # REPL ถ้าไม่มี subcommand
cargo run -p hub-cli --bin mali_gate -- serve # MCP stdio (Cursor / .cursor/mcp.json)
cargo run -p hub-cli --bin mali_gate -- init  # ตั้ง MCP config
cargo run -p hub-cli --bin mali_gate -- skills refresh
cargo run -p hub-cli --bin mali_gate_ctl -- logs --limit 20   # ใช้ ctl ขณะ MCP รัน serve (Windows)
cargo ctl logs --follow                                      # อย่าใส่ `--` หลัง ctl
cargo logs --follow                                          # alias สั้น (ดู .cargo/config.toml)
```

## นอกขอบเขต (ตอนนี้)

- Dashboard เว็บ (รอเฟส 6)
- รองรับ Claude บนเว็บ/มือถือ (ต้องเปิด remote server + auth ทำภายหลัง)
- การกัน agent ข้าม hub ด้วย sandbox ระดับ OS (พิจารณาหลังเฟส 5)

## คำถามที่ยังต้องตัดสินใจ

- OS เป้าหมายหลัก (macOS / Linux / Windows) มีผลกับ keychain และ sandbox
- agent ตัวแรกที่จะทดสอบด้วย (แนะนำ Claude Code)
- แหล่ง registry ภายนอกที่จะยอมให้ติดตั้ง
- รูปแบบ sandbox สำหรับรัน scripts (container, bubblewrap, sandbox-exec ฯลฯ)
