// Integration test งาน 2: index ลง SQLite FTS5 + ค้นหาได้จริง
// (ตาม TDD: test ตัวนี้ต้อง FAIL ก่อน — ยังไม่มี index)

use hub_registry::index::SkillIndex;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "..", "..", "tests", "fixtures"]
        .iter()
        .collect()
}

fn temp_db(label: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("hub-test-{}-{label}.db", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn indexes_all_five_fixtures() {
    let db = temp_db("count");
    let idx = SkillIndex::open(&db).expect("เปิด db ควรสำเร็จ");
    let n = idx.index_dir(&fixtures_dir()).expect("index ควรสำเร็จ");
    assert_eq!(n, 5, "fixture มี 5 skills");

    // ล้างไฟล์ db ชั่วคราว
    let _ = std::fs::remove_file(&db);
}

#[test]
fn search_pdf_finds_two_skills() {
    let db = temp_db("pdf");
    let idx = SkillIndex::open(&db).unwrap();
    idx.index_dir(&fixtures_dir()).unwrap();

    let hits = idx.search("pdf").expect("ค้นควรสำเร็จ");
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    assert!(
        names.contains(&"pdf-summary") && names.contains(&"pdf-master"),
        "ค้น 'pdf' ต้องเจอ pdf-summary และ pdf-master ได้: {names:?}"
    );
    assert_eq!(hits.len(), 2, "จาก 5 fixtures ควรเจอ 2 ตัวพอดี");

    let _ = std::fs::remove_file(&db);
}

#[test]
fn search_thai_word_finds_weekly_report() {
    let db = temp_db("thai");
    let idx = SkillIndex::open(&db).unwrap();
    idx.index_dir(&fixtures_dir()).unwrap();

    let hits = idx.search("รายงาน").expect("ค้นไทยควรสำเร็จ");
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    assert!(
        names.contains(&"weekly-report"),
        "ค้นคำไทย 'รายงาน' ต้องเจอ weekly-report: {names:?}"
    );

    let _ = std::fs::remove_file(&db);
}

#[test]
fn reindexing_does_not_duplicate() {
    let db = temp_db("dup");
    let idx = SkillIndex::open(&db).unwrap();
    idx.index_dir(&fixtures_dir()).unwrap();
    idx.index_dir(&fixtures_dir()).unwrap();

    let hits = idx.search("pdf").unwrap();
    assert_eq!(hits.len(), 2, "index ซ้ำต้อง upsert ไม่เพิ่มชื่อซ้ำ");

    let _ = std::fs::remove_file(&db);
}

#[test]
fn list_all_returns_sorted_names() {
    let db = temp_db("list");
    let idx = SkillIndex::open(&db).unwrap();
    idx.index_dir(&fixtures_dir()).unwrap();

    let all = idx.list_all().expect("list ควรสำเร็จ");
    assert_eq!(all.len(), 5);
    let names: Vec<&str> = all.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "db-backup",
            "deploy-site",
            "pdf-master",
            "pdf-summary",
            "weekly-report"
        ]
    );
    // ทุกแถวต้องมี dir ชี้โฟลเดอร์ skill จริง
    for hit in &all {
        assert!(
            hit.dir.join("SKILL.md").exists(),
            "dir ของ {} ต้องชี้โฟลเดอร์จริง",
            hit.name
        );
    }

    let _ = std::fs::remove_file(&db);
}

#[test]
fn search_with_fts_syntax_characters_is_safe() {
    let db = temp_db("injection");
    let idx = SkillIndex::open(&db).unwrap();
    idx.index_dir(&fixtures_dir()).unwrap();

    // อักขระพิเศษของ fts5 query ต้องไม่ทำให้ค้นพัง
    for q in ["pdf \"", "pdf*", "(", "AND NOT", ""] {
        let _ = idx.search(q).expect("ค้นด้วยอักขระพิเศษต้องไม่ error");
    }

    let _ = std::fs::remove_file(&db);
}
