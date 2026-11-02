# Provisioning + Control Center — Design

- วันที่: 2026-11-02
- สถานะ: รอผู้ใช้ review
- ขอบเขต: เฟส 3–5 ต่อยอด (ไม่ใช่เฟส 6 dashboard เว็บ)

## 1. ปัญหาและเป้าหมาย

ผู้ใช้ต้องคอย setup skills/MCP เองทีละ AI, จำไม่ได้ว่า skill ไหนใช้อยู่หรือเลิกใช้แล้ว,
และ AI A กับ AI B ที่ทำงานแบบเดียวกันเก่งไม่เท่ากันเพราะได้ของไม่เท่ากัน

เป้าหมาย: **hub ดูแลตัวเองให้มากที่สุด**

1. ของที่อนุมัติแล้วไปถึงทุก agent ที่ควรได้ โดยไม่ต้อง setup ซ้ำ
2. agent เห็นเฉพาะของที่มีสิทธิ์ และโหลดเฉพาะที่จะใช้ (lazy จริง ทั้ง skill และ MCP tool)
3. ของที่ไม่มีสิทธิ์ → agent ขอได้ → ผู้ใช้ตอบครั้งเดียว → hub จำ
4. เห็นว่าอะไรใช้อยู่/ไม่ได้ใช้ และเก็บกวาดได้ (โดยผู้ใช้ยืนยัน)
5. ควบคุมทุกอย่างได้จาก TUI: AI CLI (pause/resume, connect/remove), skills, MCP, approvals, logs
6. งานที่นานมี loading animation และไม่ทำให้หน้าจอค้าง

ไม่อยู่ในขอบเขต: dashboard เว็บ, แคตตาล็อก MCP แนะนำ (ทำทีหลัง), MCP HTTP/REST transport (งานเฟส 3 ค้างเดิม),
sandbox scripts, การกัน agent ข้าม hub

หลักการเดิมที่ต้องคง: default deny, deny ชนะ allow, token ไม่ออกจอ/log, log ผ่าน redact,
hub เป็น "ประตู" ไม่ใช่ "กำแพง"

## 2. โมเดลสิทธิ์: shared + ต่อ agent

### 2.1 รูปแบบ policy.toml (ขยาย — ไฟล์เดิมยังใช้ได้)

```toml
# ทุก agent ที่ลงทะเบียนได้ของในส่วนนี้ (ยกเว้นถูก deny ที่ระดับ agent)
[shared]
skills = ["weekly-report", "pdf-*"]
mcp = ["github", "figcraft"]          # ทั้ง server (ทุก tool ยกเว้นที่ deny)

[shared.mcp_tools.github]             # (ไม่บังคับ) จำกัด tool ของ server ใน shared
deny_tools = ["delete_*"]

[agents.claude-code]
default = "ask"                       # ใหม่: ค่าแนะนำสำหรับ agent ใหม่
skills = { allow = ["deploy-docs"], deny = ["deploy-*"] }
mcp.postgres = "ask"

[agents."*"]
default = "deny"
```

### 2.2 ลำดับตัดสิน (skill หรือ MCP tool) สำหรับ agent `A`

1. agent ไม่มีใน DB / ถูก pause → **deny**
2. ตรง deny ของ `A` → **deny**
3. ตรง deny ใน `[shared]` (เช่น `deny_tools`) → **deny**
4. ตรง allow ของ `A` หรืออยู่ใน `[shared]` → **allow**
5. กฎโหมดเฉพาะ (`mcp.<server> = "ask"`) → ตามนั้น
6. `default` ของ `A` → ตามนั้น; ไม่มี `[agents.A]` → `[agents."*"]` → ไม่มี → **deny**

agent ที่มี `default = "allow"` (เช่นค่าปัจจุบันของ cursor/claude-code/codex/opencode/gemini-cli/qwen-code)
ยังเห็นทุกอย่างเหมือนเดิม — **ไม่เปลี่ยนให้เงียบๆ** ดู 2.5

### 2.3 การมองเห็น (lazy)

| Meta-tool | พฤติกรรมใหม่ |
|---|---|
| `skills_search(query)` | `results`: เฉพาะที่ verdict = allow; `requestable`: ที่อยู่ในคลังและ verdict = ask (ชื่อ + description สั้น ไม่มีเนื้อหา); ที่ deny ไม่แสดง; `registry_suggestions` เหมือนเดิม |
| `mcp_list(query?)` **ใหม่** | รายชื่อ server ที่ verdict ≠ deny: `name`, `description`, `tool_count`, `access` (`allow`/`ask`) — ไม่มีรายชื่อ tool |
| `connect(server)` | คืนเฉพาะ tool ที่ verdict = allow (+ `requestable_tools` ที่เป็น ask แบบชื่ออย่างเดียว) |
| `skills_load` / `skills_read_file` / `call` | ตรวจสิทธิ์เหมือนเดิม; ถ้า ask → ไหลเข้า flow ขอสิทธิ์ (2.4) |

`description` ของ server มาจาก config (ใส่เอง) หรือ `serverInfo`/`instructions` ตอน handshake ที่ cache ไว้ (ดู 3.3)

meta-tools รวมเป็น 8 ตัว: `skills_search, skills_load, skills_read_file, skills_install, mcp_list, connect, call, release`
— คำอธิบาย tool และ `instructions` ใน handshake ต้องบอกลำดับใช้งาน:
"ค้นก่อน (`skills_search` / `mcp_list`) → โหลด/connect เฉพาะที่ต้องใช้ → `release` เมื่อเสร็จ;
ถ้าเจอใน `requestable` ให้เรียกได้เลย hub จะถามผู้ใช้"

### 2.4 Flow ขอสิทธิ์ (ask → จำ)

1. agent เรียกของที่ verdict = ask → hub สร้าง pending approval (ตาราง `pending_approvals` เดิม) แล้วรอ (สูงสุด 120s ตามเดิม)
2. ผู้ใช้ตอบใน TUI แท็บ Approvals หรือ `mali_gate approve <id>` ด้วย 1 ใน 3:
   - `--shared` / ปุ่ม `s` (**ค่าเริ่มต้น**) → เพิ่มเข้า `[shared]`
   - `--agent` / ปุ่ม `o` → เพิ่มเข้า allow ของ agent นั้น
   - `--no` / ปุ่ม `n` → ปฏิเสธครั้งนี้ (ไม่เขียน deny ถาวร; มีตัวเลือก `N` = deny ถาวรของ agent นั้น)
3. hub เขียน policy (ดู 2.6), log `approval` พร้อม decision `ask_approved`/`ask_denied` และ args `{scope}`, แล้วตอบ agent
4. ครั้งต่อไปไม่ถามซ้ำ
5. `--yes` เดิมของ `approve` = `--shared` (คงความเข้ากันได้)

migration: `pending_approvals` เพิ่มคอลัมน์ `grant_scope TEXT` (`shared|agent|once|deny`) — `resolved` เดิมคงไว้

### 2.5 Migration ของ policy เดิม

- `mali_gate init` และ TUI แท็บ Clients แสดงคำเตือนต่อ agent ที่มี `default = "allow"`:
  "เห็นทุกอย่าง — เปลี่ยนเป็น lazy?" ผู้ใช้ยืนยันจึงเปลี่ยนเป็น `default = "ask"`
  พร้อมย้าย skill/MCP ที่ agent นั้น**เคยใช้จริงใน log** เข้า `[shared]` (ไม่ให้งานที่ใช้อยู่สะดุด)
- agent ใหม่จาก `init` ได้ `default = "ask"` (แทน `"allow"` ที่ทำไว้รอบก่อน)

### 2.6 การเขียน policy.toml

- ใช้ crate `toml_edit` เพื่อคงคอมเมนต์/ลำดับเดิม; เขียนแบบ atomic (tmp + rename) และสำรอง `policy.toml.mali-gate.bak` ครั้งแรก
- โค้ดเขียนอยู่ใน `hub-policy` (`PolicyEditor`: `grant_shared`, `grant_agent`, `revoke`, `set_default`) — CLI/TUI/MCP เรียกผ่านตัวนี้เท่านั้น
- `serve` โหลด policy ใหม่เมื่อไฟล์เปลี่ยน (เช็ก mtime ก่อนตัดสินแต่ละครั้ง — ถูกกว่า watcher และพอสำหรับ local)

## 3. คลัง MCP

### 3.1 ที่เก็บ

`config.toml` `[mcp.servers.<name>]` เหมือนเดิม เพิ่มฟิลด์ไม่บังคับ:

```toml
[mcp.servers.figcraft]
transport = "stdio"
command = 'C:\Users\me\AppData\Local\figcraft\figcraft-cli.exe'
args = ["mcp"]
env_keys = ["FIGMA_TOKEN"]        # ชื่อ env ที่ต้องใช้ — ค่าจริงอยู่ใน OS keychain (keyring)
description = "Figma design tools"
imported_from = "opencode"        # ข้อมูลประกอบ
```

ความลับ: ค่า `env` ที่ import มาจาก client ถ้าชื่อดูเป็นความลับ (`*TOKEN*`, `*KEY*`, `*SECRET*`, `*PASSWORD*`)
→ ย้ายเข้า keychain (`keyring`, service `mali_gate`, key `mcp/<server>/<ENV>`) และเก็บแค่ชื่อใน `env_keys`;
ค่าอื่นเก็บใน `env = {…}` ปกติ ตอน spawn ประกอบ env จากทั้งสองที่

### 3.2 นำเข้าจาก AI CLI (`mali_gate mcp import` / TUI `m` / ขั้นตอนเสริมใน `init`)

1. อ่าน MCP servers จาก config ของทุก client ที่ `init` รู้จัก (ใช้ `targets_for` เดิม + รูปแบบ JSON/TOML เดิม) — ข้าม `mali-gate`
2. รวมตัวซ้ำ: เทียบ `(command, args)` หลัง normalize; ชื่อชนกันแต่คนละคำสั่ง → เติม suffix `-<client>`
3. แสดงรายการให้ผู้ใช้ติ๊ก (CLI: รายการ + ยืนยัน `y/N`; `--yes` ข้ามได้; `--dry-run` ดูอย่างเดียว)
4. ที่เลือก: เพิ่มเข้า `config.toml`, ทดสอบเชื่อมต่อ (3.3), เพิ่มเข้า `[shared] mcp`,
   แล้ว**เอาออกจาก config ของ client** (backup `*.mali-gate.bak` แบบเดียวกับ init)
5. server ที่ทดสอบไม่ผ่าน → ยังนำเข้าได้ แต่ไม่ลบออกจาก client และรายงานเหตุผล
6. ไม่รองรับ transport ที่ไม่ใช่ stdio (url/http) — รายงานว่า "ข้าม: hub ยังไม่รองรับ HTTP"

### 3.3 เพิ่มเอง + ทดสอบ + cache

- `mali_gate mcp add <name> [--description …] [--env K=V]… [--secret-env K]… -- <command> [args…]`
- `mali_gate mcp list | test <name> | remove <name>`
- ทดสอบ = spawn → initialize → tools/list → ปิด (timeout 30s)
- ผล cache ในตาราง `mcp_catalog(server, tools_json, server_info_json, checked_at, status, error)` (migration ใหม่)
  ใช้ตอบ `mcp_list` / ค้นหา โดยไม่ต้อง spawn server
- ข้อความ error ต้องบอกวิธีแก้ เช่น "command not found: npx — ติดตั้ง Node.js หรือใช้ path เต็ม"

## 4. ติดตั้ง/จัดการ skill

ใช้ของเดิม (`install run`, allowlist แหล่ง, pin hash, pending queue) เพิ่ม:

- `mali_gate skills grant <name> [--agent A]` / `revoke <name> [--agent A]` / `remove <name>` (ลบโฟลเดอร์ + index, ถามยืนยัน)
- skill ที่ติดตั้งจากคำขอของ agent → เข้า `[shared]` ตาม scope ที่ผู้ใช้เลือกตอนอนุมัติ

## 5. ติดตามการใช้งาน

- view จากตาราง `events` เดิม (ไม่เพิ่ม schema): ต่อ resource (`skill:<name>`, `mcp:<server>`) →
  `last_used`, `uses_30d`, `agents` (ใครใช้)
- `unused` = มีสิทธิ์อยู่แต่ไม่ถูกใช้นานกว่า `[hub] unused_after_days` (ค่าเริ่มต้น 30)
- แสดงใน TUI (แท็บ Skills/MCP), `mali_gate stats --resources`, และหน้าต้อนรับ REPL ("3 unused — /tui")
- ไม่ลบอัตโนมัติ — เสนอ revoke/remove ให้ผู้ใช้กดยืนยันเท่านั้น

## 6. Pause / Resume / Remove / Connect ต่อ AI CLI

- migration: `agents` เพิ่ม `paused INTEGER NOT NULL DEFAULT 0`, `paused_at TEXT`
- `serve`: ก่อนทุก meta-tool อ่าน `paused` ของ agent ตัวเอง (query เดียว ใช้ connection ค้าง)
  - paused → ปฏิเสธด้วยข้อความ "agent paused by user (mali_gate)" + log `policy_denied` (args `{reason:"paused"}`)
  - ตอนเปลี่ยนเป็น paused ครั้งแรกที่ serve เห็น → `release` connection upstream ทั้งหมดของ session นั้น
- CLI: `mali_gate agent pause|resume <id>`
- Remove = เอา entry `mali-gate` ออกจาก config ทุกไฟล์ของ client นั้น (backup) + `agent revoke`
- Connect = `init` เฉพาะ client เดียว: `mali_gate init --only <agent-id>`
- refactor: แยกส่วน "ต่อ client หนึ่งตัว" ใน `init.rs` เป็นฟังก์ชันสาธารณะ (`connect_client`, `remove_client`,
  `verify_client`) เพื่อให้ CLI และ TUI ใช้ร่วมกัน

## 7. TUI ศูนย์ควบคุม

### 7.1 โครง

- `mali_gate tui` และ `/tui` ใน REPL เปิดที่แท็บ Clients; `logs --follow` เปิดที่แท็บ Logs (พฤติกรรมเดิมคงไว้)
- แถบหัว: `● LIVE` + แท็บ `1 Clients 2 Skills 3 MCP 4 Approvals(n) 5 Logs`; สลับด้วย `1–5`/`Tab`/`Shift-Tab`
- footer: ปุ่มลัดของแท็บปัจจุบัน + ข้อความผล/สถานะล่าสุด
- จอแคบ < 110 คอลัมน์: detail ไปอยู่ใต้รายการ (แบบเดียวกับ Logs)

### 7.2 แท็บ

| แท็บ | คอลัมน์ | ปุ่ม |
|---|---|---|
| Clients | AI CLI, status (● connected / ⏸ paused / ○ not wired / · not installed / ✗ error), agent, last seen, default (allow ⚠ / ask), skills, mcp | `p` pause/resume, `c` connect, `x` remove, `v` verify, `L` เปลี่ยนเป็น lazy (2.5), `Enter` detail |
| Skills | name, scope (shared / agents…), last used, uses 30d, ป้าย unused | `i` install, `g` grant, `d` revoke, `D` remove, `/` ค้นหา |
| MCP | name, status (✓/✗ + เหตุผลสั้น), tools, scope, last used | `a` add, `m` import, `t` test, `g`/`d` grant/revoke, `D` remove |
| Approvals | เวลา, agent, ขออะไร, เหลือเวลา (นับถอยหลังจาก 120s) | `s` shared, `o` agent only, `n` ไม่ให้, `N` deny ถาวร |
| Logs | (ดีไซน์ปัจจุบัน) | (เดิม) |

- input ข้อความ (URL, ชื่อ, คำสั่ง) ใช้ช่อง prompt บรรทัดเดียวที่ footer; ยืนยันการลบด้วย `y`
- แท็บ Approvals: ตัวเลขกระพริบเมื่อมีคำขอใหม่; คำขอที่หมดเวลาแสดง `expired` แล้วหายไปหลัง 30s

### 7.3 งานเบื้องหลัง + loading animation

- `TaskRunner`: thread pool เล็ก (≤4) รับงาน (`Verify`, `Connect`, `Remove`, `InstallSkill`, `ImportMcp`, `TestMcp`, …),
  ส่งผลกลับผ่าน `mpsc` — event loop ไม่เคยรอ I/O ตรงๆ
- spinner ดอกมะลิ `✿ ❀ ✾ ❁` (เปลี่ยนเฟรมทุก 120ms) + ข้อความ + เวลาที่ผ่านไป บนแถวของ item นั้นและที่ footer
- จบ → ✓/✗ ค้าง 5s แล้วกลับเป็นสถานะปกติ; error เต็มดูได้ใน detail
- `Esc` ยกเลิกงานที่ยกเลิกได้ (verify/test = kill child process)
- ระหว่างงานของ item ใด item นั้นห้ามสั่งงานซ้อน (ปุ่มไม่ทำงาน + บอกที่ footer)

### 7.4 Spinner ใน CLI ปกติ

- โมดูล `progress` ใน hub-cli: `Spinner::start("verifying Claude Code")` → `.ok("connected (7 tools)")` / `.fail(msg)`
- ใช้ใน `init`, `mcp add|test|import`, `install run`
- stdout ไม่ใช่ TTY → ไม่มี animation พิมพ์บรรทัดผลอย่างเดียว

## 8. หน่วยและการแยก crate

| หน่วย | crate | หน้าที่ |
|---|---|---|
| `PolicyEditor` + ลำดับตัดสินใหม่ (shared) | hub-policy | อ่าน/เขียน policy, ตัดสิน |
| `AgentStore` pause/resume | hub-policy | สถานะ agent |
| `McpCatalog` (config + cache + import + keychain) | hub-connect | คลัง MCP |
| `usage` queries | hub-log | ใช้ล่าสุด/unused |
| meta-tools (`mcp_list`, กรองผล, pause check, reload policy) | hub-mcp | ฝั่ง agent |
| `clients` (แยกจาก init: detect/connect/remove/verify) | hub-cli → ย้ายไป crate ใหม่ `hub-clients` ถ้า TUI ต้องใช้ (hub-tui ห้ามพึ่ง hub-cli) | จัดการ AI CLI |
| TUI แท็บ + `TaskRunner` + spinner | hub-tui | หน้าจอ |
| `progress` spinner CLI | hub-cli | CLI |

ตัดสินใจ: สร้าง crate **`hub-clients`** (ย้าย detection/config writers/verify จาก `init.rs`) เพราะทั้ง hub-cli และ hub-tui ต้องใช้

## 9. Error handling

- ทุก action ใน TUI คืน `Result` → แสดง ✗ + ข้อความสั้นที่ footer, รายละเอียดใน detail; ไม่มี action ใดทำให้ TUI ปิด
- เขียนไฟล์ config/policy: atomic + backup ครั้งแรก; อ่านไม่ได้ (JSON/TOML เสีย) → ไม่เขียนทับ รายงานไฟล์+เหตุผล
- DB ล็อก: `busy_timeout` 5s ที่มีแล้ว; poll ล้มเหลวแสดงที่ footer แล้วลองใหม่ (มีแล้ว)
- keychain ใช้ไม่ได้ → import ไม่ย้ายค่าความลับ (คง server ไว้ใน client) และบอกเหตุผล — ห้ามเขียนความลับลง config.toml

## 10. การทดสอบ

- unit: ลำดับตัดสินใหม่ (shared/deny/ask/pause) ครบทุกแถวของ 2.2; `PolicyEditor` คงคอมเมนต์ + round-trip;
  import (dedupe, ชื่อชน, ข้าม mali-gate, แยกความลับ) จาก fixture config ของ Claude/OpenCode/Codex/Cursor;
  usage/unused query; spinner โหมด non-TTY
- integration (hub-mcp, spawn `serve` จริงแบบที่ `verify_connection` ทำ):
  `skills_search` ไม่คืน skill ที่ deny, คืน `requestable` สำหรับ ask; `mcp_list`; `connect` กรอง tool;
  ask → approve shared → policy มีรายการ → เรียกซ้ำไม่ถาม; pause ระหว่าง session → call ถัดไปถูกปฏิเสธ
- TUI: TestBackend วาดแต่ละแท็บที่ 80/160 คอลัมน์; ปุ่มหลักเรียก action ที่ถูกต้อง (ใช้ fake `TaskRunner`)
- ทุกขั้น: `cargo fmt`, `clippy -D warnings`, `cargo test --workspace`

## 11. ลำดับส่งมอบ (แต่ละช่วงใช้งานได้จริง)

1. **สิทธิ์ใหม่ + lazy:** `[shared]`, ลำดับตัดสิน, `PolicyEditor`, กรอง `skills_search`/`connect`, `mcp_list`, reload policy, approve 3 แบบ, migration 2.5
2. **คลัง MCP:** `mcp add/list/test/remove`, cache, import จาก AI CLI + keychain
3. **Pause/Remove/Connect + `hub-clients`:** ย้ายโค้ด init, `agent pause/resume`, `init --only`, serve เช็ก pause
4. **Usage/unused:** queries + `stats --resources` + หน้าต้อนรับ
5. **TUI ศูนย์ควบคุม:** แท็บ Clients/Skills/MCP/Approvals, `TaskRunner`, spinner
6. **Spinner CLI** + เก็บงานเอกสาร (CLAUDE.md, ตารางสถานะ)

## 12. ความเสี่ยง / คำถามที่ยังเปิด

- client บางตัวรีโหลด config เองไม่ได้ → หลัง import/remove ต้องบอกให้รีสตาร์ท (แสดงในรายงานทุกครั้ง)
- การลบ MCP ออกจาก config ของ client เป็นการเปลี่ยนพฤติกรรมของ client โดยตรง — ทำเฉพาะเมื่อทดสอบผ่านและผู้ใช้ยืนยัน, มี backup
- `keyring` บน Windows ใช้ Credential Manager — ต้องทดสอบจริงบนเครื่องผู้ใช้
- ไฟล์ `.claude.json` ถูก Claude Code เขียนบ่อย: เขียนแบบ atomic แล้ว**อ่านกลับตรวจ**ว่าการแก้ยังอยู่
