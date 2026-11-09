use std::path::{Path, PathBuf};

use super::{README_BYTES, REGISTRY_END, REGISTRY_START, markdown, verify_documentation};

fn fixture(name: &str) -> PathBuf {
    crate::test_support::reserve_temp_directory(&format!("wizard-gate-{name}"))
}

fn valid_readme() -> String {
    format!(
        "before\n{REGISTRY_START}\n{}{REGISTRY_END}\nafter\n",
        markdown()
    )
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("create README link");
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("create README link");
}

#[test]
fn ci_readme_refuses_a_redirected_file() {
    let fixture = fixture("readme-link");
    let repository = fixture.join("repository");
    let sentinel = fixture.join("sentinel");
    std::fs::create_dir(&repository).expect("create repository fixture");
    std::fs::write(&sentinel, valid_readme()).expect("write external sentinel");
    let readme = repository.join("README.md");
    create_file_symlink(&sentinel, &readme);

    let error = verify_documentation(&repository)
        .expect_err("a redirected README was accepted")
        .to_string();

    assert!(error.contains("not a real file"), "{error}");
    std::fs::remove_file(readme).expect("remove README link");
    std::fs::remove_dir_all(fixture).expect("remove gate fixture");
}

#[test]
fn ci_readme_refuses_oversize_before_parsing() {
    let fixture = fixture("readme-oversize");
    let readme = fixture.join("README.md");
    std::fs::File::create(&readme)
        .expect("create oversized README")
        .set_len(README_BYTES + 1)
        .expect("size oversized README");

    let error = verify_documentation(&fixture)
        .expect_err("an oversized README was accepted")
        .to_string();

    assert!(error.contains("exceeding the 65536-byte"), "{error}");
    std::fs::remove_dir_all(fixture).expect("remove gate fixture");
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
