use super::*;

#[test]
fn semantic_artifacts_are_bounded_before_and_during_allocation() {
    let root = crate::test_support::reserve_temp_directory("wizard-bounded-artifact");
    let path = root.join("artifact");
    let file = File::create(&path).unwrap();
    file.set_len(33).unwrap();
    drop(file);
    let error = read_bounded_file(&path, 32, "fixture artifact")
        .unwrap_err()
        .to_string();
    assert!(error.contains("exceeding the 32-byte allocation limit"));

    std::fs::write(&path, b"x").unwrap();
    let changed = path.clone();
    let error = read_bounded_file_with(
        &path,
        32,
        "fixture artifact",
        || Ok(()),
        move || {
            OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), changed)?
                .set_len(33)?;
            Ok(())
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("grew beyond the 32-byte allocation limit"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_qualification_digests_have_a_streaming_ceiling() {
    let root = crate::test_support::reserve_temp_directory("wizard-bounded-digest");
    let path = root.join("artifact");
    File::create(&path).unwrap().set_len(33).unwrap();

    let error = digest_file_bounded(&path, 32).unwrap_err().to_string();

    assert!(
        error.contains("exceeding its 32-byte digest limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_qualification_digest_growth_is_refused_after_measurement() {
    let root = crate::test_support::reserve_temp_directory("wizard-growing-digest");
    let path = root.join("artifact");
    std::fs::write(&path, b"x").unwrap();
    let growing = path.clone();

    let error = digest_file_bounded_with(&path, 32, move || {
        OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), growing)?
            .set_len(33)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("grew beyond its 32-byte digest limit while hashing"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_qualification_digest_truncation_is_refused_after_measurement() {
    let root = crate::test_support::reserve_temp_directory("wizard-truncated-digest");
    let path = root.join("artifact");
    std::fs::write(&path, b"1234").unwrap();
    let shrinking = path.clone();

    let error = digest_file_bounded_with(&path, 4, move || {
        OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), shrinking)?
            .set_len(3)?;
        Ok(())
    })
    .unwrap_err()
    .to_string();

    assert!(error.contains("changed length while hashing"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_oversized_qualification_record_is_refused_before_parsing() {
    let root = crate::test_support::reserve_temp_directory("wizard-oversized-qualification-record");
    let path = root.join("record");
    File::create(&path)
        .unwrap()
        .set_len(MAX_RECORD_BYTES + 1)
        .unwrap();

    let error = read_record(&path, Kind::Mutation).unwrap_err().to_string();

    assert!(
        error.contains("exceeding the 262144-byte allocation limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_oversized_generation_receipt_is_refused_before_parsing() {
    let root = crate::test_support::reserve_temp_directory("wizard-oversized-generation-record");
    let path = root.join(VMWARE_GENERATION_RECEIPT);
    File::create(&path)
        .unwrap()
        .set_len(MAX_VMWARE_TEXT_BYTES + 1)
        .unwrap();

    let error = read_generation_record(&path, Kind::VmwareSelfTest)
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("exceeding the 1048576-byte allocation limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tracked_model_files_are_size_checked_before_streaming() {
    let root = crate::test_support::reserve_temp_directory("wizard-model-input-bound");
    let models = root.join("models");
    std::fs::create_dir_all(&models).unwrap();
    run_git(&root, &["init", "--quiet"]);
    let oversized = models.join("oversized.tla");
    std::fs::write(&oversized, b"tracked").unwrap();
    run_git(&root, &["add", "models/oversized.tla"]);
    OpenOptions::open(OpenOptions::write(&mut File::options(), true), &oversized)
        .unwrap()
        .set_len(MAX_MODEL_FILE_BYTES + 1)
        .unwrap();

    let error = tracked_tree_digest(&root, "models")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("exceeding its 1048576-byte limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn tracked_model_tree_has_an_independent_total_size_bound() {
    let root = crate::test_support::reserve_temp_directory("wizard-model-tree-bound");
    let models = root.join("models");
    std::fs::create_dir_all(&models).unwrap();
    run_git(&root, &["init", "--quiet"]);
    for index in 0..5 {
        let path = models.join(format!("input-{index}.tla"));
        std::fs::write(&path, b"tracked").unwrap();
    }
    run_git(&root, &["add", "models"]);
    for index in 0..5 {
        OpenOptions::open(
            OpenOptions::write(&mut File::options(), true),
            models.join(format!("input-{index}.tla")),
        )
        .unwrap()
        .set_len(MAX_MODEL_FILE_BYTES)
        .unwrap();
    }

    let error = tracked_tree_digest(&root, "models")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("exceeding its 4194304-byte limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn streamed_model_tree_digest_preserves_the_length_delimited_schema() {
    let root = crate::test_support::reserve_temp_directory("wizard-model-digest");
    std::fs::create_dir_all(root.join("models/nested")).unwrap();
    run_git(&root, &["init", "--quiet"]);
    for (path, contents) in [
        ("models/a.tla", b"short".as_slice()),
        ("models/nested/longer.cfg", b"different bytes".as_slice()),
    ] {
        std::fs::write(root.join(path), contents).unwrap();
    }
    run_git(&root, &["add", "models"]);

    let observed = tracked_tree_digest(&root, "models").unwrap();
    let listing = git_output_bytes(
        &root,
        &["ls-files", "-z", "--", "models"],
        MAX_MODEL_PATH_LIST_BYTES,
        "model digest compatibility fixture",
    )
    .unwrap();
    let paths: Vec<&str> = listing
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| std::str::from_utf8(path).unwrap())
        .collect();
    let mut legacy = Vec::new();
    let mut bytes = 0u64;
    for relative in &paths {
        let contents = std::fs::read(root.join(relative)).unwrap();
        bytes += contents.len() as u64;
        legacy.extend_from_slice(&(relative.len() as u64).to_le_bytes());
        legacy.extend_from_slice(relative.as_bytes());
        legacy.extend_from_slice(&(contents.len() as u64).to_le_bytes());
        legacy.extend_from_slice(&contents);
    }

    assert_eq!(observed.files, paths.len());
    assert_eq!(observed.bytes, bytes);
    assert_eq!(observed.sha256, sha256::hex(&sha256::digest(&legacy)));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deterministic_machine_images_have_exact_sizes_before_verification() {
    let root = crate::test_support::reserve_temp_directory("wizard-exact-artifact");
    let image = root.join(VMWARE_IMAGE);
    File::create(&image)
        .unwrap()
        .set_len(boot_image_bytes() - 1)
        .unwrap();
    assert!(require_artifact_size(Kind::VmwareShipped, VMWARE_IMAGE, &image).is_err());
    OpenOptions::open(OpenOptions::write(&mut File::options(), true), &image)
        .unwrap()
        .set_len(boot_image_bytes())
        .unwrap();
    require_artifact_size(Kind::VmwareShipped, VMWARE_IMAGE, &image).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_qualification_size_policy_has_an_independent_sparse_caller_witness() {
    struct ExpectedPolicy {
        prefix: String,
        relative: String,
        artifact: Option<(Kind, &'static str)>,
        maximum: u64,
        exact: bool,
    }

    let record = |slug: &str| ExpectedPolicy {
        prefix: format!("qualification.{slug}.record"),
        relative: format!("qualification/{slug}/record"),
        artifact: None,
        maximum: 262_144,
        exact: false,
    };
    let artifact =
        |slug: &str, kind: Kind, key: &str, name: &'static str, maximum: u64, exact: bool| {
            ExpectedPolicy {
                prefix: format!("qualification.{slug}.{key}"),
                relative: format!("qualification/{slug}/{name}"),
                artifact: Some((kind, name)),
                maximum,
                exact,
            }
        };

    let mut expected = vec![
        record("cargo-mutants-grade"),
        artifact(
            "cargo-mutants-grade",
            Kind::CargoMutantsGrade,
            "request",
            "request",
            1_048_576,
            false,
        ),
        artifact(
            "cargo-mutants-grade",
            Kind::CargoMutantsGrade,
            "inventory",
            "inventory.json",
            67_108_864,
            false,
        ),
        artifact(
            "cargo-mutants-grade",
            Kind::CargoMutantsGrade,
            "mutants",
            "mutants.json",
            67_108_864,
            false,
        ),
        artifact(
            "cargo-mutants-grade",
            Kind::CargoMutantsGrade,
            "outcomes",
            "outcomes.json",
            67_108_864,
            false,
        ),
        artifact(
            "cargo-mutants-grade",
            Kind::CargoMutantsGrade,
            "adjudications",
            "adjudications.json",
            16_777_216,
            false,
        ),
        artifact(
            "cargo-mutants-grade",
            Kind::CargoMutantsGrade,
            "doctest",
            "doctest.transcript",
            1_048_576,
            false,
        ),
        record("codex-security-deep"),
        artifact(
            "codex-security-deep",
            Kind::CodexSecurityDeep,
            "request",
            "request",
            262_144,
            false,
        ),
        artifact(
            "codex-security-deep",
            Kind::CodexSecurityDeep,
            "manifest",
            "scan-manifest.json",
            4_194_304,
            false,
        ),
        artifact(
            "codex-security-deep",
            Kind::CodexSecurityDeep,
            "findings",
            "findings.json",
            33_554_432,
            false,
        ),
        artifact(
            "codex-security-deep",
            Kind::CodexSecurityDeep,
            "coverage",
            "coverage.json",
            4_194_304,
            false,
        ),
        artifact(
            "codex-security-deep",
            Kind::CodexSecurityDeep,
            "report",
            "report.md",
            33_554_432,
            false,
        ),
        artifact(
            "codex-security-deep",
            Kind::CodexSecurityDeep,
            "worker-transitions",
            "worker-transitions",
            1_048_576,
            false,
        ),
        record("advisory"),
        artifact(
            "advisory",
            Kind::Advisory,
            "policy",
            ".cargo/audit.toml",
            4_096,
            false,
        ),
        artifact(
            "advisory",
            Kind::Advisory,
            "transcript",
            "cargo-audit.transcript",
            16_781_312,
            false,
        ),
        artifact(
            "advisory",
            Kind::Advisory,
            "refresh-transcript",
            "cargo-audit-refresh.transcript",
            16_781_312,
            false,
        ),
        record("mutation"),
        artifact(
            "mutation",
            Kind::Mutation,
            "report",
            "report",
            16_777_216,
            false,
        ),
        record("model"),
    ];
    for (key, name) in [
        ("ring-clean", "ring-clean.transcript"),
        ("ring-memory-clean", "ring-memory-clean.transcript"),
        (
            "ring-memory-store-relaxed",
            "ring-memory-store-relaxed.transcript",
        ),
        (
            "ring-memory-load-relaxed",
            "ring-memory-load-relaxed.transcript",
        ),
        (
            "ring-memory-no-reservation",
            "ring-memory-no-reservation.transcript",
        ),
        (
            "ring-memory-duplicate-terminal",
            "ring-memory-duplicate-terminal.transcript",
        ),
    ] {
        expected.push(artifact("model", Kind::Model, key, name, 16_781_312, false));
    }
    for (slug, kind) in [
        ("vmware-self-test", Kind::VmwareSelfTest),
        ("vmware-shipped", Kind::VmwareShipped),
    ] {
        expected.push(record(slug));
        for (key, name, maximum, exact) in [
            ("generation", "generation.receipt", 1_048_576, false),
            ("image", "bedrock-flat.img", 67_108_864, true),
            ("namespace", "wizard-nvm-flat.img", 68_157_440, true),
            ("vmdk", "bedrock.vmdk", 1_048_576, false),
            ("namespace-vmdk", "wizard-nvm.vmdk", 1_048_576, false),
            ("vmx", "bedrock.vmx", 1_048_576, false),
            ("handoff", "dma-handoff.bin", 48, true),
            ("transcript", "serial.log", 16_777_216, false),
            ("observer", "vmrun-list.txt", 1_048_576, false),
        ] {
            expected.push(artifact(slug, kind, key, name, maximum, exact));
        }
    }

    assert_eq!(BUNDLE_FILES.len(), expected.len());
    let root = crate::test_support::reserve_temp_directory("wizard-size-policy");
    let path = root.join("sparse-artifact");
    File::create(&path).unwrap();

    for (spec, expected) in BUNDLE_FILES.iter().zip(&expected) {
        assert_eq!(spec.prefix, expected.prefix);
        assert_eq!(spec.relative, expected.relative);
        assert_eq!(
            bundle_file_size_policy(spec).unwrap(),
            (expected.maximum, expected.exact),
            "{} has the wrong bundle policy",
            expected.relative
        );
        if let Some((kind, name)) = expected.artifact {
            assert_eq!(
                artifact_size_policy(kind, name).unwrap(),
                (expected.maximum, expected.exact),
                "{} has the wrong artifact policy",
                expected.relative
            );
        }

        let refused = if expected.exact {
            expected.maximum - 1
        } else {
            expected.maximum + 1
        };
        OpenOptions::open(OpenOptions::write(&mut File::options(), true), &path)
            .unwrap()
            .set_len(refused)
            .unwrap();
        assert!(
            require_bundle_file_size(spec, &path).is_err(),
            "{} admitted a sparse file outside its bundle policy",
            expected.relative
        );
        if let Some((kind, name)) = expected.artifact {
            assert!(
                require_artifact_size(kind, name, &path).is_err(),
                "{} admitted a sparse file outside its artifact policy",
                expected.relative
            );
        }

        OpenOptions::open(OpenOptions::write(&mut File::options(), true), &path)
            .unwrap()
            .set_len(expected.maximum)
            .unwrap();
        require_bundle_file_size(spec, &path).unwrap();
        if let Some((kind, name)) = expected.artifact {
            require_artifact_size(kind, name, &path).unwrap();
        }
    }

    std::fs::remove_dir_all(root).unwrap();
}
