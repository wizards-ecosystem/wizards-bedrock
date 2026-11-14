use super::*;
use crate::test_support::{create_directory_symlink, create_file_symlink, reserve_temp_directory};

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
fn a_release_artifact_that_retains_a_host_path_is_refused() {
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
