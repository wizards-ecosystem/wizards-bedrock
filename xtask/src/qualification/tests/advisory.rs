use super::*;

fn advisory_fixture(label: &str) -> (PathBuf, Paths, PathBuf, SourceIdentity) {
    let fixture = crate::test_support::reserve_temp_directory(&format!("wizard-advisory-{label}"));
    let repository = fixture.join("repository");
    let root = fixture.join("qualification");
    let directory = root.join(Kind::Advisory.slug());
    std::fs::create_dir_all(directory.join(".cargo")).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::write(repository.join("Cargo.lock"), "locked graph\n").unwrap();
    std::fs::write(directory.join(ADVISORY_POLICY_PATH), ADVISORY_POLICY).unwrap();
    let database_revision = "d".repeat(40);
    let output = Output {
        status: success_status(),
        stdout: clean_advisory_report(&database_revision),
        stderr: b"Fetching advisory database\n".to_vec(),
    };
    std::fs::write(
        directory.join(ADVISORY_REFRESH_TRANSCRIPT),
        render_advisory_transcript(&output, &database_revision, DatabaseBinding::Fetched).unwrap(),
    )
    .unwrap();
    let pinned = Output {
        status: success_status(),
        stdout: pinned_advisory_report(),
        stderr: Vec::new(),
    };
    std::fs::write(
        directory.join(ADVISORY_TRANSCRIPT),
        render_advisory_transcript(&pinned, &database_revision, DatabaseBinding::Pinned).unwrap(),
    )
    .unwrap();
    let mut record = complete_record(Kind::Advisory);
    let lock = digest_file(&repository.join("Cargo.lock")).unwrap();
    record
        .fields
        .insert("input.cargo-lock.bytes".to_string(), lock.bytes.to_string());
    record
        .fields
        .insert("input.cargo-lock.sha256".to_string(), lock.sha256);
    for (prefix, path) in [
        ("artifact.policy", directory.join(ADVISORY_POLICY_PATH)),
        (
            "artifact.refresh-transcript",
            directory.join(ADVISORY_REFRESH_TRANSCRIPT),
        ),
        ("artifact.transcript", directory.join(ADVISORY_TRANSCRIPT)),
    ] {
        let digest = digest_file(&path).unwrap();
        record
            .fields
            .insert(format!("{prefix}.bytes"), digest.bytes.to_string());
        record
            .fields
            .insert(format!("{prefix}.sha256"), digest.sha256);
    }
    std::fs::write(directory.join(RECORD), record.render()).unwrap();
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    (fixture, Paths::new(repository, None), root, source)
}

fn clean_advisory_report(database_revision: &str) -> Vec<u8> {
    format!(
        "{{\"database\":{{\"advisory-count\":1,\"last-commit\":\"{database_revision}\",\"last-updated\":\"2026-08-30T12:00:00Z\"}},\"lockfile\":{{\"dependency-count\":1}},\"settings\":{{\"target_arch\":[],\"target_os\":[],\"severity\":null,\"ignore\":[],\"informational_warnings\":[\"unmaintained\",\"unsound\",\"notice\"]}},\"vulnerabilities\":{{\"found\":false,\"count\":0,\"list\":[]}},\"warnings\":{{}}}}\n"
    )
    .into_bytes()
}

fn pinned_advisory_report() -> Vec<u8> {
    String::from_utf8(clean_advisory_report(&"d".repeat(40)))
        .unwrap()
        .replace(&format!("\"{}\"", "d".repeat(40)), "null")
        .replace("\"2026-08-30T12:00:00Z\"", "null")
        .into_bytes()
}

fn unchecked_advisory_transcript(stdout: &[u8]) -> Vec<u8> {
    let stderr = b"";
    let header = format!(
        "{ADVISORY_TRANSCRIPT_FORMAT}\nexit.code=0\nstderr.bytes={}\nstderr.sha256={}\nstdout.bytes={}\nstdout.sha256={}\n\n",
        stderr.len(),
        sha256::hex(&sha256::digest(stderr)),
        stdout.len(),
        sha256::hex(&sha256::digest(stdout)),
    );
    let mut transcript = header.into_bytes();
    transcript.extend_from_slice(stdout);
    transcript.extend_from_slice(stderr);
    transcript
}

fn rebind_advisory_artifact(root: &Path, prefix: &str, path: &Path) {
    let record_path = root.join(Kind::Advisory.slug()).join(RECORD);
    let mut record = read_record(&record_path, Kind::Advisory).unwrap();
    let digest = digest_file(path).unwrap();
    record
        .fields
        .insert(format!("{prefix}.bytes"), digest.bytes.to_string());
    record
        .fields
        .insert(format!("{prefix}.sha256"), digest.sha256);
    std::fs::write(record_path, record.render()).unwrap();
}

#[test]
fn advisory_schema_binds_database_time_command_and_tool_identity() {
    for field in ["database", "database-time", "time", "command", "tool"] {
        let mut record = complete_record(Kind::Advisory);
        match field {
            "database" => {
                record
                    .fields
                    .insert("database.revision".to_string(), "forged".to_string());
            }
            "database-time" => {
                record.fields.insert(
                    "database.last-updated".to_string(),
                    "not-a-time".to_string(),
                );
            }
            "time" => {
                record
                    .fields
                    .insert("observation.utc-unix-seconds".to_string(), "0".to_string());
            }
            "command" => {
                record.fields.insert(
                    "execution.command".to_string(),
                    "cargo-audit audit --stale".to_string(),
                );
            }
            "tool" => {
                record.fields.insert(
                    "input.cargo-audit.version".to_string(),
                    "cargo 1.97.0".to_string(),
                );
            }
            _ => unreachable!(),
        }
        assert!(
            validate_record(&record, Kind::Advisory).is_err(),
            "forged advisory {field} identity was accepted"
        );
    }
}

#[test]
fn advisory_database_identity_rejects_forged_origin_and_dirty_bytes() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-advisory-database");
    let parent = fixture.join("advisory");
    let database = parent.join(ADVISORY_DATABASE);
    std::fs::create_dir_all(&database).unwrap();
    run_git(&database, &["init", "--quiet"]);
    std::fs::write(database.join("advisory.toml"), "[advisory]\n").unwrap();
    run_git(&database, &["add", "advisory.toml"]);
    run_git(
        &database,
        &[
            "-c",
            "user.name=Wizard Test",
            "-c",
            "user.email=wizard@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    run_git(
        &database,
        &["remote", "add", "origin", ADVISORY_DATABASE_URL],
    );
    advisory_database_identity(&parent, &database).unwrap();

    run_git(
        &database,
        &[
            "remote",
            "set-url",
            "origin",
            "https://example.invalid/forged.git",
        ],
    );
    assert!(advisory_database_identity(&parent, &database).is_err());
    run_git(
        &database,
        &["remote", "set-url", "origin", ADVISORY_DATABASE_URL],
    );
    std::fs::write(database.join("advisory.toml"), "forged\n").unwrap();
    assert!(advisory_database_identity(&parent, &database).is_err());
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_missing_advisory_record_is_rejected() {
    let (fixture, paths, root, source) = advisory_fixture("missing");
    std::fs::remove_file(root.join(Kind::Advisory.slug()).join(RECORD)).unwrap();

    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();

    assert!(error.contains("qualification record"), "{error}");
    let implementation = PRODUCTION;
    let verification = implementation.find("fn verify_records(").unwrap();
    let advisory = implementation[verification..]
        .find("verify_advisory_qualification(paths, root, source)?;")
        .unwrap();
    let loop_start = implementation[verification..]
        .find("for kind in [")
        .unwrap();
    assert!(advisory < loop_start);
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn a_stale_advisory_record_cannot_describe_another_lockfile() {
    let (fixture, paths, root, source) = advisory_fixture("stale");
    verify_advisory_qualification(&paths, &root, &source).unwrap();
    std::fs::write(paths.root().join("Cargo.lock"), "changed locked graph\n").unwrap();

    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();

    assert!(error.contains("another Cargo.lock"), "{error}");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn forged_advisory_policy_and_result_bytes_are_rejected_semantically() {
    let (fixture, paths, root, source) = advisory_fixture("forged");
    let directory = root.join(Kind::Advisory.slug());
    let transcript_path = directory.join(ADVISORY_TRANSCRIPT);
    let valid = std::fs::read(&transcript_path).unwrap();
    let mut forged = valid.clone();
    let result = forged
        .windows(b"exit.code=0".len())
        .position(|window| window == b"exit.code=0")
        .unwrap();
    forged[result + b"exit.code=".len()] = b'1';
    std::fs::write(&transcript_path, forged).unwrap();
    rebind_advisory_artifact(&root, "artifact.transcript", &transcript_path);
    assert!(verify_advisory_qualification(&paths, &root, &source).is_err());

    std::fs::write(&transcript_path, valid).unwrap();
    rebind_advisory_artifact(&root, "artifact.transcript", &transcript_path);
    let policy = directory.join(ADVISORY_POLICY_PATH);
    std::fs::write(&policy, "[advisories]\nignore = [\"RUSTSEC-2099-0001\"]\n").unwrap();
    rebind_advisory_artifact(&root, "artifact.policy", &policy);
    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();
    assert!(error.contains("no-ignore"), "{error}");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn forged_green_advisory_json_is_rejected_semantically() {
    let (fixture, paths, root, source) = advisory_fixture("forged-json");
    let transcript = root.join(Kind::Advisory.slug()).join(ADVISORY_TRANSCRIPT);
    let revision = "d".repeat(40);
    let clean = String::from_utf8(clean_advisory_report(&revision)).unwrap();
    let hostile = [
        ("empty object", "{}\n".to_string()),
        (
            "unknown top-level field",
            clean.replace("\"warnings\":{}", "\"warnings\":{},\"forged\":true"),
        ),
        (
            "unknown database field",
            clean.replace(
                "\"advisory-count\":1",
                "\"advisory-count\":1,\"forged\":true",
            ),
        ),
        (
            "empty advisory database",
            clean.replace("\"advisory-count\":1", "\"advisory-count\":0"),
        ),
        (
            "malformed database time",
            clean.replace("2026-08-30T12:00:00Z", "not-a-time"),
        ),
        (
            "unknown lockfile field",
            clean.replace(
                "\"dependency-count\":1",
                "\"dependency-count\":1,\"forged\":true",
            ),
        ),
        (
            "empty lockfile",
            clean.replace("\"dependency-count\":1", "\"dependency-count\":0"),
        ),
        (
            "unknown settings field",
            clean.replace("\"target_arch\":[]", "\"target_arch\":[],\"forged\":true"),
        ),
        (
            "vulnerability found",
            clean.replace("\"found\":false", "\"found\":true"),
        ),
        (
            "unknown vulnerability field",
            clean.replace("\"found\":false", "\"found\":false,\"forged\":true"),
        ),
        (
            "inconsistent vulnerability count",
            clean.replace("\"count\":0,\"list\":[]", "\"count\":1,\"list\":[]"),
        ),
        (
            "dependency warning",
            clean.replace("\"warnings\":{}", "\"warnings\":{\"unmaintained\":[{}]}"),
        ),
        (
            "ignored advisory setting",
            clean.replace("\"ignore\":[]", "\"ignore\":[\"RUSTSEC-2099-0001\"]"),
        ),
        (
            "different database revision",
            clean.replace(&revision, &"e".repeat(40)),
        ),
    ];

    for (label, report) in hostile {
        std::fs::write(
            &transcript,
            unchecked_advisory_transcript(report.as_bytes()),
        )
        .unwrap();
        rebind_advisory_artifact(&root, "artifact.transcript", &transcript);
        assert!(
            verify_advisory_qualification(&paths, &root, &source).is_err(),
            "forged {label} report was accepted"
        );
    }
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn an_empty_advisory_database_cannot_author_a_qualification() {
    let revision = "d".repeat(40);
    let report = String::from_utf8(clean_advisory_report(&revision))
        .unwrap()
        .replace("\"advisory-count\":1", "\"advisory-count\":0");

    let error = verify_advisory_report(report.as_bytes(), &revision, DatabaseBinding::Fetched)
        .unwrap_err()
        .to_string();

    assert!(error.contains("empty advisory database"), "{error}");
}

#[test]
fn a_pinned_audit_report_carries_null_database_identity_by_tool_design() {
    let revision = "d".repeat(40);
    let facts = verify_advisory_report(
        &pinned_advisory_report(),
        &revision,
        DatabaseBinding::Pinned,
    )
    .unwrap();
    assert_eq!(facts.advisory_count, 1);
    assert_eq!(facts.last_updated, None);
    let fetched = verify_advisory_report(
        &clean_advisory_report(&revision),
        &revision,
        DatabaseBinding::Fetched,
    )
    .unwrap();
    assert_eq!(
        fetched.last_updated.as_deref(),
        Some("2026-08-30T12:00:00Z")
    );
    let error = verify_advisory_report(
        &pinned_advisory_report(),
        &revision,
        DatabaseBinding::Fetched,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("no usable advisory database identity"),
        "{error}"
    );
}

#[test]
fn a_population_transcript_without_database_identity_is_rejected() {
    let (fixture, paths, root, source) = advisory_fixture("null-population");
    let refresh = root
        .join(Kind::Advisory.slug())
        .join(ADVISORY_REFRESH_TRANSCRIPT);
    std::fs::write(
        &refresh,
        unchecked_advisory_transcript(&pinned_advisory_report()),
    )
    .unwrap();
    rebind_advisory_artifact(&root, "artifact.refresh-transcript", &refresh);
    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("no usable advisory database identity"),
        "{error}"
    );
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn the_pinned_audit_must_see_the_population_runs_database() {
    let (fixture, paths, root, source) = advisory_fixture("count-drift");
    let refresh = root
        .join(Kind::Advisory.slug())
        .join(ADVISORY_REFRESH_TRANSCRIPT);
    let drifted = String::from_utf8(clean_advisory_report(&"d".repeat(40)))
        .unwrap()
        .replace("\"advisory-count\":1", "\"advisory-count\":2");
    std::fs::write(&refresh, unchecked_advisory_transcript(drifted.as_bytes())).unwrap();
    rebind_advisory_artifact(&root, "artifact.refresh-transcript", &refresh);
    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();
    assert!(error.contains("different advisory databases"), "{error}");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn an_advisory_record_must_repeat_the_attested_database_update_time() {
    let (fixture, paths, root, source) = advisory_fixture("update-time");
    let record_path = root.join(Kind::Advisory.slug()).join(RECORD);
    let mut record = read_record(&record_path, Kind::Advisory).unwrap();
    record.fields.insert(
        "database.last-updated".to_string(),
        "2026-08-31T12:00:00Z".to_string(),
    );
    std::fs::write(&record_path, record.render()).unwrap();
    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();
    assert!(error.contains("another database update time"), "{error}");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn advisory_transcript_rendering_refuses_output_beyond_its_ceiling() {
    let revision = "d".repeat(40);
    let output = Output {
        status: success_status(),
        stdout: clean_advisory_report(&revision),
        stderr: Vec::new(),
    };
    let error =
        render_advisory_transcript_with_limit(&output, &revision, DatabaseBinding::Fetched, 1)
            .unwrap_err()
            .to_string();

    assert!(
        error.contains("cargo-audit transcript exceeds its bounded artifact limit"),
        "{error}"
    );
}

#[test]
fn advisory_transcript_verification_refuses_bytes_beyond_its_ceiling() {
    let revision = "d".repeat(40);
    let output = Output {
        status: success_status(),
        stdout: clean_advisory_report(&revision),
        stderr: Vec::new(),
    };
    let transcript =
        render_advisory_transcript(&output, &revision, DatabaseBinding::Fetched).unwrap();
    let error = verify_advisory_transcript_with_limit(
        &transcript,
        &revision,
        DatabaseBinding::Fetched,
        transcript.len() as u64 - 1,
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("cargo-audit transcript exceeds its bounded artifact limit"),
        "{error}"
    );
}

#[test]
fn an_oversized_advisory_transcript_is_rejected_before_reading() {
    let (fixture, paths, root, source) = advisory_fixture("oversized");
    let transcript = root.join(Kind::Advisory.slug()).join(ADVISORY_TRANSCRIPT);
    OpenOptions::open(OpenOptions::write(&mut File::options(), true), &transcript)
        .unwrap()
        .set_len(MAX_ADVISORY_TRANSCRIPT_BYTES + 1)
        .unwrap();

    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();

    assert!(error.contains("exceeding its"), "{error}");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn an_advisory_record_bound_to_another_source_is_rejected() {
    let (fixture, paths, root, _) = advisory_fixture("source");
    let source = SourceIdentity {
        commit: "9".repeat(40),
        tree: "8".repeat(40),
    };

    let error = verify_advisory_qualification(&paths, &root, &source)
        .unwrap_err()
        .to_string();

    assert!(error.contains("another commit"), "{error}");
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn advisory_invalidation_precedes_even_failed_tool_resolution() {
    let root = crate::test_support::reserve_temp_directory("wizard-advisory-invalidation");
    let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
    let directory = qualification_root(&paths).join(Kind::Advisory.slug());
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(RECORD), "stale advisory pass").unwrap();

    let error = begin_advisory_with(&paths, || bail!("injected tool resolution failure"))
        .unwrap_err()
        .to_string();

    assert!(
        error.contains("injected tool resolution failure"),
        "{error}"
    );
    assert!(!directory.join(RECORD).exists());
    assert!(directory.is_dir());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cargo_audit_version_is_one_exact_resolved_identity() {
    assert_eq!(
        cargo_audit_version(ADVISORY_TOOL_VERSION).unwrap(),
        ADVISORY_TOOL_VERSION
    );
    assert!(cargo_audit_version("cargo-audit-audit 0.22.2\nforged").is_err());
    assert!(cargo_audit_version("cargo-audit-audit 0.22.3").is_err());
    assert!(cargo_audit_version("cargo 1.97.0").is_err());
}

#[test]
fn advisory_refresh_and_review_commands_are_direct_exact_and_bounded() {
    let tool = Tool {
        path: PathBuf::from("resolved-cargo-audit"),
        version: ADVISORY_TOOL_VERSION.to_string(),
    };
    let directory = PathBuf::from("qualification/advisory");
    let database = directory.join(ADVISORY_DATABASE);
    let lock = PathBuf::from("repository/Cargo.lock");
    for fetch in [true, false] {
        let command = advisory_command(&tool, &directory, &database, &lock, fetch);
        let mut expected: Vec<std::ffi::OsString> = [
            "audit",
            "--color",
            "never",
            "--db",
            database.to_str().unwrap(),
            "--deny",
            "warnings",
            "--file",
            lock.to_str().unwrap(),
            "--json",
        ]
        .into_iter()
        .map(std::ffi::OsString::from)
        .collect();
        if !fetch {
            expected.push("--no-fetch".into());
        }
        expected.extend([
            std::ffi::OsString::from("--url"),
            std::ffi::OsString::from(ADVISORY_DATABASE_URL),
        ]);

        assert_eq!(command.get_program(), tool.path.as_os_str());
        assert_eq!(command.get_current_dir(), Some(directory.as_path()));
        assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
        assert!(command.get_envs().any(|(name, value)| {
            name == std::ffi::OsStr::new("CARGO_NET_OFFLINE") && value.is_none()
        }));
    }
}
