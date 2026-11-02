//! index skill ลง SQLite FTS5 — ค้นหาแบบ full-text (รองรับไทย/อังกฤษ)
//!
//! โครงสร้าง:
//! - ตารางปกติ `skills` (id, name, description, dir) — ข้อมูลจริง
//! - ตารางเสมือน `skills_fts` (external content) ผูก rowid กับ `skills.id`
//!   กระจายบันทึกผ่าน trigger ให้ fts ติดตาม skills เองเสมอ
//!
//! tokenizer ที่เลือกคือ `trigram` — เพราะ `unicode61` ทำงานกับไทยไม่ดี (สระ/วรรณยุกต์
//! ลอยตัวทำให้ prefix match พลาดง่าย) ส่วน trigram ค้นย่อยคำยาว ≥3 อักขระได้จริง
//! คำสั้นกว่านั้น (เช่น "db") จะใช้ `LIKE` เป็นชั้นสำรองแทน

use std::path::{Path, PathBuf};

use rusqlite::Connection;

#[derive(thiserror::Error, Debug)]
pub enum IndexError {
    #[error("failed to open database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("failed to scan skill folder: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Parse(#[from] super::skill::ParseError),
}

/// ผลการค้นหาหนึ่งรายการ
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub name: String,
    pub description: String,
    /// โฟลเดอร์ของ skill (สำหรับ skills_load / skills_read_file ใช้อ่านต่อไป)
    pub dir: PathBuf,
}

/// ฐานข้อมูล index ของ skills
pub struct SkillIndex {
    conn: Connection,
}

impl SkillIndex {
    /// เปิด (หรือสร้าง) ฐานข้อมูล index ที่ path ที่กำหนด
    pub fn open(db_path: &Path) -> Result<Self, IndexError> {
        let conn = Connection::open(db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init_schema(&conn)?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<(), IndexError> {
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS skills (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL,
                dir TEXT NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS skills_fts
            USING fts5(name, description, content='skills', content_rowid='id',
                       tokenize='trigram');

            CREATE TRIGGER IF NOT EXISTS skills_fts_ai AFTER INSERT ON skills BEGIN
                INSERT INTO skills_fts(rowid, name, description)
                VALUES (new.id, new.name, new.description);
            END;

            CREATE TRIGGER IF NOT EXISTS skills_fts_ad AFTER DELETE ON skills BEGIN
                INSERT INTO skills_fts(skills_fts, rowid, name, description)
                VALUES ('delete', old.id, old.name, old.description);
            END;

            CREATE TRIGGER IF NOT EXISTS skills_fts_au AFTER UPDATE ON skills BEGIN
                INSERT INTO skills_fts(skills_fts, rowid, name, description)
                VALUES ('delete', old.id, old.name, old.description);
                INSERT INTO skills_fts(rowid, name, description)
                VALUES (new.id, new.name, new.description);
            END;
            ",
        )?;
        Ok(())
    }

    /// สแกนโฟลเดอร์ skill ทั้งหมดใต้ `root` (โฟลเดอร์ที่มี SKILL.md) แล้ว upsert เข้า index
    ///
    /// คืนจำนวน skill ที่ index สำเร็จ (ไฟล์ใด parse ไม่ผ่านจะ error ออกทาง caller)
    pub fn index_dir(&self, root: &Path) -> Result<usize, IndexError> {
        let mut count = 0usize;
        // ลึกแค่ชั้นเดียว: root/<skill-name>/SKILL.md
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let sk_dir = entry.path();
            if !sk_dir.is_dir() {
                continue;
            }
            let skill_md = sk_dir.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            let manifest = super::skill::parse_skill_file(&skill_md)?;
            self.upsert(&manifest)?;
            count += 1;
        }
        Ok(count)
    }

    /// เพิ่มหรือปรับปรุง skill ตามชื่อ (upsert — index ซ้ำไม่สร้างแถวซ้ำ)
    pub fn upsert(&self, m: &super::skill::SkillManifest) -> Result<(), IndexError> {
        const UPSERT: &str = "
            INSERT INTO skills (name, description, dir) VALUES (?1, ?2, ?3)
            ON CONFLICT(name) DO UPDATE SET description = excluded.description,
                                           dir = excluded.dir
        ";
        self.conn.execute(
            UPSERT,
            rusqlite::params![m.name, m.description, m.sk_dir.display().to_string()],
        )?;
        Ok(())
    }

    /// ลบ skill ออกจาก index ตามชื่อ (ไม่ลบไฟล์บนดิสก์)
    pub fn delete_by_name(&self, name: &str) -> Result<bool, IndexError> {
        let n = self.conn.execute(
            "DELETE FROM skills WHERE name = ?1",
            rusqlite::params![name],
        )?;
        Ok(n > 0)
    }

    /// ค้นหา skill ตามชื่อที่ index (exact match)
    pub fn get_by_name(&self, name: &str) -> Result<Option<SearchHit>, IndexError> {
        const ONE: &str = "
            SELECT name, description, dir
            FROM skills
            WHERE name = ?1 COLLATE NOCASE
            LIMIT 1
        ";
        let mut stmt = self.conn.prepare(ONE)?;
        let mut rows = stmt.query(rusqlite::params![name])?;
        if let Some(row) = rows.next()? {
            Ok(Some(SearchHit {
                name: row.get(0)?,
                description: row.get(1)?,
                dir: PathBuf::from(row.get::<_, String>(2)?),
            }))
        } else {
            Ok(None)
        }
    }

    /// รายการ skill ทั้งหมดใน index เรียงตามชื่อ (ให้ `mali_gate skills list` ใช้)
    pub fn list_all(&self) -> Result<Vec<SearchHit>, IndexError> {
        const LIST: &str = "
            SELECT name, description, dir
            FROM skills
            ORDER BY name COLLATE NOCASE
        ";
        let mut stmt = self.conn.prepare(LIST)?;
        let rows = stmt.query_map([], |row| {
            Ok(SearchHit {
                name: row.get(0)?,
                description: row.get(1)?,
                dir: PathBuf::from(row.get::<_, String>(2)?),
            })
        })?;
        let mut all = Vec::new();
        for row in rows {
            all.push(row?);
        }
        Ok(all)
    }

    /// ค้นหา skill ด้วย full-text (จัดอันดับตามความเกี่ยวข้องของ fts5)
    ///
    /// - คำยาว ≥3 อักขระ → หาผ่าน fts5 (trigram, จับ substring ได้ รองรับไทย)
    /// - คำสั้นกว่า หรือเมื่อ fts ไม่เจอ → หาผ่าน `LIKE` (case-insensitive) ตามชั้นสำรอง
    /// - อักขระพิเศษถูกตัดหมดก่อน ไม่มีทางพังด้วย syntax ของ fts5
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit>, IndexError> {
        let tokens = Self::clean_tokens(query);
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let long: Vec<String> = tokens
            .iter()
            .filter(|t| t.chars().count() >= 3)
            .cloned()
            .collect();

        let mut hits = if long.is_empty() {
            Vec::new()
        } else {
            Self::search_fts(&self.conn, &long)?
        };
        if hits.is_empty() {
            hits = Self::search_like(&self.conn, &tokens)?;
        }
        Ok(hits)
    }

    fn search_fts(conn: &Connection, tokens: &[String]) -> Result<Vec<SearchHit>, IndexError> {
        // คำที่ quote เอาไว้เป็น phrase — หลายคำต่อกันด้วยช่องว่าง = ความหมาย AND
        let expr = tokens
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" ");
        let mut stmt = conn.prepare(
            "
            SELECT s.name, s.description, s.dir
            FROM skills_fts AS fts
            JOIN skills AS s ON s.id = fts.rowid
            WHERE skills_fts MATCH ?1
            ORDER BY fts.rank
            ",
        )?;
        let rows = stmt.query_map(rusqlite::params![expr], |row| {
            Ok(SearchHit {
                name: row.get(0)?,
                description: row.get(1)?,
                dir: PathBuf::from(row.get::<_, String>(2)?),
            })
        })?;
        let mut hits = Vec::new();
        for row in rows {
            hits.push(row?);
        }
        Ok(hits)
    }

    fn search_like(conn: &Connection, tokens: &[String]) -> Result<Vec<SearchHit>, IndexError> {
        let mut stmt =
            conn.prepare("SELECT name, description, dir FROM skills ORDER BY name COLLATE NOCASE")?;
        let rows = stmt.query_map([], |row| {
            Ok(SearchHit {
                name: row.get(0)?,
                description: row.get(1)?,
                dir: PathBuf::from(row.get::<_, String>(2)?),
            })
        })?;
        let mut hits = Vec::new();
        for row in rows {
            let hit = row?;
            let hay_name = hit.name.to_lowercase();
            let hay_desc = hit.description.to_lowercase();
            let all_match = tokens
                .iter()
                .all(|t| hay_name.contains(t.as_str()) || hay_desc.contains(t.as_str()));
            if all_match {
                hits.push(hit);
            }
        }
        Ok(hits)
    }

    /// แตก query เป็นคำ: เก็บเฉพาะตัวอักษร/ตัวเลข (ครอบไทย + อังกฤษ) — auto กัน syntax
    fn clean_tokens(query: &str) -> Vec<String> {
        query
            .split_whitespace()
            .map(|word| word.chars().filter(|c| c.is_alphanumeric()).collect())
            .filter(|word: &String| !word.is_empty())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_tokens_strips_everything_but_alnum() {
        assert_eq!(SkillIndex::clean_tokens("pdf"), vec!["pdf"]);
        assert_eq!(
            SkillIndex::clean_tokens("hello \"quoted\" (word)!!"),
            vec!["hello", "quoted", "word"]
        );
        assert_eq!(
            SkillIndex::clean_tokens("รายงาน pdf"),
            vec!["รายงาน", "pdf"]
        );
        assert_eq!(SkillIndex::clean_tokens("!!!"), Vec::<String>::new());
        assert_eq!(SkillIndex::clean_tokens("  "), Vec::<String>::new());
    }
}
