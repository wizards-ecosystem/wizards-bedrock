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
    assert!(repository_entry_is_supported(EntryKind::Directory, false));
    assert!(repository_entry_is_supported(EntryKind::File, false));
    assert!(!repository_entry_is_supported(EntryKind::Symlink, false));
    assert!(!repository_entry_is_supported(EntryKind::Directory, true));
    assert!(!repository_entry_is_supported(EntryKind::Other, false));
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

// Drift from `boot/limine.conf` builds an unbootable image that only `boot-check` would find.
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
            .map(|value| value.trim().rsplit('/').next().unwrap_or("").to_string())
            .unwrap_or_else(|| panic!("no {key} line in boot/limine.conf"))
    };

    assert_eq!(named("path:"), KERNEL_ON_ESP);
    assert_eq!(named("module_path:"), PROGRAM_ON_ESP);
}

// The lists are compared to each other, so a crate in neither passes; Cargo makes a path
// dependency a member by itself, so the host gate would never see it.
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
        // A directory is a crate only when it has a manifest.
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
