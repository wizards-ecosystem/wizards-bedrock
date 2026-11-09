use super::*;
use crate::test_support::reserve_temp_directory;

#[test]
#[ignore]
fn parked_qemu_version_fixture() {
    loop {
        std::thread::park();
    }
}

#[test]
fn a_parked_qemu_identity_probe_reaches_its_named_deadline() {
    let executable = std::env::current_exe().unwrap();
    let root = std::env::current_dir().unwrap();
    let error = qemu_version_output_with(&root, &executable, |command, deadline, label| {
        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [std::ffi::OsStr::new("--version")]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
        assert_eq!(deadline, QEMU_VERSION_DEADLINE);
        assert_eq!(label, QEMU_VERSION_LABEL);

        let mut parked = Command::new(&executable);
        parked.args([
            "--exact",
            "evidence::tests::parked_qemu_version_fixture",
            "--ignored",
        ]);
        bounded_process::output(parked, Duration::from_millis(50), label)
    })
    .unwrap_err();

    assert!(format!("{error:#}").contains("QEMU version identity probe timed out after 50 ms"));
}

#[test]
fn a_dirty_source_cannot_be_called_clean() {
    assert!(status_is_clean("\r\n  "));
    assert!(!status_is_clean(" M kernel/src/main.rs\n"));
    assert!(!status_is_clean("?? untracked.rs\n"));
}

#[test]
fn release_source_capture_requires_two_clean_views_of_one_identity() {
    let current = ("a".repeat(40), "b".repeat(40));
    require_stable_clean_source("", &current, "", &current).unwrap();

    let changed = ("9".repeat(40), current.1.clone());
    assert!(require_stable_clean_source("", &current, "", &changed).is_err());
    assert!(require_stable_clean_source(" M Cargo.lock\n", &current, "", &current).is_err());
    assert!(require_stable_clean_source("", &current, "?? transient-input\n", &current).is_err());
}

#[test]
fn release_reproduction_uses_copied_sources_and_rejects_host_paths() {
    let implementation = include_str!("../evidence.rs");
    let create = implementation.find("pub(crate) fn create(").unwrap();
    let verify = implementation[create..]
        .find("pub(crate) fn verify(")
        .unwrap()
        + create;
    let body = &implementation[create..verify];
    assert_eq!(body.matches("reproduce_release(paths,").count(), 1);
    assert_eq!(
        body.matches("reproduce_release_alternate(paths,").count(),
        1
    );
    assert!(body.contains(".zip(&release_b.host_roots)"));
    assert!(body.contains(".all(|(left, right)| left != right)"));
    assert!(body.contains("require_no_host_paths(path, maximum, label, &host_roots)?"));

    let reproduce = implementation.find("fn reproduce_release(").unwrap();
    let absent = implementation[reproduce..]
        .find("fn require_no_host_paths(")
        .unwrap()
        + reproduce;
    assert!(
        implementation[reproduce..absent]
            .contains("mutate::workspace::with_clean_source_copy(paths, build)")
    );
    assert_eq!(
        implementation[reproduce..absent]
            .matches("mutate::workspace::with_clean_source_copy(paths")
            .count(),
        2
    );
    assert!(implementation[reproduce..absent].contains("CARGO_NET_OFFLINE"));
    assert!(implementation[reproduce..absent].contains("std::fs::hard_link"));
    assert!(!implementation[reproduce..absent].contains("credentials.toml"));

    let root = reserve_temp_directory("wizard-host-path-evidence");
    let artifact = root.join("artifact");
    let host = PathBuf::from(r"C:\private host");
    std::fs::write(&artifact, b"prefix C:\\private host suffix").unwrap();
    assert!(
        require_no_host_paths(&artifact, 1024, "fixture", std::slice::from_ref(&host)).is_err()
    );
    std::fs::write(&artifact, b"prefix /wizard/source suffix").unwrap();
    require_no_host_paths(&artifact, 1024, "fixture", &[host]).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn parent_docs_relation_accepts_only_exact_regular_file_modifications() {
    let old = "1".repeat(40);
    let new = "2".repeat(40);
    let raw = format!(":100644 100644 {old} {new} M\0docs/016-pre-wizzy-foundation.md\0");
    let qualified = qualification::SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    assert_eq!(
        QualificationRelation::from_parent_diff(&qualified, raw.as_bytes())
            .unwrap()
            .files,
        ["docs/016-pre-wizzy-foundation.md"]
    );
    let changed_blob = format!(
        ":100644 100644 {old} {} M\0docs/016-pre-wizzy-foundation.md\0",
        "3".repeat(40)
    );
    assert_ne!(
        QualificationRelation::from_parent_diff(&qualified, raw.as_bytes())
            .unwrap()
            .digest,
        QualificationRelation::from_parent_diff(&qualified, changed_blob.as_bytes())
            .unwrap()
            .digest,
    );

    let added = format!(":000000 100644 {} {new} A\0README.md\0", "0".repeat(40));
    assert!(parse_parent_docs_diff(added.as_bytes()).is_err());
    let source = format!(":100644 100644 {old} {new} M\0xtask/src/evidence.rs\0");
    assert!(parse_parent_docs_diff(source.as_bytes()).is_err());
    let empty = b"";
    assert!(parse_parent_docs_diff(empty).is_err());
}

#[test]
fn the_parent_diff_admits_only_sorted_unique_regular_modifications() {
    let old = "1".repeat(40);
    let new = "2".repeat(40);
    let row =
        |modes: &str, status: &str, path: &str| format!(":{modes} {old} {new} {status}\0{path}\0");
    let refused = |raw: &str, diagnostic: &str| {
        let error = parse_parent_docs_diff(raw.as_bytes()).unwrap_err();
        assert!(format!("{error:#}").contains(diagnostic), "{error:#}");
    };
    let readme = row("100644 100644", "M", "README.md");
    let log = row("100644 100644", "M", "docs/004-decision-log.md");
    assert_eq!(
        parse_parent_docs_diff(format!("{readme}{log}").as_bytes()).unwrap(),
        ["README.md", "docs/004-decision-log.md"]
    );
    refused(&format!("{log}{readme}"), "not unique and sorted");
    refused(&format!("{readme}{readme}"), "not unique and sorted");
    refused(
        &row("100644 100644", "T", "README.md"),
        "ordinary 100644 file modifications",
    );
    refused(
        &row("100755 100755", "M", "README.md"),
        "ordinary 100644 file modifications",
    );
    refused(readme.trim_end_matches('\0'), "not NUL terminated");
    refused(&format!("{readme}extra\0"), "incomplete row");
}

#[test]
fn same_commit_relation_requires_the_live_current_mutation_pass() {
    assert!(relation_requires_current_mutation("same-commit"));
    assert!(!relation_requires_current_mutation("parent-docs-only-v1"));
}

#[test]
fn parent_docs_relation_requires_one_exact_direct_parent() {
    let evidence = "1".repeat(40);
    let qualified = "2".repeat(40);
    require_single_direct_parent(&format!("{evidence} {qualified}\n"), &evidence, &qualified)
        .unwrap();
    assert!(
        require_single_direct_parent(
            &format!("{evidence} {}\n", "3".repeat(40)),
            &evidence,
            &qualified
        )
        .is_err()
    );
    assert!(
        require_single_direct_parent(
            &format!("{evidence} {qualified} {}\n", "3".repeat(40)),
            &evidence,
            &qualified
        )
        .is_err()
    );
}

#[test]
fn a_duplicate_evidence_key_is_rejected_before_canonical_rendering() {
    let mut record = Record::default();
    record.insert("source.commit", "one").unwrap();

    let error = record.insert("source.commit", "two").unwrap_err();
    assert_eq!(error.to_string(), "duplicate evidence key `source.commit`");
}

#[test]
fn an_oversized_manifest_is_refused_before_payload_allocation() {
    let root = reserve_temp_directory("wizard-oversized-manifest");
    let paths = Paths::new(root.join("repository"), Some(&root.join("target")));
    let evidence = evidence_root(&paths);
    std::fs::create_dir_all(&evidence).unwrap();
    File::create(evidence.join(MANIFEST_NAME))
        .unwrap()
        .set_len(MAX_MANIFEST_BYTES + 1)
        .unwrap();

    let error = verify(&paths).unwrap_err().to_string();
    assert!(error.contains("exceeding the 1048576-byte allocation limit"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_evidence_digests_have_a_streaming_ceiling() {
    let root = reserve_temp_directory("wizard-evidence-digest-bound");
    let path = root.join("artifact");
    File::create(&path).unwrap().set_len(33).unwrap();

    let error = digest_file_bounded(&path, 32, "fixture artifact")
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("exceeding its 32-byte hashing limit"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_evidence_digest_growth_is_refused_after_measurement() {
    let root = reserve_temp_directory("wizard-evidence-digest-growth");
    let path = root.join("artifact");
    std::fs::write(&path, b"x").unwrap();
    let growing = path.clone();

    let error = digest_file_bounded_with(
        &path,
        4,
        "fixture artifact",
        move || {
            let mut options = OpenOptions::new();
            OpenOptions::write(&mut options, true);
            OpenOptions::open(&options, growing)?.set_len(5)?;
            Ok(())
        },
        || Ok(()),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("grew beyond its 4-byte hashing limit while it was hashed"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_evidence_digest_truncation_is_refused_during_hashing() {
    let root = reserve_temp_directory("wizard-evidence-digest-truncation");
    let path = root.join("artifact");
    std::fs::write(&path, b"1234").unwrap();
    let shrinking = path.clone();

    let error = digest_file_bounded_with(
        &path,
        4,
        "fixture artifact",
        move || {
            let mut options = OpenOptions::new();
            OpenOptions::write(&mut options, true);
            OpenOptions::open(&options, shrinking)?.set_len(3)?;
            Ok(())
        },
        || Ok(()),
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("changed length while it was hashed"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn generic_evidence_digest_remeasures_after_hashing() {
    let root = reserve_temp_directory("wizard-evidence-digest-remeasure");
    let path = root.join("artifact");
    std::fs::write(&path, b"1234").unwrap();
    let shrinking = path.clone();

    let error = digest_file_bounded_with(
        &path,
        4,
        "fixture artifact",
        || Ok(()),
        move || {
            let mut options = OpenOptions::new();
            OpenOptions::write(&mut options, true);
            OpenOptions::open(&options, shrinking)?.set_len(3)?;
            Ok(())
        },
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("changed length after it was hashed"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deterministic_artifact_size_is_checked_before_hashing_payload_bytes() {
    let root = reserve_temp_directory("wizard-exact-evidence-artifact");
    let path = root.join("oversized.img");
    File::create(&path).unwrap().set_len(33).unwrap();
    let reached = std::cell::Cell::new(false);

    let error = digest_file_exact_with(&path, 32, "fixture image", |_file| {
        reached.set(true);
        Ok(Digest {
            bytes: 33,
            sha256: "0".repeat(64),
        })
    })
    .unwrap_err()
    .to_string();

    assert!(error.contains("33 bytes, expected exactly 32"), "{error}");
    assert!(!reached.get(), "oversized image bytes reached the hasher");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deterministic_artifact_digest_requires_the_exact_returned_byte_count() {
    let root = reserve_temp_directory("wizard-exact-evidence-count");
    let path = root.join("image");
    File::create(&path).unwrap().set_len(32).unwrap();

    let error = digest_file_exact_with(&path, 32, "fixture image", |_file| {
        Ok(Digest {
            bytes: 33,
            sha256: "0".repeat(64),
        })
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("fixture image hashed 33 bytes, expected exactly 32"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn deterministic_artifact_digest_remeasures_the_exact_extent() {
    let root = reserve_temp_directory("wizard-exact-evidence-remeasure");
    let path = root.join("image");
    File::create(&path).unwrap().set_len(32).unwrap();
    let growing = path.clone();

    let error = digest_file_exact_with(&path, 32, "fixture image", move |file| {
        let (bytes, digest) = sha256::digest_reader(file)?;
        let mut options = OpenOptions::new();
        OpenOptions::write(&mut options, true);
        OpenOptions::open(&options, growing)?.set_len(33)?;
        Ok(Digest {
            bytes,
            sha256: sha256::hex(&digest),
        })
    })
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("changed outside its exact 32-byte extent while being hashed"),
        "{error}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_manifest_schema_is_exact() {
    let mut record = Record::default();
    record.insert("known", "value").unwrap();
    let expected = BTreeSet::from(["known".to_string(), "required".to_string()]);
    assert!(require_exact_keys(&record, &expected).is_err());

    record.insert("required", "value").unwrap();
    record.insert("unknown", "value").unwrap();
    assert!(require_exact_keys(&record, &expected).is_err());
}

#[test]
fn equal_lengths_do_not_make_artifacts_identical() {
    let first = Digest {
        bytes: 4096,
        sha256: "a".repeat(64),
    };
    let second = Digest {
        bytes: 4096,
        sha256: "b".repeat(64),
    };
    assert!(!byte_identical(&first, &second));
    assert!(byte_identical(&first, &first));
}

#[test]
fn every_recorded_result_must_pass() {
    let mut record = Record::default();
    for step in gate::REGISTRY.steps() {
        record.insert(step.evidence_key(), PASS).unwrap();
    }
    record
        .insert("result.artifacts-byte-identical", PASS)
        .unwrap();
    record
        .insert("result.shipped-artifact-boot", "fail")
        .unwrap();
    assert!(require_pass_results(&record).is_err());
}

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
fn evidence_v9_is_rejected_even_when_its_fields_are_canonical() {
    let mut record = Record::default();
    record.insert("a.first", "one").unwrap();
    record.insert("z.last", "two").unwrap();
    let old = record.render().replacen(FORMAT, "wizard-evidence-v9", 1);

    let error = parse_manifest(&old).unwrap_err().to_string();
    assert!(error.contains("no supported format header"), "{error}");
    let previous = record.render().replacen(FORMAT, "wizard-evidence-v10", 1);
    let error = parse_manifest(&previous).unwrap_err().to_string();
    assert!(error.contains("no supported format header"), "{error}");
}

#[test]
fn an_omitted_campaign_status_is_refused_not_defaulted() {
    let [grade, deep] = qualification::CampaignStates::status_keys();
    assert!(declared_campaigns(&Record::default()).is_err());
    let mut only_grade = Record::default();
    only_grade.insert(grade.clone(), "deferred").unwrap();
    assert!(declared_campaigns(&only_grade).is_err());
    let mut only_deep = Record::default();
    only_deep.insert(deep.clone(), "deferred").unwrap();
    assert!(declared_campaigns(&only_deep).is_err());

    let mut record = Record::default();
    record.insert(grade, "deferred").unwrap();
    record.insert(deep, PASS).unwrap();
    assert_eq!(
        declared_campaigns(&record).unwrap(),
        qualification::CampaignStates::from_labels(["deferred", PASS]).unwrap()
    );
}

#[test]
fn a_broken_manifest_symlink_cannot_redirect_the_writer() {
    let root = reserve_temp_directory("wizard-evidence-symlink");
    let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
    let evidence = evidence_root(&paths);
    let external = root.join("outside-manifest");
    std::fs::create_dir_all(&evidence).unwrap();
    create_file_symlink(&external, &evidence.join(MANIFEST_TEMP));

    invalidate_manifest(&paths).unwrap();

    assert!(!std::fs::symlink_metadata(evidence.join(MANIFEST_TEMP)).is_ok());
    assert!(!external.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("creating broken manifest symlink");
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_file(target, link).expect("creating broken manifest symlink");
}

#[cfg(unix)]
fn create_directory_symlink(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).expect("creating directory symlink");
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) {
    std::os::windows::fs::symlink_dir(target, link).expect("creating directory symlink");
}

fn write_qualification_bundle_fixture(root: &Path) {
    for spec in qualification::BUNDLE_FILES {
        let path = root.join(spec.relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = File::create(path).unwrap();
        file.write_all(spec.prefix.as_bytes()).unwrap();
        let (maximum, exact) = qualification::bundle_file_size_policy(spec).unwrap();
        if exact {
            file.set_len(maximum).unwrap();
        }
    }
}

#[test]
fn a_substituted_evidence_root_is_never_recursively_reset() {
    let root = reserve_temp_directory("wizard-evidence-root-link");
    let outside = root.join("outside");
    let evidence = root.join("evidence-link");
    std::fs::create_dir_all(outside.join("work-a")).unwrap();
    std::fs::write(outside.join("work-a/keep"), "outside").unwrap();
    create_directory_symlink(&outside, &evidence);

    assert!(reset_child(&evidence, "work-a").is_err());
    assert_eq!(
        std::fs::read_to_string(outside.join("work-a/keep")).unwrap(),
        "outside"
    );

    let ancestor_target = root.join("ancestor-target");
    let ancestor_link = root.join("ancestor-link");
    std::fs::create_dir_all(ancestor_target.join("evidence")).unwrap();
    create_directory_symlink(&ancestor_target, &ancestor_link);
    assert!(require_evidence_root(&ancestor_link.join("evidence")).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_redirected_target_is_refused_before_evidence_root_creation() {
    let root = reserve_temp_directory("wizard-evidence-target-link");
    let outside = root.join("outside");
    let target = root.join("target-link");
    std::fs::create_dir_all(&outside).unwrap();
    create_directory_symlink(&outside, &target);
    let paths = Paths::new(root.join("repository"), Some(&target));

    assert!(invalidate_manifest(&paths).is_err());
    assert!(!outside.join("evidence").exists());
    std::fs::remove_dir_all(root).unwrap();
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

#[test]
fn an_evidence_attempt_invalidates_an_older_success_first() {
    let root = reserve_temp_directory("wizard-evidence-invalidation");
    let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
    let evidence = evidence_root(&paths);
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join(MANIFEST_NAME), "stale pass").unwrap();

    invalidate_manifest(&paths).unwrap();

    assert!(!evidence.join(MANIFEST_NAME).exists());
    let source = include_str!("../evidence.rs");
    let create = source.find("pub(crate) fn create").unwrap();
    let verify = source[create..].find("pub(crate) fn verify").unwrap() + create;
    let body = &source[create..verify];
    assert!(
        body.find("invalidate_manifest(paths)?;").unwrap() < body.find("qemu::nvme_image").unwrap()
    );
    assert!(
        body.find("verify_record(paths, &root, &record)?;").unwrap()
            < body
                .find("write_manifest(&root, &manifest, &record.render())?;")
                .unwrap()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn qualification_records_cannot_be_omitted_or_changed_offline() {
    let bundled: BTreeSet<&str> = qualification::BUNDLE_FILES
        .iter()
        .map(|spec| spec.relative)
        .collect();
    assert_eq!(
        bundled,
        BTreeSet::from([
            "qualification/cargo-mutants-grade/adjudications.json",
            "qualification/cargo-mutants-grade/doctest.transcript",
            "qualification/cargo-mutants-grade/inventory.json",
            "qualification/cargo-mutants-grade/mutants.json",
            "qualification/cargo-mutants-grade/outcomes.json",
            "qualification/cargo-mutants-grade/record",
            "qualification/cargo-mutants-grade/request",
            "qualification/codex-security-deep/coverage.json",
            "qualification/codex-security-deep/findings.json",
            "qualification/codex-security-deep/record",
            "qualification/codex-security-deep/report.md",
            "qualification/codex-security-deep/request",
            "qualification/codex-security-deep/scan-manifest.json",
            "qualification/codex-security-deep/worker-transitions",
            "qualification/advisory/.cargo/audit.toml",
            "qualification/advisory/cargo-audit-refresh.transcript",
            "qualification/advisory/cargo-audit.transcript",
            "qualification/advisory/record",
            "qualification/model/record",
            "qualification/model/ring-clean.transcript",
            "qualification/model/ring-memory-clean.transcript",
            "qualification/model/ring-memory-duplicate-terminal.transcript",
            "qualification/model/ring-memory-load-relaxed.transcript",
            "qualification/model/ring-memory-no-reservation.transcript",
            "qualification/model/ring-memory-store-relaxed.transcript",
            "qualification/mutation/record",
            "qualification/mutation/report",
            "qualification/vmware-self-test/generation.receipt",
            "qualification/vmware-self-test/record",
            "qualification/vmware-self-test/serial.log",
            "qualification/vmware-self-test/vmrun-list.txt",
            "qualification/vmware-self-test/wizard-nvm-flat.img",
            "qualification/vmware-self-test/wizard-nvm.vmdk",
            "qualification/vmware-self-test/bedrock-flat.img",
            "qualification/vmware-self-test/bedrock.vmdk",
            "qualification/vmware-self-test/bedrock.vmx",
            "qualification/vmware-self-test/dma-handoff.bin",
            "qualification/vmware-shipped/generation.receipt",
            "qualification/vmware-shipped/record",
            "qualification/vmware-shipped/serial.log",
            "qualification/vmware-shipped/vmrun-list.txt",
            "qualification/vmware-shipped/wizard-nvm-flat.img",
            "qualification/vmware-shipped/wizard-nvm.vmdk",
            "qualification/vmware-shipped/bedrock-flat.img",
            "qualification/vmware-shipped/bedrock.vmdk",
            "qualification/vmware-shipped/bedrock.vmx",
            "qualification/vmware-shipped/dma-handoff.bin",
        ])
    );
    let root = reserve_temp_directory("wizard-evidence-qualification-fixture");
    write_qualification_bundle_fixture(&root);
    let campaigns = qualification::CampaignStates::from_labels([PASS, PASS]).unwrap();
    let mut record = Record::default();
    add_qualification_files(&mut record, &root, &campaigns).unwrap();
    let expected: BTreeSet<String> = record.fields.keys().cloned().collect();

    record.fields.remove("qualification.model.record.sha256");
    assert!(require_exact_keys(&record, &expected).is_err());

    let mut record = Record::default();
    add_qualification_files(&mut record, &root, &campaigns).unwrap();
    let model_record = root.join("qualification/model/record");
    let bytes = std::fs::read(&model_record).unwrap();
    std::fs::write(&model_record, vec![b'x'; bytes.len()]).unwrap();
    let error = verify_qualification_files(&record, &root, &campaigns)
        .unwrap_err()
        .to_string();
    assert!(error.contains("qualification.model.record"), "{error}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_bundle_parent_link_is_rejected_before_qualification_hashing() {
    let root = reserve_temp_directory("wizard-evidence-parent-link");
    let spec = qualification::BUNDLE_FILES
        .iter()
        .find(|spec| spec.relative == "qualification/model/record")
        .unwrap();
    let model = root.join("qualification/model");
    let alternate = root.join("qualification/model-alternate");
    std::fs::create_dir_all(&model).unwrap();
    std::fs::write(model.join("record"), spec.prefix).unwrap();
    std::fs::rename(&model, &alternate).unwrap();
    create_directory_symlink(&alternate, &model);
    let hashing_started = std::cell::Cell::new(false);

    assert!(
        verified_qualification_digest_with(&root, spec, || {
            hashing_started.set(true);
            Ok(())
        })
        .is_err()
    );
    assert!(!hashing_started.get());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn exceptional_artifacts_are_the_exact_reproduced_inputs() {
    let root = reserve_temp_directory("wizard-evidence-cross-artifact-fixture");
    let repository = root.join("repo");
    let evidence = root.join("evidence");
    let paths = Paths::new(repository.clone(), None);
    let files = [
        evidence.join(ARTIFACTS[1].relative),
        evidence.join("qualification/vmware-self-test/bedrock-flat.img"),
        evidence.join("qualification/vmware-shipped/bedrock-flat.img"),
        evidence.join("qualification/vmware-self-test/wizard-nvm-flat.img"),
        evidence.join("qualification/vmware-shipped/wizard-nvm-flat.img"),
        qemu::nvme_path(&repository),
    ];
    for path in &files {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    std::fs::write(&files[0], "shipped!").unwrap();
    std::fs::write(&files[1], "selftest").unwrap();
    std::fs::write(&files[2], "shipped!").unwrap();
    for path in &files[3..] {
        std::fs::write(path, "namespace").unwrap();
    }
    assert!(verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_ok());

    std::fs::write(&files[2], "changed!").unwrap();
    assert!(verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_err());
    std::fs::write(&files[2], "shipped!").unwrap();
    std::fs::write(&files[4], "foreign!!").unwrap();
    assert!(verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_err());
    std::fs::write(&files[4], "namespace").unwrap();
    std::fs::write(&files[1], "shipped!").unwrap();
    assert!(verify_qualification_artifact_identity_with_sizes(&paths, &evidence, 8, 9).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn optional_tools_never_change_the_manifest_schema() {
    let mut absent = Record::default();
    add_tool_identity(&mut absent, "tool.java", &ToolIdentity::Unavailable).unwrap();

    let mut present = Record::default();
    add_tool_identity(
        &mut present,
        "tool.java",
        &ToolIdentity::Available {
            version: "java 21\nVM build 11".to_string(),
            digest: Digest {
                bytes: 42,
                sha256: "a".repeat(64),
            },
        },
    )
    .unwrap();

    assert_eq!(
        absent.fields.keys().collect::<Vec<_>>(),
        present.fields.keys().collect::<Vec<_>>()
    );
    assert_eq!(absent.get("tool.java.state").unwrap(), UNAVAILABLE);
    assert_eq!(
        present.get("tool.java.version").unwrap(),
        "java 21\\nVM build 11"
    );
}

#[test]
fn an_unavailable_qualification_dependency_is_named_exactly() {
    let mut recorded = Record::default();
    let mut observed = Record::default();
    add_tool_identity(
        &mut recorded,
        "tool.java",
        &ToolIdentity::Available {
            version: "java 21".to_string(),
            digest: Digest {
                bytes: 42,
                sha256: "a".repeat(64),
            },
        },
    )
    .unwrap();
    add_tool_identity(&mut observed, "tool.java", &ToolIdentity::Unavailable).unwrap();
    add_tool_identity(&mut recorded, "tool.vmware", &ToolIdentity::Unavailable).unwrap();
    add_tool_identity(&mut observed, "tool.vmware", &ToolIdentity::Unavailable).unwrap();

    let error = require_inputs_match(&recorded, &observed)
        .unwrap_err()
        .to_string();
    assert!(error.contains("tool.java"), "{error}");
    assert!(error.contains("manifest `available`"), "{error}");
    assert!(error.contains("current `unavailable`"), "{error}");
}
