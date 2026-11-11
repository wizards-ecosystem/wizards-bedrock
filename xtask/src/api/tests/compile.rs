use super::*;

#[test]
fn compiler_queries_remove_inherited_loader_variables() {
    let query = compiler_query(Path::new("rustc"), Path::new("."));
    let environment = query.get_envs().collect::<BTreeMap<_, _>>();
    for loader in ["LD_PRELOAD", "LD_LIBRARY_PATH"] {
        assert_eq!(
            environment.get(std::ffi::OsStr::new(loader)),
            Some(&None),
            "{loader} reached the compiler query"
        );
    }
}

#[test]
fn compiler_graphs_have_distinct_stable_build_directories() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-graph-build");
    let build = root.join("build");
    std::fs::create_dir_all(&build).expect("graph build root");
    let graph_zero = graph_build_directory(&build, 0).expect("graph zero build");
    let graph_one = graph_build_directory(&build, 1).expect("graph one build");
    let graph_one_again = graph_build_directory(&build, 1).expect("stable graph one build");
    std::fs::remove_dir_all(root).expect("remove graph build fixture");

    assert!(
        graph_zero != graph_one,
        "distinct compiler graphs shared incremental state"
    );
    assert!(
        graph_one == graph_one_again,
        "one compiler graph changed build identity between rounds"
    );
}

#[test]
fn instrumented_rounds_cannot_change_build_script_output_authority() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-build-authority");
    let package = root.join("debug/build/fixture-build-id");
    let out_dir = package.join("out");
    std::fs::create_dir_all(&out_dir).expect("build-script OUT_DIR fixture");
    std::fs::write(package.join("output"), "cargo:rerun-if-changed=fixed\n")
        .expect("build-script output fixture");
    let generated = out_dir.join("generated.rs");
    std::fs::write(&generated, "pub const VALUE: usize = 1;\n").expect("generated source fixture");

    let baseline = build_script_authority_snapshot(&root).expect("pristine authority");
    require_build_script_authority_unchanged(&root, &baseline, "fixture graph")
        .expect("unchanged authority");
    std::fs::write(&generated, "pub const VALUE: usize = 2;\n")
        .expect("marker-sensitive generated source");
    let error = require_build_script_authority_unchanged(&root, &baseline, "fixture graph")
        .expect_err("changed OUT_DIR source must be refused");
    assert!(format!("{error:#}").contains("changed build-script output"));

    std::fs::remove_dir_all(root).expect("remove build-authority fixture");
}

#[test]
fn pristine_generated_source_cannot_mint_a_diagnostic_marker() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-generated-marker");
    let out = root.join("debug/build/fixture-build-id/out");
    std::fs::create_dir_all(&out).expect("generated marker OUT_DIR");
    std::fs::write(
        out.join("generated.rs"),
        "#[deprecated(note = \"wizard-api-audit:\\x30\")]\npub fn forged() {}\n",
    )
    .expect("marker-sensitive generated source");

    let error = build_script_authority_snapshot(&root)
        .expect_err("decoded marker in pristine OUT_DIR must be refused");
    assert!(
        format!("{error:#}").contains("reserved diagnostic marker"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).expect("remove generated marker fixture");
}

#[test]
fn compiler_binaries_and_target_metadata_cannot_mint_a_diagnostic_marker() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-sysroot-marker");
    let bin = root.join("bin");
    let host_lib = root.join("lib/rustlib/fixture-host/lib");
    let kernel_lib = root.join("lib/rustlib/fixture-kernel/lib");
    let codegen = root.join("lib/rustlib/fixture-host/codegen-backends/fixture.dll");
    std::fs::create_dir_all(&bin).expect("compiler fixture bin");
    std::fs::create_dir_all(&host_lib).expect("host sysroot fixture");
    std::fs::create_dir_all(&kernel_lib).expect("kernel sysroot fixture");
    std::fs::create_dir_all(codegen.parent().expect("codegen backend parent"))
        .expect("codegen backend fixture directory");
    let cargo = bin.join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
    let compiler = bin.join(if cfg!(windows) { "rustc.exe" } else { "rustc" });
    let metadata = host_lib.join("libcore-fixture.rmeta");
    std::fs::write(&cargo, b"fixed cargo").expect("fixture Cargo");
    std::fs::write(&compiler, b"fixed rustc").expect("fixture rustc");
    std::fs::write(&metadata, b"fixed metadata").expect("fixture host metadata");
    std::fs::write(&codegen, b"fixed codegen backend").expect("fixture codegen backend");
    std::fs::write(
        kernel_lib.join("libcore-fixture.rlib"),
        b"fixed kernel library",
    )
    .expect("fixture kernel library");
    let targets = vec!["fixture-host".to_string(), "fixture-kernel".to_string()];
    let baseline = compiler_input_snapshot(&root, &cargo, &compiler, &targets)
        .expect("marker-free compiler authority");

    for (label, path, original) in [
        (
            "compiler binary",
            compiler.as_path(),
            b"fixed rustc".as_slice(),
        ),
        (
            "precompiled metadata",
            metadata.as_path(),
            b"fixed metadata".as_slice(),
        ),
        (
            "codegen backend",
            codegen.as_path(),
            b"fixed codegen backend".as_slice(),
        ),
    ] {
        let mut hostile = format!("hostile {MARKER}0").into_bytes();
        hostile.push(0xff);
        std::fs::write(path, hostile).expect("inject compiler marker");
        let error = compiler_input_snapshot(&root, &cargo, &compiler, &targets).expect_err(label);
        assert!(
            format!("{error:#}").contains("reserved diagnostic marker"),
            "{label}: {error:#}"
        );
        std::fs::write(path, original).expect("restore compiler input");
    }

    std::fs::write(&codegen, b"changed codegen backend").expect("change codegen backend");
    let current = compiler_input_snapshot(&root, &cargo, &compiler, &targets)
        .expect("changed marker-free codegen authority");
    let error = require_compiler_snapshot_unchanged(&baseline, &current)
        .expect_err("changed codegen backend must be refused");
    assert!(
        format!("{error:#}")
            .replace('\\', "/")
            .contains("codegen-backends/fixture.dll"),
        "{error:#}"
    );
    std::fs::write(&codegen, b"fixed codegen backend").expect("restore codegen backend");

    std::fs::write(&metadata, b"changed metadata").expect("change compiler input");
    let current = compiler_input_snapshot(&root, &cargo, &compiler, &targets)
        .expect("changed marker-free compiler authority");
    let error = require_compiler_snapshot_unchanged(&baseline, &current)
        .expect_err("changed compiler input must be refused");
    assert!(
        format!("{error:#}")
            .replace('\\', "/")
            .contains("libcore-fixture.rmeta"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).expect("remove compiler marker fixture");
}

#[test]
fn compiler_graphs_are_offline_lockfile_frozen_and_use_compact_structured_diagnostics() {
    const PACKAGES: &[&str] = &["fixture"];
    let pass = gate::CargoPass {
        name: "fixture",
        packages: PACKAGES,
        target: None,
        features: None,
    };
    let plan = CompilationPlan {
        pass,
        target: "fixture-host".to_string(),
        kind: RootKind::Production,
        mode: CompilationMode::RegisteredPass,
    };
    let args = cargo_arguments(&plan);

    assert_eq!(
        &args[..5],
        [
            "check",
            "--jobs=1",
            "--offline",
            "--locked",
            "--message-format=json-diagnostic-short"
        ]
    );
    assert_eq!(args.iter().filter(|arg| *arg == "--locked").count(), 1);
    assert!(
        args.windows(2)
            .any(|args| args == ["--target", "fixture-host"])
    );
}

#[test]
fn compiler_graphs_are_serialized_at_the_owned_cargo_boundary() {
    const PACKAGES: &[&str] = &["fixture"];
    let pass = gate::CargoPass {
        name: "fixture",
        packages: PACKAGES,
        target: None,
        features: None,
    };
    let plan = CompilationPlan {
        pass,
        target: "fixture-host".to_string(),
        kind: RootKind::Production,
        mode: CompilationMode::RegisteredPass,
    };

    assert_eq!(
        cargo_arguments(&plan)
            .iter()
            .filter(|argument| argument.as_str() == "--jobs=1")
            .count(),
        1,
        "each API-audit Cargo graph must own one single-job scheduling bound"
    );
}

#[test]
fn concurrent_graphs_stay_bounded_ordered_and_isolate_the_alone_graph() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

    const WORKERS: usize = 3;
    const ALONE: usize = 5;
    let plans = (0..12)
        .map(|index| CompilationPlan {
            pass: gate::CargoPass {
                name: "fixture",
                packages: &["fixture"],
                target: None,
                features: None,
            },
            target: "fixture-host".to_string(),
            kind: RootKind::Verification,
            mode: CompilationMode::TestHarness(CargoTarget {
                package: if index == ALONE { "xtask" } else { "fixture" }.to_string(),
                package_id: format!("fixture-{index}"),
                name: format!("harness-{index}"),
                relative: PathBuf::from("src/lib.rs"),
                kind: CargoTargetKind::Library,
                test: true,
                executable: false,
            }),
        })
        .collect::<Vec<_>>();
    let running = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let arrived = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let met = AtomicBool::new(true);
    let alone_ran = AtomicBool::new(false);
    let alone_shared = AtomicBool::new(false);
    let passing = run_bounded_in_order(
        7,
        WORKERS,
        |index| compiles_alone(&plans[index]),
        |index| {
            let now = running.fetch_add(1, SeqCst) + 1;
            peak.fetch_max(now, SeqCst);
            if index == ALONE {
                alone_ran.store(true, SeqCst);
                alone_shared.store(now != 1 || finished.load(SeqCst) != 6, SeqCst);
            }
            if index < WORKERS {
                arrived.fetch_add(1, SeqCst);
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while arrived.load(SeqCst) < WORKERS {
                    if std::time::Instant::now() >= deadline {
                        met.store(false, SeqCst);
                        break;
                    }
                    std::thread::yield_now();
                }
            }
            std::thread::sleep(Duration::from_millis(20));
            running.fetch_sub(1, SeqCst);
            finished.fetch_add(1, SeqCst);
            Ok(index * 10)
        },
    )
    .expect("every graph passes");
    assert_eq!(passing, vec![0, 10, 20, 30, 40, 50, 60]);
    assert!(met.load(SeqCst), "the first graphs never ran together");
    assert_eq!(peak.load(SeqCst), WORKERS, "the worker bound changed");
    assert!(alone_ran.load(SeqCst), "the alone graph never ran");
    assert!(
        !alone_shared.load(SeqCst),
        "the alone graph shared the machine"
    );

    let started = AtomicUsize::new(0);
    let completed = AtomicUsize::new(0);
    let error = run_bounded_in_order(
        12,
        WORKERS,
        |index| compiles_alone(&plans[index]),
        |index| {
            started.fetch_add(1, SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            completed.fetch_add(1, SeqCst);
            if index == 1 || index == 2 {
                bail!("graph {index} failed");
            }
            Ok(index)
        },
    )
    .expect_err("a failed graph fails the round");
    assert_eq!(error.to_string(), "graph 1 failed");
    assert_eq!(
        started.load(SeqCst),
        WORKERS,
        "later graphs started after failure"
    );
    assert_eq!(
        completed.load(SeqCst),
        WORKERS,
        "admitted graphs were not joined"
    );
}

#[test]
fn streamed_cargo_records_are_bounded_individually() {
    let record = b"{\"reason\":\"build-finished\",\"success\":true}\n";
    let input = record.repeat(32);
    cargo_compilation_stream(
        std::io::BufReader::new(std::io::Cursor::new(input)),
        None,
        record.len(),
    )
    .expect("aggregate non-authoritative records need no whole-output capture")
    .finish()
    .expect("record-only stream");

    let oversized = format!("{{\"reason\":\"{}\"}}\n", "x".repeat(64));
    let error = cargo_compilation_stream(
        std::io::BufReader::new(std::io::Cursor::new(oversized)),
        None,
        32,
    )
    .expect_err("one oversized record must be refused");
    assert!(format!("{error:#}").contains("record exceeded 32 bytes"));

    let error = cargo_compilation_stream(
        std::io::BufReader::new(std::io::Cursor::new(
            b"{\"reason\":\"build-finished\",\"success\":true}".as_slice(),
        )),
        None,
        128,
    )
    .expect_err("unterminated Cargo JSON must be refused");
    assert!(format!("{error:#}").contains("without a record terminator"));
}

#[test]
fn retained_marker_evidence_has_aggregate_count_and_byte_bounds() {
    let marker = MarkerDiagnostic {
        id: 7,
        origin: CargoDiagnosticOrigin {
            package_id: "fixture 0.1.0 (path+file:///fixture)".to_string(),
            target_name: "fixture".to_string(),
            target_kind: CargoTargetKind::Library,
            target_source: PathBuf::from("/fixture/src/lib.rs"),
        },
        reported: "/wizard/source/libs/fixture/src/lib.rs".to_string(),
        line: 1,
        column: 1,
        immediate_expansion: None,
    };
    let marker_bytes = marker_retained_bytes(&marker).expect("bounded marker size");

    let mut count_bounded = CargoCompilationEvidence::new(None);
    count_bounded
        .retain_marker_with_limits(marker.clone(), 1, usize::MAX)
        .expect("first marker within count bound");
    let error = count_bounded
        .retain_marker_with_limits(marker.clone(), 1, usize::MAX)
        .expect_err("second marker must cross the count bound");
    assert!(format!("{error:#}").contains("exceeded 1 records"));

    let mut byte_bounded = CargoCompilationEvidence::new(None);
    let error = byte_bounded
        .retain_marker_with_limits(marker, usize::MAX, marker_bytes - 1)
        .expect_err("one marker must cross the aggregate byte bound");
    assert!(format!("{error:#}").contains(&format!("exceeded {} bytes", marker_bytes - 1)));
}

#[test]
fn compiler_graphs_never_merge_distinct_cargo_targets() {
    let source = PRODUCTION;
    let compilation = source
        .split_once("fn compile_graphs(")
        .and_then(|(_, source)| source.split_once("#[cfg(test)]"))
        .map(|(source, _)| source)
        .expect("API compilation implementation");
    assert!(
        !compilation.contains("--all-targets"),
        "mutually exclusive Cargo targets shared one evidence graph"
    );
}

#[test]
fn cargo_test_artifacts_must_prove_the_standard_harness() {
    let selected = CargoTarget {
        package: "fixture".to_string(),
        package_id: "fixture-id".to_string(),
        name: "fixture_core".to_string(),
        relative: PathBuf::from("src/lib.rs"),
        kind: CargoTargetKind::Library,
        test: true,
        executable: false,
    };
    let artifact = |test| {
        format!(
            "{{\"reason\":\"compiler-artifact\",\"package_id\":\"fixture-id\",\"target\":{{\"name\":\"fixture_core\",\"kind\":[\"lib\"]}},\"profile\":{{\"test\":{test}}}}}\n"
        )
    };
    cargo_test_output(artifact("true").as_bytes(), &selected).expect("standard harness artifact");
    let error = cargo_test_output(artifact("false").as_bytes(), &selected)
        .expect_err("custom harness must be refused");
    assert!(format!("{error:#}").contains("custom test harness"));
}

#[test]
fn every_host_target_mode_receives_its_own_compiler_graph() {
    let mut packages = crate::paths::HOST_PACKAGES
        .iter()
        .map(|package| ((*package).to_string(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    let package = crate::paths::HOST_PACKAGES[0];
    let target = |name: &str, kind, test| CargoTarget {
        package: package.to_string(),
        package_id: format!("{package}-id"),
        name: name.to_string(),
        relative: PathBuf::from(format!("libs/{package}/{name}.rs")),
        kind,
        test,
        executable: matches!(kind, CargoTargetKind::Bin | CargoTargetKind::Example),
    };
    packages.insert(
        package.to_string(),
        vec![
            target("core", CargoTargetKind::Library, true),
            target("tool", CargoTargetKind::Bin, true),
            target("demo", CargoTargetKind::Example, true),
            target("integration", CargoTargetKind::Test, false),
            target("benchmark", CargoTargetKind::Bench, false),
            target("build-script-build", CargoTargetKind::CustomBuild, false),
        ],
    );
    let plan = AuditPlan::from_catalog(
        CargoCatalog {
            packages,
            resolved: BTreeMap::new(),
        },
        compiler_identity(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("repository root"),
        )
        .expect("fixture compiler identity"),
    )
    .expect("fixture plan");
    let identities = plan
        .compilations
        .iter()
        .filter_map(|compilation| match &compilation.mode {
            CompilationMode::Example(target) if target.package == package => {
                Some(format!("normal:{}", target.name))
            }
            CompilationMode::TestHarness(target) if target.package == package => {
                Some(format!("test:{}", target.name))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let distinct = identities.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(
        identities.len(),
        distinct.len(),
        "two target modes shared one graph plan"
    );
    assert_eq!(
        distinct,
        [
            "normal:demo",
            "test:benchmark",
            "test:core",
            "test:demo",
            "test:integration",
            "test:tool",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
    );
}

#[test]
fn authoritative_verdict_includes_every_compiled_graph() {
    let source = PRODUCTION;
    let check = source
        .split_once("pub fn check(paths: &Paths) -> Result<()> {")
        .and_then(|(_, source)| source.split_once("struct Compilation"))
        .map(|(source, _)| source)
        .expect("API-audit check body");
    assert!(check.contains("let mut uses = GraphUses::default();"));
    assert_eq!(check.matches("uses.observe_cargo(").count(), 1);
    assert!(!check.contains("if compilation.kind == RootKind::Production"));
    assert!(check.contains("uses.validate_dispatches(&inventory)?;"));
    assert!(check.contains("let dead = uses.dead(&inventory);"));
}

#[test]
fn mutually_exclusive_compiler_graphs_cannot_complete_each_others_call_chain() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn target() {}
fn bridge() {
    #[cfg(audit_edge)]
    target();
}
#[cfg(not(audit_edge))]
fn main() { bridge(); }
#[cfg(audit_edge)]
fn main() {}
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let target = inventory
        .nodes
        .iter()
        .find(|node| node.label.contains("fn target"))
        .expect("target item")
        .id;
    let mut merged = GraphUses::default();
    let mut graphs = GraphUses::default();
    for round in 0..inventory.rounds {
        let insertions = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        for (index, extra) in [&["--cfg", "audit_edge"][..], &[][..]]
            .into_iter()
            .enumerate()
        {
            let output = compile_fixture(&root, extra);
            graphs
                .observe(
                    index,
                    RootKind::Production,
                    &inventory,
                    &root,
                    &output,
                    &insertions,
                )
                .expect("read graph diagnostics");
            merged
                .observe(
                    0,
                    RootKind::Production,
                    &inventory,
                    &root,
                    &output,
                    &insertions,
                )
                .expect("read merged diagnostics");
        }
    }

    assert!(
        !merged.dead(&inventory).iter().any(|node| node.id == target),
        "roots and edges from different compiler graphs formed a synthetic caller"
    );
    assert!(
        graphs.dead(&inventory).iter().any(|node| node.id == target),
        "the target was not dead in every individual compiler graph"
    );

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn a_later_round_preserves_higher_graph_evidence() {
    let root = fixture("#![warn(deprecated)]\npub fn target() {}\nfn main() { target(); }\n");
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let target = inventory
        .nodes
        .iter()
        .find(|node| node.label.contains("fn target"))
        .expect("target item")
        .id;
    let mut live = None;
    let mut quiet = None;
    for round in 0..inventory.antichain_rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        if output.contains(&format!("{MARKER}{target}")) {
            live = Some((instrumentation, output));
        } else if !instrumentation.ids.contains(&target) {
            quiet = Some((instrumentation, output));
        }
    }
    let (live_instrumentation, live_output) = live.expect("a target-use round");
    let (quiet_instrumentation, quiet_output) = quiet.expect("a non-target round");
    let mut graphs = GraphUses::default();

    graphs
        .observe(
            1,
            RootKind::Production,
            &inventory,
            &root,
            &live_output,
            &live_instrumentation,
        )
        .expect("observe graph one's resolved target use");
    assert!(!graphs.dead(&inventory).iter().any(|node| node.id == target));

    for graph in [0, 1] {
        graphs
            .observe(
                graph,
                RootKind::Production,
                &inventory,
                &root,
                &quiet_output,
                &quiet_instrumentation,
            )
            .expect("observe the next quiet compiler round");
    }

    assert!(
        graphs.graphs.get(1).is_some_and(|graph| {
            !graph.roots.is_empty() && graph.edges.values().any(|used| used.contains(&target))
        }),
        "observing graph zero erased prior graph-one evidence"
    );
    assert!(!graphs.dead(&inventory).iter().any(|node| node.id == target));
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn an_api_verdict_cannot_outlive_its_source_snapshot() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-snapshot");
    for directory in COPY_DIRECTORIES {
        std::fs::create_dir_all(root.join(directory)).expect("snapshot directory");
    }
    std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
    for file in COPY_FILES {
        std::fs::write(root.join(file), file).expect("snapshot input");
    }
    let source_relative = Path::new("libs/fixture/src/lib.rs");
    let source = root.join(source_relative);
    std::fs::create_dir_all(source.parent().unwrap()).expect("fixture source directory");
    std::fs::write(&source, "pub fn original() {}\n").expect("fixture source");
    let lock =
        File::open(std::env::current_exe().expect("test executable")).expect("fixture lock handle");
    let workspace = AuditWorkspace {
        _lock: lock,
        source_root: root.clone(),
        root: root.clone(),
        build: root.join("build"),
        cargo_home: root.clone(),
        registry_source: root.clone(),
        cargo_inputs: BTreeMap::new(),
        execution_root: root.clone(),
        source: source_snapshot(&root).expect("initial source snapshot"),
    };
    workspace
        .require_source_unchanged(&root)
        .expect("unchanged source");

    std::fs::write(&source, "pub fn replacement() {}\n").expect("change source");
    let error = workspace.require_source_unchanged(&root).unwrap_err();
    assert!(
        format!("{error:#}")
            .replace('\\', "/")
            .contains("libs/fixture/src/lib.rs")
    );
    std::fs::write(&source, "pub fn original() {}\n").expect("restore source");

    let added_relative = Path::new("wost/src/added.rs");
    let added = root.join(added_relative);
    std::fs::create_dir_all(added.parent().unwrap()).expect("added source directory");
    std::fs::write(&added, "pub fn added() {}\n").expect("add source");
    let error = workspace.require_source_unchanged(&root).unwrap_err();
    assert!(
        format!("{error:#}")
            .replace('\\', "/")
            .contains("wost/src/added.rs")
    );
    std::fs::remove_file(added).expect("remove added source");

    std::fs::remove_file(root.join("Cargo.lock")).expect("remove snapshot input");
    let error = workspace.require_source_unchanged(&root).unwrap_err();
    assert!(format!("{error:#}").contains("Cargo.lock"));
}

#[test]
fn compiler_graphs_cannot_change_the_instrumented_copy() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-copy-snapshot");
    for directory in COPY_DIRECTORIES {
        std::fs::create_dir_all(root.join(directory)).expect("snapshot directory");
    }
    std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
    for file in COPY_FILES {
        std::fs::write(root.join(file), file).expect("snapshot input");
    }
    let source = root.join("libs/fixture/src/lib.rs");
    let manifest = root.join("libs/fixture/Cargo.toml");
    let config = root.join(".cargo/config.toml");
    std::fs::create_dir_all(source.parent().unwrap()).expect("fixture source directory");
    std::fs::write(&source, "pub fn original() {}\n").expect("fixture source");
    std::fs::write(&manifest, "[package]\nname = \"fixture\"\n").expect("fixture manifest");
    std::fs::write(&config, "[net]\noffline = true\n").expect("fixture cargo config");
    let lock =
        File::open(std::env::current_exe().expect("test executable")).expect("fixture lock handle");
    let workspace = AuditWorkspace {
        _lock: lock,
        source_root: root.clone(),
        root: root.clone(),
        build: root.join("build"),
        cargo_home: root.clone(),
        registry_source: root.clone(),
        cargo_inputs: BTreeMap::new(),
        execution_root: root.clone(),
        source: source_snapshot(&root).expect("initial source snapshot"),
    };
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    inventory.instrument(&root, 0).expect("instrument fixture");
    let expected = source_snapshot(&root).expect("instrumented source snapshot");
    workspace
        .require_copy_unchanged(&expected)
        .expect("intentional instrumentation is authoritative");

    for path in [root.join("Cargo.toml"), config, manifest] {
        let original = std::fs::read(&path).expect("copied input");
        std::fs::write(&path, b"replacement").expect("replace copied input");
        let error = workspace.require_copy_unchanged(&expected).unwrap_err();
        let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
        assert!(
            format!("{error:#}")
                .replace('\\', "/")
                .contains(&relative.replace('\\', "/")),
            "{error:#}"
        );
        std::fs::write(path, original).expect("restore copied input");
    }

    let added = root.join("libs/fixture/build-script-write");
    std::fs::write(&added, b"outside OUT_DIR").expect("simulated build-script write");
    let error = workspace.require_copy_unchanged(&expected).unwrap_err();
    assert!(
        format!("{error:#}")
            .replace('\\', "/")
            .contains("libs/fixture/build-script-write")
    );
    std::fs::remove_dir_all(root).expect("remove copied-input fixture");
}

#[test]
fn copied_source_is_bound_after_instrumentation_and_after_each_graph() {
    let source = PRODUCTION;
    let production = source
        .split_once("#[cfg(test)]\nmod tests")
        .map(|(production, _)| production)
        .expect("API audit test boundary");
    assert!(
        !production.contains(".output()"),
        "an API-audit child bypasses bounded process execution"
    );
    let pristine_compile = source
        .find("let _pristine = compile_graphs(&workspace, 0, &pristine_source, &plan, None)?;")
        .expect("pristine compilation step");
    let pristine_authority = source
        .find("let pristine_build_authority = (0..plan.compilations.len())")
        .expect("pristine build-script authority snapshot");
    let instrumentation = source
        .find("let insertions = inventory.instrument(&workspace.root, round)?;")
        .expect("instrumentation step");
    let snapshot = source
        .find("let instrumented_source = source_snapshot(&workspace.root)?;")
        .expect("instrumented snapshot");
    let compilation = source
        .find("Some(&pristine_build_authority)")
        .expect("compiler graph loop");
    assert!(
        pristine_compile < pristine_authority
            && pristine_authority < instrumentation
            && instrumentation < snapshot
            && snapshot < compilation
    );

    let compile_graphs = source
        .split_once("fn compile_graphs(")
        .and_then(|(_, source)| source.split_once("fn compile_graph("))
        .map(|(source, _)| source)
        .expect("compile-graphs body");
    let compiler = compile_graphs
        .find("let compiled = compile_graph(")
        .expect("compiler graph execution");
    let authority = compile_graphs
        .find("require_build_script_authority_unchanged(")
        .expect("build-script authority guard");
    let accepted = compile_graphs
        .find("Ok(compiled)")
        .expect("accepted compilation");
    assert!(compiler < authority && authority < accepted);

    let compile_graph = source
        .split_once("fn compile_graph(")
        .and_then(|(_, source)| source.split_once("fn cargo_arguments"))
        .map(|(source, _)| source)
        .expect("compile_graph body");
    let output = compile_graph
        .find("bounded_process::output_streamed")
        .expect("compiler completion");
    let copy_guard = compile_graph
        .find("workspace.require_copy_unchanged(expected)?;")
        .expect("copied-source guard");
    let verdict = compile_graph
        .find("if !captured.status.success()")
        .expect("compiler verdict");
    assert!(output < copy_guard && copy_guard < verdict);
}

#[test]
fn compiler_graphs_stream_structured_output_instead_of_capturing_it_whole() {
    let source = PRODUCTION;
    let compile_graph = source
        .split_once("fn compile_graph(")
        .and_then(|(_, source)| source.split_once("fn cargo_arguments"))
        .map(|(source, _)| source)
        .expect("compile_graph body");

    assert!(
        compile_graph.contains("bounded_process::output_streamed("),
        "API compiler stdout is not reduced while it streams"
    );
    assert!(
        !compile_graph.contains("bounded_process::output_with_limit("),
        "API compiler still stages its complete structured output"
    );
}

#[test]
fn windows_hosts_link_with_sysroot_lld_and_other_hosts_with_cc() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-host-linker");
    let sysroot = std::fs::canonicalize(&root).unwrap();
    let windows = "x86_64-pc-windows-gnu";
    let lld_directory = sysroot.join("lib/rustlib").join(windows).join("bin");
    let driver_directory = sysroot.join("drivers");
    std::fs::create_dir_all(&lld_directory).unwrap();
    std::fs::create_dir(&driver_directory).unwrap();
    std::fs::write(lld_directory.join("rust-lld.exe"), b"lld").unwrap();
    let lone_driver_directory = sysroot.join("lone-driver");
    std::fs::create_dir(&lone_driver_directory).unwrap();
    std::fs::write(lone_driver_directory.join("cc"), b"cc").unwrap();
    std::fs::write(driver_directory.join("cc"), b"cc").unwrap();
    std::fs::write(driver_directory.join("ld"), b"ld").unwrap();
    let search_path = std::env::join_paths([
        Path::new("drivers"),
        lone_driver_directory.as_path(),
        driver_directory.as_path(),
    ])
    .unwrap();

    assert_eq!(
        host_linker(windows, &sysroot, Some(&search_path)).unwrap(),
        (
            std::fs::canonicalize(lld_directory.join("rust-lld.exe")).unwrap(),
            None
        )
    );
    assert_eq!(
        host_linker("x86_64-unknown-linux-gnu", &sysroot, Some(&search_path)).unwrap(),
        (
            std::fs::canonicalize(driver_directory.join("cc")).unwrap(),
            Some(driver_directory.clone())
        )
    );
    assert!(
        host_linker(
            "x86_64-unknown-linux-gnu",
            &sysroot,
            Some(lone_driver_directory.as_os_str())
        )
        .is_err()
    );
    assert!(
        host_linker(
            "x86_64-unknown-linux-gnu",
            &sysroot,
            Some(std::ffi::OsStr::new("drivers"))
        )
        .is_err()
    );
    assert!(host_linker("x86_64-unknown-linux-gnu", &sysroot, None).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn compiler_executable_identity_is_streamed_instead_of_allocated_whole() {
    let source = PRODUCTION;
    let capture = source
        .split_once("fn compiler_identity(source_root: &Path) -> Result<CompilerIdentity> {")
        .and_then(|(_, source)| source.split_once("impl CompilerInputAuthority"))
        .map(|(source, _)| source)
        .expect("compiler identity capture body");
    let revalidation = source
        .split_once("impl CompilerIdentity {")
        .and_then(|(_, source)| source.split_once("fn cargo_encoded_rustflags("))
        .map(|(source, _)| source)
        .expect("compiler identity revalidation body");

    assert_eq!(
        capture.matches("digest_compiler_executable(").count(),
        3,
        "compiler identity capture must stream every compiler executable"
    );
    assert_eq!(
        revalidation.matches("digest_compiler_executable(").count(),
        3,
        "compiler identity revalidation must stream every compiler executable"
    );
    assert!(
        !capture.contains("std::fs::read(") && !revalidation.contains("std::fs::read("),
        "compiler identity still allocates an entire executable"
    );
}

#[test]
fn compiler_input_authority_files_are_streamed_instead_of_allocated_whole() {
    let source = PRODUCTION;
    let boundary = source
        .split_once("fn snapshot_marker_free_file(")
        .and_then(|(_, source)| source.split_once("fn snapshot_marker_free_tree("))
        .map(|(source, _)| source)
        .expect("compiler authority file snapshot boundary");

    assert!(
        boundary.contains("stream_marker_free_digest(path, label)?"),
        "compiler authority snapshot does not use its streaming authenticator"
    );
    assert!(
        !boundary.contains("std::fs::read("),
        "compiler authority snapshot still allocates a whole input file"
    );
}

#[test]
fn source_freshness_is_checked_after_compilation_and_before_the_verdict() {
    let source = PRODUCTION;
    let discovery = source
        .split_once("impl AuditPlan {")
        .and_then(|(_, source)| source.split_once("fn from_catalog("))
        .map(|(source, _)| source)
        .expect("API-audit plan discovery body");
    let metadata = discovery
        .find("let current = CargoCatalog::discover(workspace, expected, pass, &compiler)?;")
        .expect("Cargo metadata discovery");
    let metadata_complete = discovery
        .find("let catalog = catalog.context(\"the API audit has no registered Cargo graph\")?;")
        .expect("completed Cargo metadata catalog");
    let metadata_authority = discovery
        .find("compiler.require_input_authority_unchanged()?;")
        .expect("post-metadata compiler-authority guard");
    assert_eq!(
        discovery
            .matches("compiler.require_input_authority_unchanged()?;")
            .count(),
        1
    );
    assert!(metadata < metadata_complete && metadata_complete < metadata_authority);

    let check = source
        .split_once("pub fn check(paths: &Paths) -> Result<()> {")
        .and_then(|(_, source)| source.split_once("pub(crate) fn report_names_exact_dead_item"))
        .map(|(source, _)| source)
        .expect("API-audit check body");
    let pristine = check
        .find("let _pristine = compile_graphs(&workspace, 0, &pristine_source, &plan, None)?;")
        .expect("pristine compiler pass");
    let pristine_baseline = check
        .find("let pristine_build_authority = (0..plan.compilations.len())")
        .expect("pristine build-output baseline");
    let compile = check
        .find("for round in 0..inventory.rounds {")
        .expect("compiler loop");
    let observation = check
        .find("uses.observe_cargo(")
        .expect("compiler diagnostic observation");
    let round_authority = check
        .find("plan.compiler.require_target_libraries_unchanged()?;")
        .expect("per-round target-library guard");
    let registry_proofs = check
        .match_indices("workspace.require_cargo_inputs_unchanged()?;")
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    let [pristine_registry, round_registry] = registry_proofs.as_slice() else {
        panic!("API check must rebind the registry after the pristine round and once per round");
    };
    assert!(*pristine_registry < pristine_baseline);
    assert!(compile < *round_registry && *round_registry < round_authority);
    let freshness = check
        .find("workspace.require_source_unchanged(paths.root())?;")
        .expect("source freshness guard");
    let full_authority = check
        .match_indices("plan.compiler.require_input_authority_unchanged()?;")
        .map(|(position, _)| position)
        .collect::<Vec<_>>();
    let [post_pristine, final_authority] = full_authority.as_slice() else {
        panic!("API check must contain exactly two full compiler-authority guards");
    };
    let verdict = check
        .find("let dead = uses.dead(&inventory);")
        .expect("API verdict");
    assert_eq!(
        check
            .matches("plan.compiler.require_target_libraries_unchanged()?;")
            .count(),
        1
    );
    assert!(
        pristine < *post_pristine
            && *post_pristine < pristine_baseline
            && pristine_baseline < compile
            && compile < observation
            && observation < round_authority
            && round_authority < freshness
            && freshness < *final_authority
            && *final_authority < verdict
    );
}
