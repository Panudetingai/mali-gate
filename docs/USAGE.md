# Mali Gate — คู่มือการใช้งาน

Mali Gate (`mali_gate`) เป็นประตูกลาง (Skill Hub + MCP Gateway) ให้ AI agent (Cursor, Claude Code, Codex, Gemini CLI ฯลฯ) ค้นหา/โหลด skills, เชื่อม MCP ภายนอก, และถูกบันทึก log ผ่าน hub เดียว — พร้อม policy แบบ allow / deny / ask

---

## 1. ติดตั้ง binary (build จริง)

### จาก source (Rust)

```powershell
cd D:\mali_gate
cargo build --release -p hub-cli
```

ไฟล์ที่ได้ (target ถูก redirect ตาม `.cargo/config.toml`):

| ไฟล์ | ตำแหน่ง |
|------|---------|
| `mali_gate.exe` | `D:\cargo-target\skillhub\release\mali_gate.exe` |
| `mali_gate_ctl.exe` | `D:\cargo-target\skillhub\release\mali_gate_ctl.exe` |

- **`mali_gate`** — รัน MCP server (`serve`) และคำสั่งที่ agent อาจล็อก process บน Windows
- **`mali_gate_ctl`** — คำสั่ง CLI อื่น (`logs`, `stats`, …) ขณะ Cursor กำลัง spawn `mali_gate serve` อยู่ (exe คนละตัว ไม่ชน lock)

เวอร์ชัน:

```powershell
D:\cargo-target\skillhub\release\mali_gate.exe --version
```

### วาง binary ให้ agent เรียกได้

**แนะนำ:** รัน `init` จาก binary ที่ build แล้ว — มันจะคัดลอกตัวเองไป `~/.mali_gate/bin/` และตั้ง MCP ให้ client ที่พบ

```powershell
D:\cargo-target\skillhub\release\mali_gate.exe init
```

หรือคัดลอกเอง:

```powershell
New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.mali_gate\bin"
Copy-Item D:\cargo-target\skillhub\release\mali_gate.exe "$env:USERPROFILE\.mali_gate\bin\"
Copy-Item D:\cargo-target\skillhub\release\mali_gate_ctl.exe "$env:USERPROFILE\.mali_gate\bin\"
```

จากนั้นเพิ่ม `%USERPROFILE%\.mali_gate\bin` ลง PATH (หรือปล่อยให้ `init` ทำ — ยกเว้น `--no-path`)

---

## 2. เริ่มต้นครั้งแรก

```powershell
mali_gate init
```

`init` จะ:

1. ติดตั้ง/อัปเดต binary ที่ `~/.mali_gate/bin/`
2. สแกน AI CLI ที่มีในเครื่อง (Claude Code, Codex, Gemini, **Cursor**, OpenCode, Windsurf, Qwen)
3. สร้าง agent + token ต่อ client, เขียน config MCP (เช่น `%USERPROFILE%\.cursor\mcp.json`)
4. สร้าง `policy.toml` เริ่มต้น (default deny) ถ้ายังไม่มี
5. ทด handshake MCP จริง (ข้ามได้ด้วย `--no-verify`)

ตัวเลือก:

| ฟлаг | ความหมาย |
|------|----------|
| `--dry-run` | แสดงว่าจะเปลี่ยนอะไร ไม่เขียนไฟล์ |
| `--no-verify` | ไม่ทด MCP handshake |
| `--no-path` | ไม่แก้ PATH ของผู้ใช้ |

หลัง `init` ให้ **รีสตาร์ท Cursor** (หรือ reload MCP) แล้วเปิด server ชื่อ `mali-gate` ใน MCP settings

---

## 3. ข้อมูลและโฟลเดอร์

ค่าเริ่มต้นอยู่ที่ `~/.mali_gate/` (หรือตั้ง `MALI_GATE_HOME` / legacy `SKILLHUB_HOME`)

| Path | ใช้ทำอะไร |
|------|-----------|
| `skills/` | คลัง skill (agent ไม่ควรสแกนโฟลเดอร์นี้เอง — ให้โหลดผ่าน hub) |
| `policy.toml` | สิทธิ์ skills / MCP ต่อ agent |
| `config.toml` | MCP servers ภายนอก, allowlist ติดตั้ง skill, registry index |
| `mali_gate.db` | index skills (FTS) + log events + agents/tokens |
| `bin/mali_gate.exe` | binary ที่ MCP client spawn |

---

## 4. เชื่อม Cursor / agent กับ hub

### อัตโนมัติ (หลัง `init`)

ตัวอย่าง `%USERPROFILE%\.cursor\mcp.json`:

```json
{
  "mcpServers": {
    "mali-gate": {
      "command": "C:\\Users\\YOU\\.mali_gate\\bin\\mali_gate.exe",
      "args": ["serve"],
      "env": {
        "MALI_GATE_TOKEN": "<token-ของ-agent-cursor>"
      }
    }
  }
}
```

- **`MALI_GATE_TOKEN`** บังคับเมื่อมี agent ลงทะเบียนใน DB แล้ว — token ถูกเก็บใน config ของ client ไม่พิมพ์ซ้ำบนจอ
- อย่า commit token ลง git

### รัน MCP ด้วยมือ (ทดสอบ)

```powershell
$env:MALI_GATE_TOKEN = "<token>"
mali_gate serve
```

---

## 5. Meta-tools ที่ agent ใช้ผ่าน hub

| Tool | หน้าที่ |
|------|---------|
| `skills_search(query)` | ค้น skill — ได้ `results` (ใช้ได้) + `requestable` (ขอสิทธิได้) |
| `skills_load(name)` | โหลด SKILL.md + รายการไฟล์อ้างอิง |
| `skills_read_file(name, path)` | อ่านไฟล์ใต้ skill (กัน path traversal) |
| `skills_install(source)` | ขอติดตั้ง skill (รอผู้ใช้อนุมัติ) |
| `mcp_list(query?)` | ราย MCP server ที่ใช้ได้/ขอได้ (ไม่ส่งรายชื่อ tool ทั้งหมด) |
| `connect(server)` | เปิดการเชื่อม MCP/API — คืนเฉพาะ tool ที่มีสิทธิ |
| `call(server, tool, args)` | เรียก tool ผ่าน hub (log + policy) |
| `release(server)` | ตัดการเชื่อมเมื่อจบงาน |

แนวทาง: **ค้นก่อน โหลด/เชื่อมเท่าที่ต้องใช้ แล้ว `release`**

---

## 6. คำสั่ง CLI หลัก

### Shell แบบ interactive

```powershell
mali_gate
```

คำสั่ง slash: `/help`, `/list`, `/refresh`, `/search <คำ>`, `/init`, `/logs`, `/exit` ฯลฯ

### Skills (index ในเครื่อง)

```powershell
mali_gate skills refresh          # สแกนโฟลเดอร์ skills/ ลง DB
mali_gate skills list
mali_gate skills search "รายงาน"
# รวม skill จาก ~/.cursor/skills, ~/.claude/skills ฯลฯ เข้า ~/.mali_gate/skills
mali_gate skills sync --dry-run
mali_gate skills sync --clean-sources   # sync + ลบ copy ต้นทางที่ซ้ำกับคลัง

# ลบ copy ซ้ำในโฟลเดอร์ AI CLI (skill ต้องมีในคลังแล้ว) — ดู help: mali_gate skills clean --help
mali_gate skills clean --dry-run
mali_gate skills clean
mali_gate skills clean --by-name --yes
mali_gate skills show ponytail
mali_gate skills remove old-name --yes
mali_gate skills grant weekly-report              # ให้ทุก agent (shared)
mali_gate skills grant pdf-master --agent cursor  # ให้ agent เดียว
mali_gate skills revoke deploy-site
```

#### `skills clean` — ลบ copy ซ้ำใน AI CLI

ใช้เมื่อ skill **อยู่ในคลัง** `~/.mali_gate/skills/` แล้ว แต่ยังมีโฟลเดอร์ซ้ำใน Cursor / Claude / Codex / Gemini

| โฟลเดอร์ที่สแกน | หมายเหตุ |
|-----------------|----------|
| `~/.cursor/skills/*` | แต่ละโฟลเดอร์ต้องมี `SKILL.md` |
| `~/.claude/skills/*` | ข้ามโฟลเดอร์ชื่อ `synced` |
| `~/.codex/skills/*` | |
| `~/.gemini/skills/*` | |

**ไม่ลบ** ไฟล์ในคลัง Mali Gate **ไม่ import** skill ใหม่ (ใช้ `skills sync` แทน)

| โหมด | พฤติกรรม |
|------|----------|
| ค่าเริ่มต้น | ลบ copy ใน CLI เมื่อ **เนื้อหาไฟล์ตรงกับคลัง** (hash) |
| `--dry-run` | แสดงรายการที่จะลบ ไม่ลบจริง |
| `--by-name --yes` | ลบเมื่อ **ชื่อ skill ตรงคลัง** แม้เนื้อหาต่าง (ระวัง: copy ใหม่ใน CLI หาย) |

หลัง clean ให้ agent ใช้ skill ผ่าน MCP `mali-gate` (`skills_search` / `skills_load`) ไม่พึ่ง path เก่าใน CLI

```powershell
mali_gate skills clean --help   # คำอธิบายเต็ม (อังกฤษ) จาก clap
```

### Agent และ token

```powershell
mali_gate agent add my-bot --label "My bot"
mali_gate agent list
mali_gate agent revoke my-bot
mali_gate agent lazy cursor --dry-run   # ย้ายของที่เคยใช้เข้า shared, default ask สำหรับของใหม่
```

### Policy แบบ `ask` (รอผู้ใช้ตอบ)

เมื่อ agent ขอ resource ที่ policy เป็น `ask` hub รอ ~120 วินาที:

```powershell
mali_gate approve list
mali_gate approve resolve <id>              # อนุมัติแบบ shared (default)
mali_gate approve resolve <id> --agent      # เฉพาะ agent ที่ขอ
mali_gate approve resolve <id> --once        # ครั้งเดียว
mali_gate approve resolve <id> --no         # ปฏิเสธครั้งนี้
mali_gate approve resolve <id> --deny        # ปฏิเสธและจำ
```

### ติดตั้ง skill

```powershell
mali_gate install run .\path\to\skill
mali_gate install run https://github.com/org/repo.git
# monorepo: SKILL.md อยู่ใน subfolder — ใส่ #path หลัง .git
mali_gate install run https://github.com/DietrichGebert/ponytail.git#skills/ponytail
mali_gate install pending
mali_gate install approve <id> --yes
mali_gate install approve <id> --no
```

Agent เรียก `skills_install` → ดูคิวด้วย `install pending` → อนุมัติด้วย `install approve`

### Log และสถิติ

**บน Windows ขณะ Cursor ใช้ `mali_gate serve`:**

```powershell
mali_gate_ctl logs --limit 30
mali_gate_ctl logs --follow          # TUI สด
mali_gate_ctl logs --follow --plain  # ข้อความธรรมดา
mali_gate_ctl logs --agent cursor --kind skill_load
mali_gate_ctl logs --export events.jsonl
mali_gate_ctl stats
```

ถ้าไม่มี process `serve` ค้างอยู่ ใช้ `mali_gate logs …` แทนได้ (alias ใน repo: `cargo logs --follow`)

---

## 7. ตัวอย่าง `policy.toml`

```toml
[shared]
skills = ["weekly-report"]
mcp = ["github"]

[agents.cursor]
skills = { allow = ["weekly-report", "pdf-*"], deny = ["deploy-*"] }
mcp.github = { allow_tools = ["search_*", "get_*"], deny_tools = ["delete_*"] }
mcp.postgres = "ask"

[agents."*"]
default = "deny"
```

- ไม่มี policy / ไม่ match = **ปฏิเสธ** (default deny)
- **deny ชนะ allow** เสมอ
- สิทธิ์ agent อิง **token** ไม่ใช่ชื่อใน MCP handshake

---

## 8. ตัวอย่าง `config.toml`

```toml
# MCP ภายนอก (stdio รองรับแล้ว; http/rest ยัง stub)
[mcp.servers.github]
transport = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
# ความลับใส่ keychain — ไม่ใส่ token ในไฟล์นี้

[install]
allow_local_paths = true
allow_git_hosts = ["github.com", "gitlab.com", "bitbucket.org"]

[registry]
files = ["C:/path/to/skills-index.json"]
urls = []
```

---

## 9. พัฒนา / build ใน repo

```powershell
cargo dev          # build + mali_gate serve
cargo lint         # fmt + clippy
cargo test --workspace
cargo build --release -p hub-cli
```

Target จริง: `D:\cargo-target\skillhub` — อย่าพึ่ง `target/` ใน workspace

---

## 10. แก้ปัญหาเบื้องต้น

| อาการ | แนวทาง |
|--------|--------|
| MCP ใน Cursor ไม่ขึ้น | ตรวจ `%USERPROFILE%\.cursor\mcp.json`, รีสตาร์ท Cursor, รัน `mali_gate init --no-path` แล้วดู path ไป `~/.mali_gate/bin/` |
| `authentication` / token | `mali_gate agent list`, ตรวจ `MALI_GATE_TOKEN` ใน env ของ MCP entry |
| ค้น skill ไม่เจอ | `mali_gate skills refresh`; วาง skill ใต้ `~/.mali_gate/skills/<name>/SKILL.md` |
| agent ขอ skill แล้วค้าง | เปิดเทอร์มิnal รอ `mali_gate approve resolve …` หรือ `install approve …` |
| รัน `logs` แล้ว error บน Windows | ใช้ `mali_gate_ctl` แทน `mali_gate` ขณะ serve ทำงาน |
| อัปเดต binary หลัง build ใหม่ | รัน `init` อีกครั้ง หรือ copy ไป `~/.mali_gate/bin/` แล้ว reload MCP |

---

## 11. ขอบเขต (ตอนนี้)

- Dashboard เว็บ — เฟส 6 (ยังไม่ทำ)
- MCP HTTP / REST connector — config มีได้ แต่ยังไม่ implement
- รัน scripts ใน skill — ยัง stub

รายละเอียดสถาปัตยกรรมและ roadmap: [CLAUDE.md](../CLAUDE.md)
