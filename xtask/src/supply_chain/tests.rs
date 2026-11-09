use std::fs::File;

use super::*;

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("creating SPDX file symlink");
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("creating SPDX file symlink");
}

fn repository() -> Paths {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits under the repository")
        .to_path_buf();
    Paths::new(root, None)
}

fn records() -> (Inventory, Vec<LockedPackage>) {
    let paths = repository();
    let inventory = parse_inventory(
        &read(
            paths.root().join(INVENTORY),
            MAX_INVENTORY_BYTES,
            "fixture inventory",
        )
        .unwrap(),
    )
    .unwrap();
    let lock = parse_lock(
        &read(
            paths.root().join("Cargo.lock"),
            MAX_LOCK_BYTES,
            "fixture lock file",
        )
        .unwrap(),
    )
    .unwrap();
    (inventory, lock)
}

#[test]
fn oversized_supply_chain_text_is_refused_before_allocation() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-supply-text-bound");
    let path = fixture.join("inventory.txt");
    File::create(&path)
        .unwrap()
        .set_len(MAX_INVENTORY_BYTES + 1)
        .unwrap();

    let error = read(path, MAX_INVENTORY_BYTES, "fixture inventory")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("exceeding its 1048576-byte allocation limit"),
        "{error}"
    );
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn supply_chain_text_growth_is_refused_after_measurement() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-supply-text-growth");
    let path = fixture.join("inventory.txt");
    std::fs::write(&path, b"x").unwrap();
    let growing = path.clone();

    let error = read_with(path, 4, "fixture inventory", move || {
        OpenOptions::new()
            .append(true)
            .open(growing)?
            .write_all(b"yyyy")?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("grew beyond its 4-byte allocation limit"),
        "{error}"
    );
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn supply_chain_text_rejects_a_redirect_before_opening() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-supply-text-link");
    let source = fixture.join("outside");
    let redirected = fixture.join("inventory.txt");
    std::fs::write(&source, b"outside authority").unwrap();
    create_file_symlink(&source, &redirected);
    let opened = std::cell::Cell::new(false);

    assert!(
        read_with(redirected, MAX_INVENTORY_BYTES, "fixture inventory", || {
            opened.set(true);
            Ok(())
        })
        .is_err()
    );
    assert!(!opened.get(), "a redirected file was opened before refusal");
    assert_eq!(std::fs::read(&source).unwrap(), b"outside authority");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn supply_chain_text_revalidates_its_path_after_reading() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-supply-text-revalidation");
    let path = fixture.join("inventory.txt");
    let outside = fixture.join("outside");
    std::fs::write(&path, b"original authority").unwrap();
    std::fs::write(&outside, b"outside authority").unwrap();
    let replacement = path.clone();
    let substituted = std::cell::Cell::new(false);

    let result = read_with(path, MAX_INVENTORY_BYTES, "fixture inventory", || {
        std::fs::remove_file(&replacement)?;
        create_file_symlink(&outside, &replacement);
        substituted.set(true);
        Ok(())
    });

    assert!(
        substituted.get(),
        "the pathname substitution fixture did not run"
    );
    assert!(result.is_err(), "a substituted pathname was not refused");
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside authority");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn every_locked_registry_package_has_one_exact_inventory_row() {
    let (inventory, lock) = records();
    verify_locked_crates(&inventory, &lock).unwrap();
}

#[test]
fn registry_inventory_projection_refuses_a_missing_checksum() {
    let package = LockedPackage {
        name: "fixture".to_string(),
        version: "1.0.0".to_string(),
        registry: true,
        checksum: None,
        dependencies: Vec::new(),
    };

    let error = locked_registry(&[package])
        .expect_err("registry package without a checksum was projected")
        .to_string();
    assert!(
        error.contains("registry package fixture 1.0.0 has no validated checksum"),
        "{error}"
    );
}

#[test]
fn runtime_scope_is_the_kernel_dependency_closure() {
    let paths = repository();
    let (inventory, lock) = records();
    verify_runtime_scope(paths.root(), &inventory, &lock).unwrap();
}

#[test]
fn every_license_conclusion_is_from_the_reviewed_policy() {
    let paths = repository();
    parse_inventory(
        &read(
            paths.root().join(INVENTORY),
            MAX_INVENTORY_BYTES,
            "fixture inventory",
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn every_vendored_digest_is_in_the_inventory_and_matches_the_bytes() {
    let paths = repository();
    let inventory = parse_inventory(
        &read(
            paths.root().join(INVENTORY),
            MAX_INVENTORY_BYTES,
            "fixture inventory",
        )
        .unwrap(),
    )
    .unwrap();
    verify_artifacts(&paths, &inventory).unwrap();
}

#[test]
fn every_runtime_notice_and_required_license_is_present() {
    let paths = repository();
    let inventory = parse_inventory(
        &read(
            paths.root().join(INVENTORY),
            MAX_INVENTORY_BYTES,
            "fixture inventory",
        )
        .unwrap(),
    )
    .unwrap();
    verify_notices(&paths, &inventory).unwrap();
}

#[test]
fn the_committed_spdx_is_the_deterministic_inventory_rendering() {
    let paths = repository();
    let source = read(
        paths.root().join(INVENTORY),
        MAX_INVENTORY_BYTES,
        "fixture inventory",
    )
    .unwrap();
    let inventory = parse_inventory(&source).unwrap();
    assert_eq!(
        read(
            paths.root().join(SPDX),
            MAX_SPDX_BYTES,
            "fixture SPDX document",
        )
        .unwrap(),
        render_spdx(&inventory, &source).unwrap()
    );
}

#[test]
fn spdx_publication_never_writes_through_an_external_link() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-spdx-links");
    let repository = fixture.join("repository");
    let directory = repository.join("supply-chain");
    let outside = fixture.join("outside");
    let sentinel = outside.join("sentinel");
    let destination = repository.join(SPDX);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(&sentinel, b"outside authority").unwrap();

    std::fs::hard_link(&sentinel, &destination).unwrap();
    let publication = publish_spdx(&repository, &destination, b"new SPDX bytes\n");
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    publication.unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"new SPDX bytes\n");

    std::fs::remove_file(&destination).unwrap();
    create_file_symlink(&sentinel, &destination);
    assert!(publish_spdx(&repository, &destination, b"forged\n").is_err());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"outside authority");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_failed_spdx_stage_never_exposes_partial_authoritative_bytes() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-spdx-stage");
    let repository = fixture.join("repository");
    let directory = repository.join("supply-chain");
    let destination = repository.join(SPDX);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&destination, b"old complete SPDX\n").unwrap();
    std::fs::create_dir(directory.join(SPDX_STAGING)).unwrap();

    assert!(publish_spdx(&repository, &destination, b"partial").is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"old complete SPDX\n");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_late_spdx_publication_refusal_restores_the_previous_document() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-spdx-rollback");
    let repository = fixture.join("repository");
    let directory = repository.join("supply-chain");
    let destination = repository.join(SPDX);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&destination, b"old complete SPDX\n").unwrap();

    let error = publish_spdx_with(&repository, &destination, b"new complete SPDX\n", |_| {
        bail!("injected late SPDX refusal")
    })
    .unwrap_err();

    assert!(format!("{error:#}").contains("injected late SPDX refusal"));
    assert_eq!(std::fs::read(&destination).unwrap(), b"old complete SPDX\n");
    assert_eq!(
        crate::paths::directory_entries(&directory).unwrap().len(),
        1
    );
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn oversized_spdx_renderings_are_refused_before_staging() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-spdx-output-bound");
    let repository = fixture.join("repository");
    let directory = repository.join("supply-chain");
    let destination = repository.join(SPDX);
    std::fs::create_dir_all(&directory).unwrap();
    let oversized = vec![b'x'; MAX_SPDX_BYTES as usize + 1];

    let error = publish_spdx(&repository, &destination, &oversized)
        .unwrap_err()
        .to_string();
    assert!(error.contains("exceeding its 4194304-byte publication limit"));
    assert!(
        crate::paths::directory_entries(&directory)
            .unwrap()
            .is_empty()
    );
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_locked_package_from_any_other_source_is_refused() {
    let lock = |source: &str| {
        format!(
            "version = 4\n\n[[package]]\nname = \"fixture\"\nversion = \"1.0.0\"\nsource = \"{source}\"\nchecksum = \"{}\"\n",
            "a".repeat(64)
        )
    };
    assert!(parse_lock(&lock(CRATES_IO)).unwrap()[0].registry);
    for source in [
        "registry+https://example.invalid/index",
        "sparse+https://example.invalid/index/",
        "git+https://example.invalid/fixture?rev=1#1",
    ] {
        let error = parse_lock(&lock(source)).unwrap_err().to_string();
        assert!(error.contains("is not crates.io"), "{source}: {error}");
    }
}

#[test]
fn every_dependency_declaration_shape_is_modeled_or_refused() {
    let modeled = "[package]\nname = \"fixture\"\n\n[dependencies]\nplain = \"1\"\n\
            shared.workspace = true\ninline = { workspace = true, features = [\n    \"x\",\n] }\n\n\
            [target.'cfg(unix)'.dependencies]\nunix = \"1\"\n\n[dev-dependencies.tested]\n\
            version = \"1\"\n\n[build-dependencies]\nbuilt = \"1\"\n";
    let names = manifest_dependencies(modeled)
        .unwrap()
        .into_iter()
        .map(|dependency| dependency.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["plain", "shared", "inline", "unix"]);

    for (manifest, diagnostic) in [
        (
            "[dependencies.detailed]\nversion = \"1\"\n",
            "`[dependencies.detailed]` is a dependency table",
        ),
        (
            "[target.'cfg(unix)'.dependencies.detailed]\nversion = \"1\"\n",
            "is a dependency table the runtime-scope reader does not model",
        ),
        (
            "[dependencies]\ndotted.version = \"1\"\n",
            "dotted dependency key `dotted.version`",
        ),
        (
            "[dependencies]\nalias = { package = \"real\", version = \"1\" }\n",
            "renamed dependency `alias`",
        ),
        (
            "[dependencies]\nalias = { version = \"1\", package=\"real\" }\n",
            "renamed dependency `alias`",
        ),
        (
            "[dependencies] # c\nhidden = \"1\"\n",
            "`[dependencies] # c` is a table header spelling",
        ),
        (
            "[ dependencies ]\nhidden = \"1\"\n",
            "`[ dependencies ]` is a table header spelling",
        ),
        (
            "[\"dependencies\"]\nhidden = \"1\"\n",
            "`[\"dependencies\"]` is a table header spelling",
        ),
        (
            "[dependencies . x]\nversion = \"1\"\n",
            "`[dependencies . x]` is a table header spelling",
        ),
        (
            "[target.'cfg(unix)'.dependencies] # c\nhidden = \"1\"\n",
            "`[target.'cfg(unix)'.dependencies] # c` is a table header spelling",
        ),
    ] {
        let Err(error) = manifest_dependencies(manifest) else {
            panic!("{manifest:?} was read rather than refused");
        };
        let error = error.to_string();
        assert!(error.contains(diagnostic), "{manifest}: {error}");
    }
}

#[test]
fn runtime_dependency_tables_cannot_hide_in_outer_assignments() {
    for manifest in [
        "dependencies.hidden = \"1\"\n",
        "dependencies = { hidden = \"1\" }\n",
        "[target.'cfg(unix)']\ndependencies = { hidden = \"1\" }\n",
        "[target.'cfg(unix)']\ndependencies.hidden = \"1\"\n",
        "[target]\n'cfg(unix)'.dependencies.hidden = \"1\"\n",
        "\"dependencies\" = { hidden = \"1\" }\n",
        "\"depen\\u0064encies\" = { hidden = \"1\" }\n",
        "target = { 'cfg(unix)' = { dependencies = { hidden = \"1\" } } }\n",
        "workspace = { dependencies = { hidden = \"1\" } }\n",
        "[package]\nname = \"fixture\"\n[target.'cfg(unix)']\n\
             dependencies = { hidden = \"1\" }\n[dependencies]\nvisible = \"1\"\n",
    ] {
        let error = manifest_dependencies(manifest)
            .expect_err("a hidden runtime dependency must refuse the manifest")
            .to_string();
        assert!(error.contains("key assignment"), "{manifest}: {error}");
    }
    let modeled = "[package]\nname = \"fixture\"\nversion.workspace = true\n\
            # dependencies = { decoy = \"1\" }\n\
            [dev-dependencies]\nfixture = { version = \"1\", features = [\"test\"] }\n\
            [dependencies]\nlive.workspace = true\n";
    let dependencies = manifest_dependencies(modeled).expect("modeled member manifest");
    assert_eq!(dependencies.len(), 1);
    assert_eq!(dependencies[0].name, "live");
}

#[test]
fn a_locked_path_package_must_be_a_distinct_workspace_member() {
    let lock = |paths: &[&str]| {
        let mut text = format!(
            "version = 4\n\n[[package]]\nname = \"bitflags\"\nversion = \"2.13.1\"\nsource = \"{CRATES_IO}\"\nchecksum = \"{}\"\n",
            "a".repeat(64)
        );
        for name in paths {
            text.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"
            ));
        }
        parse_lock(&text).unwrap()
    };
    let members = ["wizard-kernel", "wost"]
        .map(|name| (name.to_string(), Vec::new()))
        .into_iter()
        .collect();
    verify_path_packages(&lock(&["wizard-kernel", "wost"]), &members).unwrap();
    for (label, paths) in [
        ("outside the workspace", ["wizard-kernel", "helper"]),
        ("under a member's name", ["wost", "wost"]),
    ] {
        let Err(error) = verify_path_packages(&lock(&paths), &members) else {
            panic!("a path package {label} was admitted");
        };
        let error = error.to_string();
        assert!(
            error.contains("is not a distinct workspace member"),
            "{label}: {error}"
        );
    }
}

#[test]
fn each_runtime_component_has_exactly_one_exact_notice_entry() {
    let (inventory, _) = records();
    let preamble = "See `foundation.spdx`. SIL Open Font License 1.1 (`OFL-1.1`); BSD-2-Clause.\n";
    let entries = [
        "- `limine` `0.5.0`, the crate: MIT.\n",
        "- `bitflags` `2.13.1`: MIT.\n",
        "- `noto-sans-mono-bitmap` `0.3.2`: MIT and OFL-1.1.\n",
        "- `boot/limine/BOOTX64.EFI` `v10.x-binary-2026-07-18`: BSD-2-Clause.\n",
    ];
    let notices = |entries: &[&str]| format!("{preamble}{}", entries.concat());
    require_notices(&notices(&entries), &inventory).unwrap();

    let decoy = notices(&[
        entries[0],
        "- bitflags 2.13.1 appears here only as prose.\n",
        entries[2],
        entries[3],
    ]);
    let wrong_version = notices(&[
        entries[0],
        "- `bitflags` `2.13.0`: MIT.\n",
        entries[2],
        entries[3],
    ]);
    let extra = notices(&[
        entries[0],
        entries[1],
        entries[2],
        entries[3],
        "- `syn` `2.0.119`: development only.\n",
    ]);
    for (label, text) in [
        ("decoy", decoy),
        ("wrong version", wrong_version),
        ("extra", extra),
    ] {
        let Err(error) = require_notices(&text, &inventory) else {
            panic!("the {label} notices were accepted");
        };
        let error = error.to_string();
        assert!(
            error.contains("differ from the runtime inventory"),
            "{label}: {error}"
        );
    }
    let repeated = notices(&[entries[0], entries[1], entries[1], entries[2], entries[3]]);
    let error = require_notices(&repeated, &inventory)
        .unwrap_err()
        .to_string();
    assert!(error.contains("repeat `bitflags` `2.13.1`"), "{error}");
}

#[test]
fn a_dependency_with_two_locked_versions_must_name_which_one() {
    let (_, lock) = records();
    assert!(
        resolve_dependency(
            &lock,
            &Dependency {
                name: "bitflags".to_string(),
                version: None
            }
        )
        .is_err()
    );
}
