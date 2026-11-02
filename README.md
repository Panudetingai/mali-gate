# Mali Gate

Skill library + MCP gateway สำหรับ AI agent (Cursor, Claude Code, Codex, Gemini CLI ฯลฯ) — ค้นหา/โหลด skills, เชื่อม MCP ภายนอก, policy และ log ที่จุดเดียว

## Build (release)

```powershell
cargo build --release -p hub-cli
```

Binary:

- `D:\cargo-target\skillhub\release\mali_gate.exe` — MCP server + CLI หลัก
- `D:\cargo-target\skillhub\release\mali_gate_ctl.exe` — logs/stats ขณะ `serve` รัน (Windows)

## เริ่มใช้

```powershell
D:\cargo-target\skillhub\release\mali_gate.exe init
```

จากนั้น reload MCP ใน Cursor แล้วใช้ server `mali-gate`

## เอกสาร

- **[คู่มือการใช้งาน (ไทย)](docs/USAGE.md)** — ติดตั้ง, config, policy, approve, logs
- **[CLAUDE.md](CLAUDE.md)** — สถาปัตยกรรม, meta-tools, สถานะเฟส

## คำสั่งที่ใช้บ่อย

| คำสั่ง | ความหมาย |
|--------|----------|
| `mali_gate init` | ตั้ง MCP + token + policy |
| `mali_gate serve` | รัน MCP (stdio) |
| `mali_gate skills refresh` | index skills |
| `mali_gate_ctl logs --follow` | ดู log สด (ขณะ Cursor ใช้ serve) |
| `cargo dev` | dev: build + serve |
| `cargo lint` | fmt + clippy |

ข้อมูลผู้ใช้: `~/.mali_gate/` (`MALI_GATE_HOME`)
