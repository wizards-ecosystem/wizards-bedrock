use std::collections::BTreeMap;

use syn::visit::{self, Visit};

use super::*;

#[derive(Default)]
struct MachineCheckInventory {
    names: BTreeMap<String, usize>,
    unsupported: usize,
}

impl<'ast> Visit<'ast> for MachineCheckInventory {
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let report_receiver = matches!(
            node.receiver.as_ref(),
            syn::Expr::Path(path) if path.path.is_ident("report")
        );
        if node.method == "check" && report_receiver {
            match node.args.first() {
                Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(name),
                    ..
                })) if node.args.len() == 2 => {
                    *self.names.entry(name.value()).or_default() += 1;
                }
                _ => self.unsupported += 1,
            }
        }
        visit::visit_expr_method_call(self, node);
    }
}

fn machine_checks(source: &str) -> MachineCheckInventory {
    let syntax = syn::parse_file(source).expect("parsing machine self-test source");
    let mut inventory = MachineCheckInventory::default();
    inventory.visit_file(&syntax);
    inventory
}

#[test]
#[ignore]
fn parked_host_mutation_child() {
    loop {
        std::thread::park();
    }
}

#[test]
fn each_host_mutation_witness_gets_a_new_empty_temporary_directory() {
    let root = std::env::current_dir().expect("current directory");
    let target = root.join("target/mutation-temporary-fixture");
    std::fs::create_dir_all(&target).expect("temporary fixture target");
    let target = std::fs::canonicalize(&target).expect("canonical fixture target");
    let paths = Paths::new(root.clone(), Some(&target));
    let mut seen = Vec::new();
    for _ in 0..2 {
        let error = host_test_reports_with(&paths, "xtask", "unused", |command, _, _| {
            let envs = command.get_envs().collect::<BTreeMap<_, _>>();
            let temporary = envs[std::ffi::OsStr::new("TEMP")].expect("TEMP is set");
            for name in ["TMP", "TMPDIR"] {
                assert_eq!(envs[std::ffi::OsStr::new(name)], Some(temporary));
            }
            let temporary = std::fs::canonicalize(temporary).expect("temporary exists");
            assert!(temporary.starts_with(&target));
            assert!(
                std::fs::read_dir(&temporary)
                    .expect("temporary is a directory")
                    .next()
                    .is_none()
            );
            seen.push(temporary);
            Err(anyhow::anyhow!("stopped before Cargo"))
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("stopped before Cargo"));
    }
    assert_ne!(seen[0], seen[1]);
}

#[test]
fn a_parked_host_mutation_is_inconclusive_not_caught() {
    let root = std::env::current_dir().expect("current directory");
    let paths = Paths::new(
        root.clone(),
        Some(&root.join("target/mutation-timeout-fixture")),
    );
    std::fs::create_dir_all(paths.target_dir()).expect("timeout fixture target");
    let executable = std::env::current_exe().expect("current test executable");
    let started = std::time::Instant::now();
    let error = host_test_reports_with(
        &paths,
        "xtask",
        "mutate::tests::parked_host_mutation_child",
        |command, timeout, label| {
            assert_eq!(command.get_current_dir(), Some(root.as_path()));
            assert_eq!(timeout, HOST_TEST_TIMEOUT);
            assert_eq!(
                command.get_args().collect::<Vec<_>>(),
                [
                    "test",
                    "--package",
                    "xtask",
                    "mutate::tests::parked_host_mutation_child",
                    "--",
                    "--exact",
                ]
                .map(std::ffi::OsStr::new)
            );
            let mut parked = Command::new(&executable);
            parked.args([
                "--exact",
                "mutate::tests::parked_host_mutation_child",
                "--ignored",
            ]);
            crate::bounded_process::output(parked, Duration::from_millis(50), label)
        },
    )
    .unwrap_err();

    assert!(
        format!("{error:#}").contains("timed out after 50 ms"),
        "{error:#}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn api_audit_mutations_require_the_current_structured_dead_item_verdict() {
    let current =
        "1 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1";
    assert!(crate::api::report_names_exact_dead_item(
        current,
        "fn is_relative"
    ));
    assert!(!crate::api::report_names_exact_dead_item(
        &current.replace("fn is_relative at", "fn is_relative_helper at"),
        "fn is_relative"
    ));

    for rejected in [
        "1 public API item(s) have no production caller (0 verification-only, 1 with no compiled caller):\n  fn is_relative at libs/path-walk/src/lib.rs:1 [no compiled caller]",
        "1 forged API-audit summary:\n  fn is_relative at libs/path-walk/src/lib.rs:1",
        "1 public API item(s) have no caller: fn is_relative at libs/path-walk/src/lib.rs:1",
        "2 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1\n  fn other at libs/path-walk/src/lib.rs:2",
        "1 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1\n  fn other at libs/path-walk/src/lib.rs:2",
        "0 public API item(s) have no caller:",
        "1 public API item(s) have no caller:\n  fn is_relative at libs/path-walk/src/lib.rs:1 [no compiled caller]",
        "1 public API item(s) have no caller:\n  fn is_relative at ",
        "1 public API item(s) have no caller:\nfn is_relative at libs/path-walk/src/lib.rs:1",
    ] {
        assert!(
            !crate::api::report_names_exact_dead_item(rejected, "fn is_relative"),
            "accepted malformed or stale API-audit output: {rejected}"
        );
    }
}

#[test]
fn api_audit_commands_rebind_original_and_copied_sources_before_verdicts() {
    let source = concat!(
        include_str!("../api/catalog.rs"),
        include_str!("../api/compile.rs")
    );
    let metadata = source
        .split_once("impl CargoCatalog {")
        .and_then(|(_, source)| source.split_once("    pub(super) fn parse("))
        .map(|(source, _)| source)
        .expect("Cargo metadata command body");
    let compilation = source
        .split_once("fn compile_graph(")
        .and_then(|(_, source)| source.split_once("fn cargo_arguments("))
        .map(|(source, _)| source)
        .expect("compiler graph command body");

    for (label, command, verdict) in [
        ("Cargo metadata", metadata, "ensure!("),
        (
            "compiler graph",
            compilation,
            "if !captured.status.success()",
        ),
    ] {
        let output = command
            .find("bounded_process::output_")
            .expect("command completion");
        let copied = command
            .find("workspace.require_copy_unchanged(expected)?;")
            .expect("copied-source freshness guard");
        let original = command
            .find("workspace.require_source_unchanged(&workspace.source_root)?;")
            .expect("original-source freshness guard");
        let verdict = command.find(verdict).expect("command verdict");
        assert!(
            output < copied && copied < original && original < verdict,
            "{label} output was interpreted before both source trees were rebound"
        );
    }
}

fn rust_sources_contain(directory: &std::path::Path, needle: &str) -> bool {
    crate::paths::directory_entries(directory)
        .unwrap_or_else(|reason| {
            panic!(
                "complete directory listing for {}: {reason}",
                directory.display()
            )
        })
        .into_iter()
        .any(|entry| {
            let path = entry.path();
            if entry.is_dir() {
                rust_sources_contain(&path, needle)
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                std::fs::read_to_string(&path)
                    .unwrap_or_else(|_| panic!("reading {}", path.display()))
                    .contains(needle)
            } else {
                false
            }
        })
}

/// A pattern that stops matching silently tests nothing, so this cheap check runs in the gate.
#[test]
fn every_mutation_still_applies() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");

    let mut stale = Vec::new();
    for mutation in mutations() {
        let path = root.join(mutation.file);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("reading {}", path.display()));
        let found = text.matches(mutation.find).count();
        if found != 1 {
            stale.push(format!(
                "{}: `{}` occurs {found} times",
                mutation.file, mutation.breaks
            ));
        }
        assert_ne!(mutation.find, mutation.replace, "{}", mutation.breaks);
    }
    assert!(
        stale.is_empty(),
        "mutation patterns without one current source anchor:\n{}",
        stale.join("\n")
    );
}

/// Mutating the driver cannot change the binary already building images.
#[test]
fn a_mutation_cannot_target_the_running_driver() {
    for mutation in mutations() {
        require_target_is_rebuilt(mutation).unwrap_or_else(|error| panic!("{error}"));
    }

    let mut impossible = *mutations()
        .find(|mutation| {
            matches!(
                mutation.reddens,
                Caught::HostTest {
                    package: "xtask",
                    ..
                }
            )
        })
        .unwrap();
    impossible.file = "xtask/src/image/program.rs";
    impossible.reddens = Caught::Check("unused");
    let error = require_target_is_rebuilt(&impossible).expect_err("the driver is already running");
    assert!(error.to_string().contains("would not be rebuilt"));
}

/// Read from source, so no recursive Cargo run or machine boot is paid for.
#[test]
fn every_named_check_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root");
    let selftest =
        std::fs::read_to_string(root.join("kernel/src/selftest.rs")).expect("reading selftest.rs");
    let checks = machine_checks(&selftest);
    assert_eq!(
        checks.unsupported, 0,
        "every machine report.check name must be one exact source literal"
    );
    for (name, count) in &checks.names {
        assert_eq!(
            *count, 1,
            "machine check `{name}` is registered {count} times"
        );
    }
    let stale = mutations()
        .filter_map(|mutation| match mutation.reddens {
            Caught::Check(name) if checks.names.get(name) != Some(&1) => {
                Some(format!("{} -> {name}", mutation.breaks))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "machine mutations without one current report.check:\n{}",
        stale.join("\n")
    );

    for mutation in mutations() {
        match mutation.reddens {
            Caught::Check(_) => {}
            Caught::HostTest { package, name } => {
                assert!(
                    crate::paths::HOST_PACKAGES.contains(&package),
                    "`{}` names non-host package `{package}`",
                    mutation.breaks
                );
                let function = name
                    .rsplit_once("::")
                    .map_or(name, |(_, function)| function);
                let package = if package == "xtask" {
                    root.join(package)
                } else {
                    root.join("libs").join(package)
                };
                assert!(
                    rust_sources_contain(&package, &format!("fn {function}(")),
                    "`{}` names host test `{name}`, which package `{}` does not have",
                    mutation.breaks,
                    package.display()
                );
            }
            Caught::ApiAudit(item) => {
                assert!(!item.is_empty(), "API-audit mutations name an exact item");
            }
            Caught::BootMarker(marker) => {
                let qemu = std::fs::read_to_string(root.join("xtask/src/qemu.rs"))
                    .expect("reading qemu.rs");
                assert!(
                    qemu.contains(marker),
                    "`{}` names boot marker `{marker}`, which qemu.rs does not have",
                    mutation.breaks
                );
            }
            Caught::DegradedPanic { .. } | Caught::Panic { .. } | Caught::Timeout { .. } => {}
        }
    }
}

#[test]
fn machine_check_inventory_rejects_decoys_and_nonliteral_names() {
    let source = r#"
            fn observe(report: &mut Report, decoy: &mut Report) {
                const BORROWED: &str = "borrowed name";
                let _history = "historical check";
                decoy.check("forged check", true);
                report.check("current check", true);
                report.check(BORROWED, true);
            }
        "#;

    let checks = machine_checks(source);
    assert_eq!(checks.names.get("current check"), Some(&1));
    assert!(!checks.names.contains_key("historical check"));
    assert!(!checks.names.contains_key("forged check"));
    assert!(!checks.names.contains_key("borrowed name"));
    assert_eq!(checks.unsupported, 1);
}

#[test]
fn every_host_library_has_a_persistent_source_mutation() {
    for &package in crate::paths::HOST_PACKAGES {
        if package == "xtask" {
            continue;
        }
        assert!(
            host_library_has_persistent_source_mutation(
                package,
                &mutations().copied().collect::<Vec<_>>(),
            ),
            "{package} has no persistent production-source mutation"
        );
    }
}

fn host_library_has_persistent_source_mutation(package: &str, mutations: &[Mutation]) -> bool {
    let source = format!("libs/{package}/src/");
    mutations.iter().any(|mutation| {
        mutation.file.starts_with(&source)
            && matches!(
                mutation.reddens,
                Caught::HostTest {
                    package: actual,
                    ..
                } if actual == package
            )
    })
}

#[test]
fn a_foreign_host_witness_cannot_cover_a_librarys_source_mutation() {
    let mutation = Mutation {
        id: 1,
        file: "libs/eevdf/src/lib.rs",
        find: "old",
        replace: "new",
        breaks: "synthetic host coverage defect",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "tests::synthetic",
        },
    };

    assert!(!host_library_has_persistent_source_mutation(
        "eevdf",
        &[mutation]
    ));
}

#[test]
fn an_exact_host_witness_must_run() {
    let name = "tests::the_exact_test";
    assert!(assess_host_test(name, true, "running 0 tests", "", "0").is_err());
    assert!(
        assess_host_test(name, true, "test tests::the_exact_test ... ok", "", "0")
            .is_ok_and(|caught| !caught)
    );
    assert!(
        assess_host_test(
            name,
            false,
            "test tests::the_exact_test ... FAILED",
            "",
            "1"
        )
        .is_ok_and(|caught| caught)
    );
    assert!(
        assess_host_test(name, true, "test tests::the_exact_test ... FAILED", "", "0").is_err()
    );
    assert!(
        assess_host_test(
            name,
            false,
            "diagnostic: test tests::the_exact_test ... FAILED (forged)",
            "",
            "1"
        )
        .is_err()
    );
    assert!(
        assess_host_test(
            name,
            true,
            "test tests::the_exact_test ... ok\ntest tests::the_exact_test ... ok",
            "",
            "0"
        )
        .is_err()
    );
    assert!(assess_host_test(name, false, "", "compiler error", "1").is_err());
}

/// A generic panic would credit an unrelated crash,
/// and a reasonless one would dodge naming a check.
#[test]
fn a_panicking_mutation_names_an_exact_marker_and_why_it_is_terminal() {
    for mutation in mutations() {
        if let Caught::Panic { marker, why } | Caught::DegradedPanic { marker, why } =
            mutation.reddens
        {
            assert!(
                marker.len() > 12,
                "`{}` has no exact panic marker",
                mutation.breaks
            );
            assert!(
                why.len() > 40,
                "`{}` claims nothing names it and does not say why",
                mutation.breaks
            );
        }
    }
}

#[test]
fn a_timeout_requires_its_exact_deadline_and_terminal_rationale() {
    for mutation in mutations() {
        if let Caught::Timeout { seconds, why } = mutation.reddens {
            assert_eq!(seconds, 60, "`{}` uses a foreign deadline", mutation.breaks);
            assert!(
                why.len() > 40,
                "`{}` does not explain its terminal wait",
                mutation.breaks
            );
        }
    }

    let wanted = Caught::Timeout {
        seconds: 60,
        why: "this fixture explains why the machine cannot reach an ordinary terminal verdict",
    };
    assert!(
        assess_machine_error(
            wanted,
            anyhow::anyhow!("self-test did not finish within 60 seconds")
        )
        .is_ok_and(|caught| caught)
    );
    assert!(
        assess_machine_error(
            wanted,
            anyhow::anyhow!("self-test did not finish within 59 seconds")
        )
        .is_err()
    );
    assert!(assess_machine_error(wanted, anyhow::anyhow!("the emulator disappeared")).is_err());
    assert_eq!(machine_observation_timeout(wanted), Duration::from_secs(60));
    assert_eq!(
        machine_observation_timeout(Caught::Check("fixture check")),
        tasks::MUTATION_SELFTEST_TIMEOUT
    );
    assert_eq!(
        machine_observation_timeout(Caught::Panic {
            marker: "fixture panic",
            why: "a sufficiently detailed fixture rationale for the panic witness",
        }),
        tasks::MUTATION_SELFTEST_TIMEOUT
    );
    assert!(
        assess_selftest(
            wanted,
            qemu::SelftestOutcome::Failed(vec!["another check".to_string()])
        )
        .is_err()
    );
}

#[test]
fn a_red_baseline_stops_before_any_mutation() {
    let error = require_green("self-test", Err(anyhow::anyhow!("already red")))
        .expect_err("a red baseline cannot be accepted");
    assert!(format!("{error:#}").contains("pristine self-test mutation baseline"));
}

#[test]
fn a_wrong_machine_failure_is_inconclusive_not_caught() {
    let wanted = Caught::Check("the intended check");
    assert!(
        assess_selftest(
            wanted,
            qemu::SelftestOutcome::Failed(vec!["another check".to_string()])
        )
        .is_err()
    );
    assert!(
        assess_selftest(
            wanted,
            qemu::SelftestOutcome::Panicked {
                transcript: "unrelated panic".to_string(),
                failed: Vec::new(),
            }
        )
        .is_err()
    );
    assert!(assess_selftest(wanted, qemu::SelftestOutcome::Passed).is_ok_and(|caught| !caught));
    assert!(
        assess_selftest(
            wanted,
            qemu::SelftestOutcome::Failed(vec!["the intended check".to_string()])
        )
        .is_ok_and(|caught| caught)
    );
    assert!(
        assess_selftest(
            wanted,
            qemu::SelftestOutcome::Panicked {
                transcript: "later corruption".to_string(),
                failed: vec!["the intended check".to_string()],
            }
        )
        .is_ok_and(|caught| caught)
    );
}

#[test]
fn a_panic_requires_its_exact_marker() {
    let wanted = Caught::Panic {
        marker: "the exact cause",
        why: "this fixture explains why no ordinary check can report the terminal defect",
    };
    assert!(
        assess_selftest(
            wanted,
            qemu::SelftestOutcome::Panicked {
                transcript: "a different panic".to_string(),
                failed: Vec::new(),
            }
        )
        .is_err()
    );
    assert!(
        assess_selftest(
            wanted,
            qemu::SelftestOutcome::Panicked {
                transcript: "panic: the exact cause".to_string(),
                failed: Vec::new(),
            }
        )
        .is_ok_and(|caught| caught)
    );
}

#[test]
fn a_degraded_panic_requires_its_diagnosis_and_exact_marker() {
    let wanted = Caught::DegradedPanic {
        marker: "the exact degraded cause",
        why: "this terminal branch exists only before the degraded kernel can report a verdict",
    };
    assert!(
        assess_degraded_panic(
            wanted,
            qemu::DegradedOutcome::Panicked {
                transcript: "KERNEL PANIC: the exact degraded cause".to_string(),
                no_x2apic_diagnosed: false,
            }
        )
        .is_err()
    );
    assert!(
        assess_degraded_panic(
            wanted,
            qemu::DegradedOutcome::Panicked {
                transcript: "KERNEL PANIC: another cause".to_string(),
                no_x2apic_diagnosed: true,
            }
        )
        .is_err()
    );
    assert!(
        assess_degraded_panic(
            wanted,
            qemu::DegradedOutcome::Panicked {
                transcript: "KERNEL PANIC: the exact degraded cause".to_string(),
                no_x2apic_diagnosed: true,
            }
        )
        .is_ok_and(|caught| caught)
    );
    assert!(
        assess_degraded_panic(wanted, qemu::DegradedOutcome::Settled).is_ok_and(|caught| !caught)
    );
}

#[test]
fn an_inconclusive_machine_observation_gets_one_bounded_retry() {
    let wanted = Caught::Check("the intended check");
    let mut observations = 0;
    let caught = qualify_machine_mutation(wanted, || {
        observations += 1;
        Ok(if observations == 1 {
            qemu::SelftestOutcome::Panicked {
                transcript: "incidental panic".to_string(),
                failed: Vec::new(),
            }
        } else {
            qemu::SelftestOutcome::Failed(vec!["the intended check".to_string()])
        })
    })
    .expect("the exact retry is causal");
    assert!(caught);
    assert_eq!(observations, 2);

    observations = 0;
    let error = qualify_machine_mutation(wanted, || {
        observations += 1;
        Ok(qemu::SelftestOutcome::Panicked {
            transcript: "incidental panic".to_string(),
            failed: Vec::new(),
        })
    })
    .expect_err("repeated inconclusive observations still fail qualification");
    assert_eq!(observations, 2);
    assert!(error.to_string().contains("after 2 observations"));
}

/// A duplicate spends a witness run twice on one question, and a machine entry's run is a boot.
#[test]
fn no_two_mutations_name_the_same_property() {
    for (index, mutation) in mutations().enumerate() {
        for earlier in mutations().take(index) {
            assert_ne!(earlier.breaks, mutation.breaks, "{}", mutation.breaks);
        }
    }
}

/// Documents cite identities, so one may neither repeat nor return after its mutation is deleted.
#[test]
fn every_mutation_identity_is_permanent_and_accounted_for() {
    let mut seen = BTreeMap::new();
    for mutation in mutations() {
        assert!(
            (1..NEXT_MUTATION_ID).contains(&mutation.id),
            "#{} for `{}` is outside the issued identities",
            mutation.id,
            mutation.breaks
        );
        if let Some(earlier) = seen.insert(mutation.id, mutation.breaks) {
            panic!(
                "#{} names both `{earlier}` and `{}`",
                mutation.id, mutation.breaks
            );
        }
    }
    let mut issued = seen
        .keys()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for retired in RETIRED_MUTATION_IDS {
        assert!(
            issued.insert(*retired),
            "#{retired} is retired twice or still registered"
        );
    }
    assert!(
        issued.iter().copied().eq(1..NEXT_MUTATION_ID),
        "every issued identity must be registered or retired exactly once"
    );
}

#[test]
fn a_pattern_that_matches_once_is_applied_and_others_are_refused() {
    let mutation = Mutation {
        id: 1,
        file: "x",
        find: "keep",
        replace: "drop",
        breaks: "a thing",
        reddens: Caught::Check("a thing goes red"),
    };
    assert_eq!(apply("a keep b", &mutation).expect("once"), "a drop b");
    assert!(apply("nothing here", &mutation).is_err());
    assert!(apply("keep and keep", &mutation).is_err());
}
