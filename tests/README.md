# tests/ — integration tests + fixtures ตามโครงสร้างใน CLAUDE.md

## fixtures
`fixtures/` เก็บ skills ตัวอย่างไว้ทดสอบทุกเฟส โดยออกแบบชุดนี้ให้ครอบบรรทัด policy ตัวอย่างใน CLAUDE.md

| fixture | ไว้ทดสอบอะไร |
|---|---|
| `weekly-report/` | ครบขั้นต่ำ: เฉพาะ SKILL.md — allow `weekly-report` ตรงชื่อเลย |
| `pdf-summary/` | skill เล็กที่มีบทบาทใน allow `pdf-*` (wildcard) |
| `deploy-site/` | ถูก `deny = ["deploy-*"]` — ทดสอบว่า deny ชนะ allow เสมอ |
| `db-backup/` | เรียก script → ทดสอบเฟส 3/4 ว่า script รันผ่าน hub กับ sandbox เท่านั้น |
| `pdf-master/` | แตก branch, มี references/ — ทดสอบ FTS5 index + path traversal (กัน `../`) |

## ข้อตกลงในการแตก fixture
- ทุก `SKILL.md` ต้องมี YAML frontmatter อย่างน้อย `name:` กับ `description:` (เฟส 1 จะเขียน parser พร้อม validator)
- `description` ให้สื่องานจริง เพราะทดสอบการค้นหา (FTS5) ทั้งคำไทย/อังกฤษ
- ไฟล์/โฟลเดอร์เพิ่มเติมเฉพาะที่เทสต์ต้องใช้จริง อย่ารีดต้นฉบับไม่เกี่ยวข้อง
