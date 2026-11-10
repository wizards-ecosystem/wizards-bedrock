use super::*;

fn with_fixture_root<T>(operation: impl FnOnce(&Path) -> T) -> T {
    let root = crate::test_support::reserve_temp_directory("wizard-mutants-grade-fixture");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&root)));
    std::fs::remove_dir_all(&root).expect("removing cargo-mutants test fixture");
    match outcome {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn fixture_executable(root: &Path, name: &str) -> ExecutableIdentity {
    ExecutableIdentity {
        path: root.join(name).to_string_lossy().into_owned(),
        bytes: 1,
        sha256: "a".repeat(64),
    }
}

fn fixture_tools(root: &Path) -> ToolIdentity {
    ToolIdentity {
        cargo_mutants_path: root
            .join("cargo-mutants.exe")
            .to_string_lossy()
            .into_owned(),
        cargo_mutants_bytes: 12,
        cargo_mutants_sha256: CARGO_MUTANTS_EXECUTABLE_SHA256.to_string(),
        nextest_path: root
            .join("cargo-nextest.exe")
            .to_string_lossy()
            .into_owned(),
        nextest_bytes: 34,
        cargo: fixture_executable(root, "cargo.exe"),
        rustc: fixture_executable(root, "rustc.exe"),
        rustdoc: fixture_executable(root, "rustdoc.exe"),
        host_linker: fixture_executable(root, "rust-lld.exe"),
        dlltool: fixture_executable(
            resolve_path("dlltool.exe")
                .unwrap_or_else(|_| root.join("dlltool.exe"))
                .parent()
                .unwrap_or(root),
            "dlltool.exe",
        ),
        host_target: "x86_64-pc-windows-gnu".to_string(),
    }
}

fn fixture_private_temp(paths: &Paths) -> PathBuf {
    crate::mutate::workspace::prepare_real_directory_tree(
        &paths
            .target_dir()
            .join("qualification/test-cargo-mutants-temp"),
        "test cargo-mutants temp root",
    )
    .unwrap()
}

#[cfg(unix)]
fn exit_status(code: u8) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(i32::from(code) << 8)
}

#[cfg(windows)]
fn exit_status(code: u8) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(u32::from(code))
}

#[test]
fn direct_cargo_mutants_probe_names_its_subcommand() {
    with_fixture_root(|root| {
        let paths = Paths::new(root.to_path_buf(), Some(&root.join("target")));
        let tools = fixture_tools(root);
        let private_temp = fixture_private_temp(&paths);
        let command = cargo_mutants_version_command(&tools, &paths, &private_temp).unwrap();
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["mutants", "--version"]
        );
    });
}

fn candidate(name: &str) -> String {
    format!(
        r#"{{"diff":"d","file":"xtask/src/lib.rs","function":{{}},"genre":"FnValue","name":"{name}","package":"xtask","replacement":"false","span":{{}}}}"#
    )
}

fn phases(test_status: &str) -> String {
    format!(
        r#"[{{"phase":"Build","duration":1,"process_status":"Success","argv":["cargo","nextest","run","--no-run","--verbose","--package=xtask@0.1.0"]}},{{"phase":"Test","duration":2,"process_status":{test_status},"argv":["cargo","nextest","run","--verbose","--package=xtask@0.1.0"]}}]"#
    )
}

fn outcomes(summary: &str, end: &str) -> Vec<u8> {
    let baseline = phases("\"Success\"");
    let mutant = match summary {
        "CaughtMutant" | "Failure" => phases(r#"{"Failure":100}"#),
        "MissedMutant" => phases("\"Success\""),
        _ => phases("\"Success\""),
    };
    format!(r#"{{"outcomes":[{{"scenario":"Baseline","summary":"Success","log_path":"log/baseline.log","diff_path":null,"phase_results":{baseline}}},{{"scenario":{{"Mutant":{{"name":"m","package":"xtask","file":"xtask/src/lib.rs","function":{{}},"span":{{}},"replacement":"false","genre":"FnValue"}}}},"summary":"{summary}","log_path":"log/m.log","diff_path":"diff/m.diff","phase_results":{mutant}}}],"total_mutants":1,"missed":0,"caught":1,"timeout":0,"unviable":0,"success":0,"start_time":"2026-09-27T09:03:24.9388039Z","end_time":{end},"cargo_mutants_version":"27.1.0"}}"#).into_bytes()
}

#[test]
fn complete_grade_has_one_baseline_and_an_exact_candidate_bijection() {
    let candidates =
        parse_candidates(format!("[{}]", candidate("m")).as_bytes(), "fixture").unwrap();
    let (grade, noncaught) = parse_outcomes(
        &outcomes("CaughtMutant", "\"2026-09-27T09:06:03.3224359Z\""),
        &candidates,
    )
    .unwrap();
    assert_eq!(grade.total, 1);
    assert_eq!(grade.caught, 1);
    assert!(noncaught.is_empty());
    let duplicate = format!("[{},{}]", candidate("m"), candidate("m"));
    assert!(parse_candidates(duplicate.as_bytes(), "duplicate fixture").is_err());
}

#[test]
fn interrupted_and_unclassified_runs_are_not_qualifications() {
    let candidates = BTreeMap::from([("m".to_string(), "d".to_string())]);
    assert!(parse_outcomes(&outcomes("CaughtMutant", "null"), &candidates).is_err());
    assert!(
        parse_outcomes(
            &outcomes("Failure", "\"2026-09-27T09:06:03.3224359Z\""),
            &candidates
        )
        .is_err()
    );
}

#[test]
fn every_non_caught_outcome_needs_one_equivalence_adjudication() {
    let expected = BTreeMap::from([(
        "m".to_string(),
        NonCaught {
            summary: "MissedMutant".to_string(),
            diff: "d".to_string(),
        },
    )]);
    let good = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{"name":"m","summary":"MissedMutant","diff":"d","disposition":"equivalent","rationale":"the replacement is observationally identical"}]}"#;
    assert_eq!(verify_adjudications(good, &expected).unwrap(), 1);
    let missing = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[]}"#;
    assert!(verify_adjudications(missing, &expected).is_err());
}

#[test]
fn request_binds_source_command_concurrency_and_runner_provenance() {
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let tools = fixture_tools(fixture_root);
    let adjudications = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[]}"#;
    let request = render_request(&source, &tools, adjudications).unwrap();
    assert!(
        String::from_utf8_lossy(&request)
            .contains("command=cargo mutants -p xtask --output <xtask-owned-directory> --jobs 1")
    );
    assert_eq!(
        verify_request(&request, &source, adjudications).unwrap(),
        tools
    );
    let other_grader = ToolIdentity {
        cargo_mutants_sha256: "c".repeat(64),
        ..tools.clone()
    };
    assert!(
        verify_request(
            &render_request(&source, &other_grader, adjudications).unwrap(),
            &source,
            adjudications,
        )
        .is_err()
    );
    let weakened = String::from_utf8(request.clone())
        .unwrap()
        .replace("--jobs 1", "--jobs 2");
    let (body, _) = weakened.rsplit_once("checksum.sha256=").unwrap();
    let weakened = format!(
        "{body}checksum.sha256={}\n",
        sha256::hex(&sha256::digest(body.as_bytes()))
    );
    assert!(verify_request(weakened.as_bytes(), &source, adjudications).is_err());
    let changed_policy = br#"{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{}]}"#;
    assert!(verify_request(&request, &source, changed_policy).is_err());
}

#[test]
fn independent_inventory_must_match_run_identities_and_diffs() {
    let run = parse_candidates(format!("[{}]", candidate("m")).as_bytes(), "run").unwrap();
    let different = candidate("m").replace(r#""diff":"d""#, r#""diff":"different""#);
    let independent = parse_candidates(format!("[{different}]").as_bytes(), "inventory").unwrap();
    assert!(require_same_inventory(&run, &independent).is_err());
}

#[test]
fn phase_summary_requires_exact_order_status_and_nextest_argv() {
    let valid = phases(r#"{"Failure":100}"#);
    require_nextest_phases(
        &parse_json(valid.as_bytes(), "phases").unwrap(),
        "CaughtMutant",
    )
    .unwrap();
    let forged = phases("\"Success\"");
    assert!(
        require_nextest_phases(
            &parse_json(forged.as_bytes(), "phases").unwrap(),
            "CaughtMutant"
        )
        .is_err()
    );
    let wrong_order = valid.replacen(r#""phase":"Build""#, r#""phase":"Test""#, 1);
    assert!(
        require_nextest_phases(
            &parse_json(wrong_order.as_bytes(), "phases").unwrap(),
            "CaughtMutant"
        )
        .is_err()
    );
    let wrong_argv = valid.replace("--package=xtask@0.1.0", "--package=other@0.1.0");
    assert!(
        require_nextest_phases(
            &parse_json(wrong_argv.as_bytes(), "phases").unwrap(),
            "CaughtMutant"
        )
        .is_err()
    );
}

#[test]
fn doctest_transcript_requires_only_successful_zero_test_summaries() {
    let good = "   Doc-tests xtask\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
    verify_doctest(good.as_bytes()).unwrap();
    let forged = format!("{good}error: later failure\n");
    assert!(verify_doctest(forged.as_bytes()).is_err());
    assert!(verify_doctest(&[0xff]).is_err());
}

#[test]
fn doctest_transcript_belongs_to_exactly_the_xtask_package() {
    let good = "   Doc-tests xtask\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
    let headless = good.replacen("   Doc-tests xtask\n", "", 1);
    assert!(verify_doctest(headless.as_bytes()).is_err());
    let foreign = good.replacen("Doc-tests xtask", "Doc-tests fat32", 1);
    assert!(verify_doctest(foreign.as_bytes()).is_err());
    let doubled = format!("   Doc-tests other\n{good}");
    assert!(verify_doctest(doubled.as_bytes()).is_err());
}

#[test]
fn the_owned_output_directory_is_the_one_the_recorder_reads() {
    let arguments = COMMAND.split_whitespace().collect::<Vec<_>>();
    let output = arguments
        .windows(2)
        .find(|pair| pair[0] == "--output")
        .map(|pair| pair[1]);
    // cargo-mutants 27.1.0 creates `mutants.out` inside `--output` (its `output.rs`);
    // the recorder reads it.
    assert_eq!(output, Some("<xtask-owned-directory>"));
    assert_eq!(
        output_directory(Path::new("grade")),
        Path::new("grade").join("mutants.out")
    );
}

fn baseline_object() -> String {
    format!(
        r#"{{"scenario":"Baseline","summary":"Success","log_path":"log/baseline.log","diff_path":null,"phase_results":{}}}"#,
        phases("\"Success\"")
    )
}

#[test]
fn a_grade_ends_after_it_starts_and_recounts_every_outcome() {
    let candidates = BTreeMap::from([("m".to_string(), "d".to_string())]);
    let valid =
        String::from_utf8(outcomes("CaughtMutant", "\"2026-09-27T09:06:03.3224359Z\"")).unwrap();
    parse_outcomes(valid.as_bytes(), &candidates).unwrap();
    let instantaneous = valid.replace(
        "\"end_time\":\"2026-09-27T09:06:03.3224359Z\"",
        "\"end_time\":\"2026-09-27T09:03:24.9388039Z\"",
    );
    assert!(parse_outcomes(instantaneous.as_bytes(), &candidates).is_err());
    let unclassified = valid.replace("\"success\":0", "\"success\":1");
    assert!(parse_outcomes(unclassified.as_bytes(), &candidates).is_err());
    let miscounted = valid.replace("\"caught\":1", "\"caught\":2");
    assert!(parse_outcomes(miscounted.as_bytes(), &candidates).is_err());
}

#[test]
fn a_grade_has_one_baseline_and_an_outcome_for_every_candidate() {
    let candidates = BTreeMap::from([("m".to_string(), "d".to_string())]);
    let valid =
        String::from_utf8(outcomes("CaughtMutant", "\"2026-09-27T09:06:03.3224359Z\"")).unwrap();
    let baseline = baseline_object();
    assert!(valid.contains(&baseline));
    let doubled = valid.replacen(&baseline, &format!("{baseline},{baseline}"), 1);
    assert!(parse_outcomes(doubled.as_bytes(), &candidates).is_err());
    let absent = valid.replacen(&format!("{baseline},"), "", 1);
    assert!(parse_outcomes(absent.as_bytes(), &candidates).is_err());
    // Counters agree with the larger inventory, so only the missing outcome can refuse it.
    let two = BTreeMap::from([
        ("m".to_string(), "d".to_string()),
        ("n".to_string(), "e".to_string()),
    ]);
    let partial = valid.replace("\"total_mutants\":1", "\"total_mutants\":2");
    assert!(parse_outcomes(partial.as_bytes(), &two).is_err());
}

#[test]
fn adjudications_name_exactly_the_noncaught_mutants_with_allowed_reasons() {
    let expected = BTreeMap::from([(
        "m".to_string(),
        NonCaught {
            summary: "MissedMutant".to_string(),
            diff: "d".to_string(),
        },
    )]);
    let row = |name: &str, diff: &str, disposition: &str, rationale: &str| {
        format!(
            r#"{{"name":"{name}","summary":"MissedMutant","diff":"{diff}","disposition":"{disposition}","rationale":"{rationale}"}}"#
        )
    };
    let document = |rows: &[String]| {
        format!(
            r#"{{"format":"wizard-cargo-mutants-adjudications-v1","outcomes":[{}]}}"#,
            rows.join(",")
        )
    };
    let good = row("m", "d", "equivalent", "observationally identical");
    assert_eq!(
        verify_adjudications(document(std::slice::from_ref(&good)).as_bytes(), &expected).unwrap(),
        1
    );
    let wrong_reason = row("m", "d", "compiler-rejected", "observationally identical");
    assert!(verify_adjudications(document(&[wrong_reason]).as_bytes(), &expected).is_err());
    let wrong_diff = row("m", "different", "equivalent", "observationally identical");
    assert!(verify_adjudications(document(&[wrong_diff]).as_bytes(), &expected).is_err());
    let silent = row("m", "d", "equivalent", "  ");
    assert!(verify_adjudications(document(&[silent]).as_bytes(), &expected).is_err());
    let duplicated = document(&[good.clone(), good.clone()]);
    assert!(verify_adjudications(duplicated.as_bytes(), &expected).is_err());
    let extra = row("caught", "x", "equivalent", "observationally identical");
    assert!(verify_adjudications(document(&[good, extra]).as_bytes(), &expected).is_err());
}

#[test]
fn owned_commands_are_exactly_the_documented_ones() {
    with_fixture_root(|root| {
        let paths = Paths::new(root.to_path_buf(), Some(&root.join("target")));
        let tools = fixture_tools(root);
        let private_temp = fixture_private_temp(&paths);
        assert_eq!(
            format!("cargo {}", INVENTORY_ARGUMENTS.join(" ")),
            INVENTORY_COMMAND
        );
        assert_eq!(
            format!("cargo {}", DOCTEST_ARGUMENTS.join(" ")),
            DOCTEST_COMMAND
        );
        let inventory = inventory_command(&tools, &paths, &private_temp).unwrap();
        assert_eq!(
            inventory.get_program(),
            Path::new(&tools.cargo_mutants_path)
        );
        assert_eq!(
            inventory.get_args().collect::<Vec<_>>(),
            INVENTORY_ARGUMENTS
        );
        let doctest = doctest_command(&tools, &paths, &private_temp).unwrap();
        assert_eq!(doctest.get_program(), Path::new(&tools.cargo.path));
        assert_eq!(doctest.get_args().collect::<Vec<_>>(), DOCTEST_ARGUMENTS);
        let output = root.join("owned-output");
        let campaign = campaign_command(&tools, &paths, &output, &private_temp).unwrap();
        assert_eq!(campaign.get_program(), Path::new(&tools.cargo_mutants_path));
        let output_argument = crate::cargo::child_process_path(&output);
        let expected: Vec<_> = CAMPAIGN_ARGUMENTS_BEFORE_OUTPUT
            .iter()
            .map(std::ffi::OsStr::new)
            .chain(std::iter::once(output_argument.as_os_str()))
            .chain(
                CAMPAIGN_ARGUMENTS_AFTER_OUTPUT
                    .iter()
                    .map(std::ffi::OsStr::new),
            )
            .chain(std::iter::once(std::ffi::OsStr::new(
                CAMPAIGN_FINAL_ARGUMENT,
            )))
            .collect();
        assert_eq!(campaign.get_args().collect::<Vec<_>>(), expected);
        for command in [&campaign, &inventory, &doctest] {
            for (name, value) in [
                ("CARGO_BUILD_JOBS", "1"),
                ("NEXTEST_TEST_THREADS", "1"),
                ("RUSTC", tools.rustc.path.as_str()),
                ("RUSTDOC", tools.rustdoc.path.as_str()),
            ] {
                assert!(command.get_envs().any(|(observed, observed_value)| {
                    observed == name && observed_value == Some(std::ffi::OsStr::new(value))
                }));
            }
            let encoded = format!(
                "-C\u{1f}dlltool={}",
                crate::cargo::child_process_path(Path::new(&tools.dlltool.path)).display()
            );
            assert!(command.get_envs().any(|(observed, observed_value)| {
                observed == "CARGO_ENCODED_RUSTFLAGS"
                    && observed_value == Some(std::ffi::OsStr::new(&encoded))
            }));
            let private_temp = crate::cargo::child_process_path(&private_temp);
            for name in ["TEMP", "TMP", "TMPDIR"] {
                assert!(command.get_envs().any(|(observed, observed_value)| {
                    observed == name && observed_value == Some(private_temp.as_os_str())
                }));
            }
        }
        let campaign_path = campaign
            .get_envs()
            .find_map(|(name, value)| (name == "PATH").then_some(value).flatten())
            .unwrap();
        assert_eq!(
            std::env::split_paths(campaign_path).next().as_deref(),
            Path::new(&tools.nextest_path).parent()
        );
        let implementation = include_str!("../mutants_grade.rs");
        let execute = implementation.find("fn execute_campaign(").unwrap();
        let success = implementation[execute..]
            .find("fn require_campaign_success(")
            .unwrap()
            + execute;
        let body = &implementation[execute..success];
        assert!(body.contains("crate::bounded_process::output_with_limit("));
        assert!(!body.contains("command.output()"));
    });
}

#[test]
fn command_fixtures_do_not_inherit_the_compiling_checkout() {
    with_fixture_root(|root| {
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert!(!root.starts_with(checkout));
        let paths = Paths::new(root.to_path_buf(), Some(&root.join("target")));
        let tools = fixture_tools(root);
        let private_temp = fixture_private_temp(&paths);
        cargo_mutants_version_command(&tools, &paths, &private_temp).unwrap();
    });
}

#[test]
fn hostile_compiler_cargo_mutants_and_nextest_environment_is_removed() {
    let hostile = [
        "CARGO_BUILD_RUSTC_WRAPPER",
        "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTFLAGS",
        "RUSTDOCFLAGS",
        "RUSTUP_TOOLCHAIN",
        "TARGET",
        "NEXTEST_RUNNER",
        "NEXTEST_TEST_THREADS",
        "MUTANTS_TEST_TOOL",
        "CC",
        "LDFLAGS",
    ];
    let mut command = Command::new("fixture");
    for name in hostile {
        command.env(name, "attacker-controlled");
    }
    remove_ambient_execution_semantics(
        &mut command,
        hostile.into_iter().map(std::ffi::OsString::from),
    );
    for name in hostile {
        assert!(
            command
                .get_envs()
                .any(|(observed, value)| { observed == name && value.is_none() })
        );
    }
}

#[test]
fn parent_cargo_configuration_is_semantically_allowlisted() {
    let dlltool = r"C:\tools\dlltool.exe";
    let allowed = b"# compatibility only\n[target.x86_64-pc-windows-gnu]\nrustflags = [\"-C\", \"dlltool=C:\\\\tools\\\\dlltool.exe\"]\n";
    require_allowlisted_cargo_config_bytes(allowed, "x86_64-pc-windows-gnu", dlltool).unwrap();
    for hostile in [
        b"[build]\nrustc-wrapper = \"attacker\"".as_slice(),
        b"[target.x86_64-pc-windows-gnu]\nrunner = \"attacker\"".as_slice(),
        b"[env]\nRUSTC = { value = \"attacker\", force = true }".as_slice(),
        b"[target.x86_64-pc-windows-gnu]\nrustflags = [\"-C\", \"dlltool=C:\\\\other\\\\dlltool.exe\"]".as_slice(),
    ] {
        assert!(
            require_allowlisted_cargo_config_bytes(
                hostile,
                "x86_64-pc-windows-gnu",
                dlltool,
            )
            .is_err()
        );
    }
}

#[test]
fn owned_temp_root_controls_cargo_configuration_discovery() {
    let fixture = crate::test_support::reserve_temp_directory("wizard-mutants-temp-discovery");
    let repository = fixture.join("repository");
    let target = repository.join("target");
    crate::mutate::workspace::prepare_real_directory_tree(&target, "temp-discovery fixture target")
        .unwrap();
    let paths = Paths::new(repository.clone(), Some(&target));
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let (_, campaign) = reserve_campaign_directory(&paths, &source).unwrap();
    let private_temp = reserve_private_temp_directory(&paths, &campaign).unwrap();
    let tools = fixture_tools(&repository);
    let mut command = Command::new("fixture");
    for name in ["TEMP", "TMP", "TMPDIR"] {
        command.env(name, fixture.join("attacker-temp"));
    }
    apply_owned_environment(&mut command, &tools, &paths, &private_temp).unwrap();
    let pinned = crate::cargo::child_process_path(&private_temp);
    for name in ["TEMP", "TMP", "TMPDIR"] {
        assert!(
            command
                .get_envs()
                .any(|(observed, value)| { observed == name && value == Some(pinned.as_os_str()) })
        );
    }

    let hostile = target.join("qualification-runs/.cargo");
    std::fs::create_dir(&hostile).unwrap();
    std::fs::write(
        hostile.join("config.toml"),
        "[target.x86_64-pc-windows-gnu]\nrunner = \"attacker\"\n",
    )
    .unwrap();
    assert!(require_no_generated_cargo_config(&private_temp, &repository).is_err());
    std::fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn final_identity_check_follows_every_authoritative_tool_use_and_precedes_pass() {
    let implementation = include_str!("../mutants_grade.rs");
    let start = implementation.find("fn run_campaign(").unwrap();
    let end = implementation[start..]
        .find("pub(crate) fn verify_sealed(")
        .unwrap()
        + start;
    let body = &implementation[start..end];
    let inventory = body.find("capture_inventory(").unwrap();
    let doctest = body.find("capture_doctest(").unwrap();
    let outcomes = body.find("parse_outcomes(").unwrap();
    let adjudications = body.find("verify_adjudications(").unwrap();
    let exit = body.find("require_campaign_success(").unwrap();
    let identity = body.find("require_campaign_identity(").unwrap();
    let remeasure = body[identity..]
        .find("discover_tools(paths, &private_temp)?")
        .unwrap()
        + identity;
    let pass = body.find("attempt.pass_observed_mutants(").unwrap();
    assert!(inventory < identity);
    assert!(doctest < identity);
    assert!(outcomes < adjudications);
    assert!(adjudications < exit);
    assert!(exit < identity);
    assert!(identity < remeasure);
    assert!(remeasure < pass);
}

fn campaign_grade(missed: u64, timeout: u64, unviable: u64) -> Grade {
    let adjudicated = missed + timeout + unviable;
    Grade {
        total: 1 + adjudicated,
        caught: 1,
        missed,
        timeout,
        unviable,
        adjudicated,
    }
}

#[test]
fn campaign_exit_codes_match_adjudicated_outcomes() {
    for (missed, timeout, unviable, expected) in [
        (0, 0, 0, 0),
        (0, 0, 2, 0),
        (2, 0, 0, 2),
        (2, 0, 1, 2),
        (0, 2, 0, 3),
        (0, 2, 1, 3),
        (2, 1, 0, 3),
        (2, 1, 1, 3),
    ] {
        let grade = campaign_grade(missed, timeout, unviable);
        for code in [0, 1, 2, 3, 4, 5, 6, 70, 255] {
            assert_eq!(
                require_campaign_success(exit_status(code), &grade, Path::new("scratch")).is_ok(),
                code == expected,
                "exit {code} with {grade:?}"
            );
        }
    }
}

#[test]
fn campaign_exit_does_not_substitute_for_complete_adjudication() {
    for (missed, timeout, unviable, code) in [(1, 0, 0, 2), (0, 1, 0, 3), (0, 0, 1, 0)] {
        let mut grade = campaign_grade(missed, timeout, unviable);
        grade.adjudicated = 0;
        assert!(require_campaign_success(exit_status(code), &grade, Path::new("scratch")).is_err());
        grade.adjudicated = 2;
        assert!(require_campaign_success(exit_status(code), &grade, Path::new("scratch")).is_err());
    }
}

#[cfg(unix)]
#[test]
fn a_signaled_campaign_has_no_qualifying_exit_code() {
    use std::os::unix::process::ExitStatusExt;

    for grade in [
        campaign_grade(0, 0, 0),
        campaign_grade(1, 0, 0),
        campaign_grade(0, 1, 0),
    ] {
        assert!(
            require_campaign_success(ExitStatus::from_raw(9), &grade, Path::new("scratch"))
                .is_err()
        );
    }
}

#[test]
fn only_a_successful_unchanged_campaign_and_tracked_policy_can_pass() {
    assert!(
        require_campaign_success(
            exit_status(1),
            &campaign_grade(0, 0, 0),
            Path::new("scratch")
        )
        .is_err()
    );
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let changed_source = SourceIdentity {
        commit: "c".repeat(40),
        tree: source.tree.clone(),
    };
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let tools = fixture_tools(fixture_root);
    let changed_tools = ToolIdentity {
        nextest_bytes: 2,
        ..tools.clone()
    };
    assert!(require_campaign_identity(&source, &changed_source, &tools, &tools).is_err());
    assert!(require_campaign_identity(&source, &source, &tools, &changed_tools).is_err());
    require_source_policy(b"tracked", b"tracked").unwrap();
    assert!(require_source_policy(b"supplied", b"tracked").is_err());
}

#[test]
fn campaign_scratch_is_exclusively_reserved_under_target_and_removed() {
    let root = crate::test_support::reserve_temp_directory("wizard-owned-mutants");
    let paths = Paths::new(root.join("repository"), Some(&root.join("target")));
    let source = SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let (parent, directory) = reserve_campaign_directory(&paths, &source).unwrap();
    assert_eq!(directory.parent(), Some(parent.as_path()));
    assert!(parent.ends_with(Path::new("qualification-runs/cargo-mutants")));
    let nested = directory.join("nested");
    std::fs::create_dir(&nested).unwrap();
    assert!(remove_campaign_directory(&parent, &nested).is_err());
    std::fs::write(directory.join("owned"), "campaign").unwrap();
    remove_campaign_directory(&parent, &directory).unwrap();
    assert!(!directory.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_controlled_policy_is_utf8_without_a_byte_order_mark() {
    for (bytes, expected) in [
        (&[0xFF, 0xFE, b'{'][..], "UTF-16"),
        (&[0xFE, 0xFF, 0, b'{'][..], "UTF-16"),
        (&[0xEF, 0xBB, 0xBF, b'{'][..], "byte-order mark"),
    ] {
        let error = reject_byte_order_mark(bytes, "fixture").unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
    }
    reject_byte_order_mark(b"{}", "fixture").unwrap();
}
