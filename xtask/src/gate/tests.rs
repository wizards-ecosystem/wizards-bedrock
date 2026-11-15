use std::path::PathBuf;

use super::{REGISTRY_END, REGISTRY_START, markdown, verify_documentation};

fn fixture(name: &str) -> PathBuf {
    crate::test_support::reserve_temp_directory(&format!("wizard-gate-{name}"))
}

fn valid_readme() -> String {
    format!(
        "before\n{REGISTRY_START}\n{}{REGISTRY_END}\nafter\n",
        markdown()
    )
}

#[test]
fn a_bounded_ci_readme_preserves_registry_semantics() {
    let fixture = fixture("readme-control");
    let readme = fixture.join("README.md");
    std::fs::write(&readme, valid_readme()).expect("write valid README");
    verify_documentation(&fixture).expect("accept valid registry document");

    std::fs::write(
        &readme,
        "<!-- CI-REGISTRY:START -->\nwrong\n<!-- CI-REGISTRY:END -->\n",
    )
    .expect("write mismatched README");
    let error = verify_documentation(&fixture)
        .expect_err("a mismatched registry was accepted")
        .to_string();
    assert!(error.contains("differs from the executable registry"));
    std::fs::remove_dir_all(fixture).expect("remove gate fixture");
}
