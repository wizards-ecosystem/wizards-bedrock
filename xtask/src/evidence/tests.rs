use super::*;

use crate::test_support::reserve_temp_directory;

#[test]
fn rendering_is_sorted_and_round_trips() {
    let mut record = Record::default();
    record.insert("z.last", "two").unwrap();
    record.insert("a.first", "one").unwrap();
    let rendered = record.render();
    assert!(rendered.starts_with(&format!("{FORMAT}\na.first=one\nz.last=two\n")));
    assert_eq!(parse_manifest(&rendered).unwrap(), record);
    assert!(parse_manifest(rendered.trim_end()).is_err());
    assert!(parse_manifest(&format!("{FORMAT}\nz.last=two\na.first=one\n")).is_err());
}

#[test]
fn artifact_bytes_must_match_the_manifest() {
    let root = reserve_temp_directory("wizard-evidence-artifact-fixture");
    for spec in ARTIFACTS {
        let path = root.join(spec.relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = if spec.prefix.ends_with("kernel") {
            b"same kernel".as_slice()
        } else {
            b"same image".as_slice()
        };
        std::fs::write(path, bytes).unwrap();
    }

    let mut record = Record::default();
    add_artifacts_with_image_bytes(&mut record, &root, b"same image".len() as u64).unwrap();
    verify_artifacts_with_image_bytes(&record, &root, b"same image".len() as u64).unwrap();
    assert!(
        verify_artifacts(&record, &root).is_err(),
        "an undersized release image passed the production size policy"
    );
    std::fs::write(root.join(ARTIFACTS[1].relative), b"evil image").unwrap();
    std::fs::write(root.join(ARTIFACTS[3].relative), b"evil image").unwrap();
    assert!(verify_artifacts_with_image_bytes(&record, &root, b"same image".len() as u64).is_err());

    std::fs::remove_dir_all(root).unwrap();
}
