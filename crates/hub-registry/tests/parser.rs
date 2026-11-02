// Integration test งาน 1: parser อ่าน fixture จริงใน tests/fixtures/
// (ตาม TDD: test ตัวนี้ต้อง FAIL ก่อน — ยังไม่มี parser)

use std::path::PathBuf;

fn fixture_dir(name: &str) -> PathBuf {
    [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "tests",
        "fixtures",
        name,
        "SKILL.md",
    ]
    .iter()
    .collect()
}

#[test]
fn parses_weekly_report_fixture_name_and_description() {
    let path = fixture_dir("weekly-report");
    let skill = hub_registry::skill::parse_skill_file(&path).expect("parse ควรสำเร็จ");
    assert_eq!(skill.name, "weekly-report");
    assert!(
        skill.description.contains("รายงาน"),
        "description ควรเป็นภาษาไทยที่ใช้ค้นได้ ได้: {}",
        skill.description
    );
    assert_eq!(skill.sk_dir, path.parent().unwrap());
}

#[test]
fn parses_all_five_fixtures() {
    let cases: &[(&str, &str)] = &[
        ("weekly-report", "weekly-report"),
        ("pdf-summary", "pdf-summary"),
        ("deploy-site", "deploy-site"),
        ("db-backup", "db-backup"),
        ("pdf-master", "pdf-master"),
    ];
    for (dir, expected_name) in cases {
        let skill = hub_registry::skill::parse_skill_file(&fixture_dir(dir))
            .unwrap_or_else(|e| panic!("fixture {dir} ต้อง parse ผ่าน แต่ได้ {e}"));
        assert_eq!(skill.name, *expected_name, "dir {dir}");
        assert!(!skill.description.is_empty(), "dir {dir}");
    }
}

#[test]
fn rejects_skill_file_missing_name() {
    let mut dir = std::env::temp_dir();
    dir.push(format!("hub-registry-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SKILL.md");
    std::fs::write(
        &file,
        "---\ndescription: ไม่มี name ใน frontmatter\n---\nเนื้อหา",
    )
    .unwrap();

    let err = hub_registry::skill::parse_skill_file(&file).unwrap_err();
    assert!(matches!(err, hub_registry::skill::ParseError::MissingField));

    let _ = std::fs::remove_dir_all(&dir);
}
