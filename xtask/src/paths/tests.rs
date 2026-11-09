use super::*;

fn paths(target_dir: Option<&str>) -> Paths {
    Paths::new(PathBuf::from("/repo"), target_dir.map(Path::new))
}

#[test]
fn directory_enumeration_errors_are_not_hidden() {
    let entries = [
        Ok(PathBuf::from("visible")),
        Err(std::io::Error::other("refused entry")),
    ];

    assert!(collect_directory_entries(entries.into_iter()).is_err());
}

#[test]
fn directory_classification_errors_are_not_hidden() {
    let entries = [
        Ok(PathBuf::from("visible")),
        Err(anyhow::anyhow!("refused classification")),
    ];

    assert!(collect_classified_entries(entries.into_iter()).is_err());
}

#[test]
fn repository_entry_classification_rejects_redirects_and_special_files() {
    assert!(repository_entry_is_supported(false, false, true, false));
    assert!(repository_entry_is_supported(false, false, false, true));
    assert!(!repository_entry_is_supported(true, false, true, false));
    assert!(!repository_entry_is_supported(false, true, true, false));
    assert!(!repository_entry_is_supported(false, false, false, false));
}

#[test]
fn default_members_reads_a_list_of_paths_as_package_names() {
    let manifest = "\
[workspace]
default-members = [
    \"libs/pixel\",
    \"libs/x86-apic\",
    \"xtask\",
]
";
    assert_eq!(
        default_members(manifest),
        ["pixel", "x86-apic", "xtask"],
        "a nested path names the crate in its last segment"
    );
}

#[test]
fn default_members_ignores_a_comment_and_a_missing_key() {
    let manifest = "\
default-members = [
    # \"libs/not-a-member\" — commented out, and quoted inside the comment
    \"libs/pixel\",
]
";
    assert_eq!(default_members(manifest), ["pixel"]);
    assert!(default_members("[workspace]\nmembers = [\"a\"]\n").is_empty());
    assert!(default_members("").is_empty());
}

#[test]
fn host_packages_are_exactly_the_workspace_default_members() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("root manifest");

    assert_eq!(default_members(&manifest), HOST_PACKAGES);
}

/// The spec is two other constants glued together; nothing but this notices
/// if one of them moves.
#[test]
fn feature_specs_name_the_kernel_package_and_their_features() {
    assert_eq!(
        SELFTEST_FEATURE_SPEC,
        format!("{KERNEL_PACKAGE}/{SELFTEST_FEATURE}")
    );
    assert_eq!(
        REFUSE_PCI_FEATURE_SPEC,
        format!("{KERNEL_PACKAGE}/{REFUSE_PCI_FEATURE}")
    );
}

#[test]
fn kernel_elf_derives_from_profile() {
    let paths = paths(None);
    assert!(
        paths
            .kernel_elf(Profile::Release)
            .ends_with("x86_64-unknown-none/release/wizard-kernel")
    );
    assert!(
        paths
            .kernel_elf(Profile::Dev)
            .ends_with("x86_64-unknown-none/debug/wizard-kernel")
    );
}

#[test]
fn target_dir_defaults_below_root() {
    assert_eq!(paths(None).target_dir(), Path::new("/repo").join("target"));
}

#[test]
fn relative_target_dir_is_joined_to_root() {
    assert_eq!(
        paths(Some("build")).target_dir(),
        Path::new("/repo").join("build")
    );
}

#[test]
fn absolute_target_dir_is_used_verbatim() {
    let absolute = if cfg!(windows) {
        r"C:\shared\target"
    } else {
        "/shared/target"
    };
    assert_eq!(paths(Some(absolute)).target_dir(), Path::new(absolute));
}

#[test]
fn image_lives_in_the_target_dir() {
    assert_eq!(
        paths(Some("build")).image(),
        Path::new("/repo").join("build").join(IMAGE_NAME)
    );
}

/// The names here and the paths in `boot/limine.conf` are one contract
/// written twice.
///
/// `KERNEL_ON_ESP`'s doc says "must match the `path:` line" and nothing
/// read that file. A drift produces an unbootable image, found forty-five
/// seconds into `boot-check` rather than by `cargo test` — the same shape
/// as the `default-members` drift the test above exists for (D-105).
#[test]
fn the_boot_configuration_names_the_files_the_image_builder_writes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let config =
        std::fs::read_to_string(root.join("boot").join("limine.conf")).expect("boot/limine.conf");

    let named = |key: &str| -> String {
        config
            .lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .map(|value| {
                core::iter::Iterator::next(&mut value.trim().rsplit('/'))
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_else(|| panic!("no {key} line in boot/limine.conf"))
    };

    assert_eq!(named("path:"), KERNEL_ON_ESP);
    assert_eq!(named("module_path:"), PROGRAM_ON_ESP);
}

/// Every crate under `libs/` is in both lists.
///
/// The test above compares the two lists **to each other**, so a crate in
/// *neither* is symmetric and passes — and it needs no manifest edit to
/// exist, because Cargo promotes a path dependency inside the workspace
/// directory to a member on its own. So `libs/foo` depended on by
/// `kernel/` builds fine while `cargo xtask test`, clippy's host pass and
/// `doc` never see it. That is the `port-space` drift the sibling test
/// exists to prevent, entered by the other door (D-124).
#[test]
fn every_libs_crate_is_in_both_lists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("root manifest");
    let members = default_members(&manifest);

    let mut found = Vec::new();
    for entry in directory_entries(&root.join("libs")).expect("complete libs/ listing") {
        let path = entry.path();
        if !entry.is_dir() {
            continue;
        }
        // A directory is a crate when it has a manifest; anything else is
        // not this test's business.
        let manifest = path.join("Cargo.toml");
        match std::fs::symlink_metadata(&manifest) {
            Ok(metadata) => {
                let kind = metadata.file_type();
                assert!(
                    kind.is_file() && !kind.is_symlink() && !is_reparse_point(&metadata),
                    "{} is not a regular manifest",
                    manifest.display()
                );
            }
            Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => continue,
            Err(reason) => panic!("inspecting {}: {reason}", manifest.display()),
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| panic!("library path is not UTF-8: {}", path.display()))
            .to_string();
        assert!(
            members.contains(&name),
            "libs/{name} is not in the workspace default-members"
        );
        assert!(
            HOST_PACKAGES.contains(&name.as_str()),
            "libs/{name} is not in HOST_PACKAGES"
        );
        found.push(name);
    }
    assert!(
        found.len() > 15,
        "only {} crates found in libs/",
        found.len()
    );
}
