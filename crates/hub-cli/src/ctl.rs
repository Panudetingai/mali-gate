//! `mali_gate_ctl` — binary เดียวกับ `mali_gate` แต่ชื่อไฟล์ต่าง
//! (Windows ล็อก `mali_gate.exe` ขณะ MCP client รัน `serve` อยู่ — ใช้ตัวนี้ build/รัน CLI ได้)
//! แยกไฟล์ entry เพื่อไม่ให้ cargo เตือน "main.rs found in multiple build targets"

include!("main.rs");
