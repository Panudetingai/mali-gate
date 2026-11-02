# rmcp + MCP spec — โน้ตตรวจสอบล่าสุด (เฟส 0)

วันที่ตรวจ: 2026-11-02 (ด้วยการค้นเว็บ ไม่ได้เดาจากความจำ — ตามข้อตกลงใน CLAUDE.md)

## เวอร์ชันล่าสุด
- **rmcp v3.8.0** (มี 71 versions published; 3.x ต้องอ่าน migration guide ก่อน)
- repo ทางการ: https://github.com/modelcontextprotocol/rust-sdk
- ติดตั้ง: `cargo add rmcp --features server`
  (= มี feature `server` เปิด default อยู่แล้ว; ถ้าทำ client เพิ่ม feature ฝั่ง client)

## MCP spec ที่รองรับ
- สเปก stable ล่าสุด: **2026-07-28** (สอดคล้องกับ 2025-11-25 และก่อนหน้า)
  doc: https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning
- MCP ใช้ versioning แบบวันที่; rmcp เจรจา protocol version อัตโนมัติตอน connect
  (`ProtocolVersion::LATEST`, `V_2026_07_28`, `KNOWN_VERSIONS`)
- พฤติกรรมเฉพาะเวอร์ชันถูก gate ไว้ตาม negotiated version (ง่ายต่อการให้ client เก่าและใหม่ใช้ hub ตัวเดียวกัน)

## ฟีเจอร์ใหม่ในสเปก 2026-07-28 ที่มีผลกับ hub
- server discovery & negotiation
- transport-neutral subscriptions
- long-running tasks (เหมาะกับงาน deploy/install ที่ต้องรอ)
- response caching
- header standardization (`Mcp-Method` เป็นต้น)
- JSON Schema 2020-12

## สิ่งที่เฟส 1 (MVP stdio server) จะใช้เป็นทางรอง
- feature `server` (default) เปิด stdio server ได้
- rmcp-macros = procedural macro สร้าง tool implementation (ไม่ต้องเขียน dispatch เอง)
- เก็บ transport อื่นไว้เฟสหลังจาก stdio เสร็จและ log เห็นจริง:

## ข้อควรระวังสำหรับ hub
- อย่าเปิดหลาย feature ที่ไม่ใช้ — feature บวมทำ compile time และ target dir โตเกินจริง
