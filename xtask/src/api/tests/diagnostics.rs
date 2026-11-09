use super::*;

#[test]
fn virtual_source_diagnostics_accept_native_separators_without_traversal() {
    let expected = PathBuf::from("libs/timebase/src/lib.rs");
    assert_eq!(
        virtual_source_relative("/wizard/source\\libs\\timebase\\src\\lib.rs"),
        Some(expected.clone())
    );
    assert_eq!(
        virtual_source_relative("/wizard/source/libs/timebase/src/lib.rs"),
        Some(expected)
    );
    for refused in [
        "/wizard/source",
        "/wizard/source-other/libs/timebase/src/lib.rs",
        "/wizard/source/../outside.rs",
    ] {
        assert_eq!(virtual_source_relative(refused), None, "{refused}");
    }
}

#[test]
fn cargo_diagnostic_origins_bind_markers_to_the_exact_package_and_target() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-diagnostic-origin");
    let source = root.join("libs/fixture/src/main.rs");
    std::fs::create_dir_all(source.parent().unwrap()).expect("first-party source directory");
    std::fs::create_dir_all(root.join("tools/evil/src")).expect("sibling package source");
    std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"libs/fixture\", \"tools/evil\"]\nresolver = \"3\"\n",
    )
    .expect("origin fixture workspace manifest");
    std::fs::write(
        root.join("libs/fixture/Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nevil = { path = \"../../tools/evil\" }\n",
    )
    .expect("origin fixture package manifest");
    std::fs::write(
        root.join("tools/evil/Cargo.toml"),
        "[package]\nname = \"evil\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("sibling package manifest");
    std::fs::write(
        &source,
        "pub fn called() {}\npub fn caller() { called(); }\nfn main() { caller(); }\n",
    )
    .expect("first-party origin fixture source");
    std::fs::write(
        root.join("tools/evil/src/lib.rs"),
        format!("include!({:?});\n", source.to_string_lossy()),
    )
    .expect("sibling package include");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    let compiler = compiler_identity(repository).expect("adjacent compiler identity");
    let metadata = isolated_test_cargo_command(&root, &compiler)
        .args(["metadata", "--format-version=1", "--offline"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("origin fixture Cargo metadata");
    assert!(
        metadata.status.success(),
        "{}{}",
        String::from_utf8_lossy(&metadata.stdout),
        String::from_utf8_lossy(&metadata.stderr)
    );
    let catalog = CargoCatalog::parse(&root, &root, &metadata.stdout, &["fixture"], true)
        .expect("origin fixture catalog");
    let inventory =
        Inventory::discover_from_catalog(&root, &catalog).expect("origin fixture inventory");
    let called = inventory
        .nodes
        .iter()
        .find(|node| node.symbol == "called")
        .expect("called API")
        .id;
    let expected_package = inventory.files[inventory.nodes[called].file]
        .origin
        .package_id
        .clone();
    let mut legitimate = Uses::default();
    let mut observed_hostile = false;
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument origin fixture");
        let result = isolated_test_cargo_command(&root, &compiler)
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("CARGO_ENCODED_RUSTFLAGS", "")
            .args([
                "check",
                "--offline",
                "--locked",
                "--message-format=json",
                "--package",
                "fixture",
            ])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("compile origin fixture");
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let diagnostics =
            cargo_compilation_output(&result.stdout, None).expect("structured origin diagnostics");
        let matching = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.origin.package_id == expected_package)
            .cloned()
            .collect::<Vec<_>>();
        let hostile = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.origin.package_id != expected_package)
            .cloned()
            .collect::<Vec<_>>();
        if !hostile.is_empty() {
            observed_hostile = true;
            let error = Uses::default()
                .observe_cargo(
                    RootKind::Production,
                    &inventory,
                    &root,
                    &hostile,
                    &instrumentation,
                )
                .expect_err("wrong-package diagnostic must be refused");
            assert!(format!("{error:#}").contains("Cargo origin"), "{error:#}");
        }
        legitimate
            .observe_cargo(
                RootKind::Production,
                &inventory,
                &root,
                &matching,
                &instrumentation,
            )
            .expect("matching-package diagnostics remain authoritative");
    }
    assert!(
        observed_hostile,
        "sibling package include emitted no hostile marker diagnostic"
    );
    assert!(
        !legitimate
            .dead(&inventory)
            .iter()
            .any(|node| node.id == called),
        "matching-origin diagnostics did not preserve legitimate liveness"
    );
    std::fs::remove_dir_all(root).expect("remove diagnostic-origin fixture");
}

#[test]
fn cargo_target_origins_require_every_identity_field() {
    let root = fixture("pub fn first() {}\nfn main() {}\n");
    let second = root.join("libs/second/src/main.rs");
    std::fs::create_dir_all(second.parent().expect("second source parent"))
        .expect("second target directory");
    std::fs::write(&second, "pub fn second() {}\nfn main() {}\n").expect("second target source");
    let target = |package: &str, package_id: &str, relative: &str| CargoTarget {
        package: package.to_string(),
        package_id: package_id.to_string(),
        name: "shared-name".to_string(),
        relative: PathBuf::from(relative),
        kind: CargoTargetKind::Bin,
        test: true,
        executable: true,
    };
    let targets = [
        target("fixture", "fixture-id", "libs/fixture/src/main.rs"),
        target("second", "second-id", "libs/second/src/main.rs"),
    ];

    let inventory = Inventory::discover_with_targets(&root, &targets)
        .expect("two same-kind targets retain exact origins");
    assert_eq!(inventory.files.len(), 2);
    assert_eq!(
        inventory
            .files
            .iter()
            .map(|file| file.origin.package.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["fixture", "second"])
    );
    std::fs::remove_dir_all(root).expect("remove target-origin fixture");
}

#[test]
fn core_offset_of_diagnostics_are_discarded_at_the_terminal_opaque_callsite() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-offset-of-expansion");
    std::fs::create_dir_all(root.join("libs/fixture/src"))
        .expect("offset_of fixture source directory");
    std::fs::create_dir_all(root.join("wost/src")).expect("empty offset_of fixture wost source");
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"libs/fixture\"]\nresolver = \"3\"\n",
    )
    .expect("offset_of fixture workspace manifest");
    std::fs::write(
        root.join("libs/fixture/Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("offset_of fixture package manifest");
    std::fs::write(
        root.join("libs/fixture/src/lib.rs"),
        r#"#![warn(deprecated)]
#[repr(C)]
pub struct Gate { pub field: u8 }

#[cfg(test)]
mod tests {
    use super::Gate;

    macro_rules! inner_offset {
        () => { core::mem::offset_of!(Gate, field) };
    }
    macro_rules! nested_offset {
        () => { inner_offset!() };
    }

    #[test]
    fn layout() {
        let _ = core::mem::offset_of!(Gate, field);
        let _ = nested_offset!();
    }
}
"#,
    )
    .expect("offset_of fixture source");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    let compiler = compiler_identity(repository).expect("adjacent compiler identity");
    let metadata = isolated_test_cargo_command(&root, &compiler)
        .args(["metadata", "--format-version=1", "--offline"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("offset_of fixture Cargo metadata");
    assert!(
        metadata.status.success(),
        "{}{}",
        String::from_utf8_lossy(&metadata.stdout),
        String::from_utf8_lossy(&metadata.stderr)
    );
    let catalog = CargoCatalog::parse(&root, &root, &metadata.stdout, &["fixture"], true)
        .expect("offset_of fixture catalog");
    let selected = catalog
        .package("fixture")
        .expect("offset_of fixture package")
        .iter()
        .find(|target| target.kind == CargoTargetKind::Library)
        .expect("offset_of fixture library target")
        .clone();
    let inventory = Inventory::discover_from_catalog(&root, &catalog).expect("offset_of inventory");
    let gate = inventory
        .nodes
        .iter()
        .find(|node| node.symbol == "Gate")
        .expect("Gate API")
        .id;
    let instrumentation = (0..inventory.antichain_rounds)
        .find_map(|round| {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument offset_of fixture");
            instrumentation
                .ids
                .contains(&gate)
                .then_some(instrumentation)
        })
        .expect("Gate instrumentation round");
    let compile = || {
        isolated_test_cargo_command(&root, &compiler)
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("CARGO_ENCODED_RUSTFLAGS", "")
            .env("CARGO_BUILD_WARNINGS", "warn")
            .args([
                "test",
                "--no-run",
                "--offline",
                "--locked",
                "--message-format=json",
                "--package",
                "fixture",
                "--lib",
                "--target",
            ])
            .arg(&compiler.host_target)
            .args(["--color=never"])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("compile offset_of fixture")
    };
    let compiled = compile();
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let diagnostics =
        cargo_test_output(&compiled.stdout, &selected).expect("structured offset_of diagnostics");
    let external = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.id == gate
                && inventory.file_index(&root, &diagnostic.reported).is_none()
                && diagnostic
                    .immediate_expansion
                    .as_ref()
                    .is_some_and(|expansion| expansion.terminal)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !external.is_empty(),
        "offset_of emitted no Gate marker with an external primary span"
    );
    for diagnostic in &external {
        assert!(
            Path::new(&diagnostic.reported).ends_with(Path::new("library/core/src/mem/mod.rs")),
            "unexpected external primary span: {}",
            diagnostic.reported
        );
        let expansion = diagnostic
            .immediate_expansion
            .as_ref()
            .expect("offset_of diagnostic immediate expansion");
        assert_eq!(expansion.macro_decl_name, "core::mem::offset_of!");
        let file = inventory
            .file_index(&root, &expansion.location.reported)
            .expect("classified offset_of invocation");
        let at = corrected_marker_location(
            file,
            expansion.location.line,
            expansion.location.column,
            &instrumentation,
        )
        .expect("corrected offset_of invocation");
        assert!(
            inventory.is_opaque_macro_use(file, at),
            "offset_of expansion did not identify the opaque invocation"
        );
    }
    let mut uses = Uses::default();
    uses.observe_cargo(
        RootKind::TestHarness,
        &inventory,
        &root,
        &external,
        &instrumentation,
    )
    .expect("terminal opaque expansion diagnostics are discarded");
    assert!(uses.roots.is_empty());
    assert!(uses.edges.is_empty());
    assert!(uses.constructions.is_empty());
    assert!(uses.dead(&inventory).iter().any(|node| node.id == gate));

    let mut non_opaque = external[0].clone();
    let expansion = non_opaque
        .immediate_expansion
        .as_mut()
        .expect("offset_of diagnostic immediate expansion");
    expansion.location.line = 4;
    expansion.location.column = 1;
    let error = Uses::default()
        .observe_cargo(
            RootKind::TestHarness,
            &inventory,
            &root,
            &[non_opaque],
            &instrumentation,
        )
        .expect_err("an external primary cannot use a non-macro expansion callsite");
    assert!(
        format!("{error:#}").contains("not an opaque macro invocation"),
        "{error:#}"
    );

    let mut unclassified = external[0].clone();
    unclassified
        .immediate_expansion
        .as_mut()
        .expect("offset_of diagnostic immediate expansion")
        .location
        .reported = root.join("external-expansion.rs").display().to_string();
    let error = Uses::default()
        .observe_cargo(
            RootKind::TestHarness,
            &inventory,
            &root,
            &[unclassified],
            &instrumentation,
        )
        .expect_err("an external primary cannot use an unclassified expansion callsite");
    assert!(
        format!("{error:#}").contains("unclassified immediate expansion source"),
        "{error:#}"
    );

    let mut missing = external[0].clone();
    missing.immediate_expansion = None;
    let error = Uses::default()
        .observe_cargo(
            RootKind::TestHarness,
            &inventory,
            &root,
            &[missing],
            &instrumentation,
        )
        .expect_err("an external primary must identify its immediate expansion callsite");
    assert!(
        format!("{error:#}").contains("without an immediate expansion callsite"),
        "{error:#}"
    );

    let classified_nested = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.id == gate
                && inventory.file_index(&root, &diagnostic.reported).is_some()
                && diagnostic
                    .immediate_expansion
                    .as_ref()
                    .is_some_and(|expansion| !expansion.terminal)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !classified_nested.is_empty(),
        "nested offset_of emitted no classified nonterminal Gate diagnostic"
    );
    let mut uses = Uses::default();
    uses.observe_cargo(
        RootKind::TestHarness,
        &inventory,
        &root,
        &classified_nested,
        &instrumentation,
    )
    .expect("a classified primary retains its lexical opaque-macro policy");
    assert!(uses.roots.is_empty());
    assert!(uses.edges.is_empty());
    assert!(uses.constructions.is_empty());
    assert!(uses.dead(&inventory).iter().any(|node| node.id == gate));

    let external_nested = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.id == gate
                && inventory.file_index(&root, &diagnostic.reported).is_none()
                && diagnostic
                    .immediate_expansion
                    .as_ref()
                    .is_some_and(|expansion| !expansion.terminal)
        })
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        !external_nested.is_empty(),
        "nested offset_of emitted no external nonterminal Gate diagnostic"
    );
    let error = Uses::default()
        .observe_cargo(
            RootKind::TestHarness,
            &inventory,
            &root,
            &external_nested,
            &instrumentation,
        )
        .expect_err("an external primary cannot traverse a nested expansion chain");
    assert!(
        format!("{error:#}").contains("nested expansion chain"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).expect("remove offset_of fixture");
}

#[test]
fn decoded_literals_cannot_forge_a_compiler_diagnostic_marker() {
    for (label, source) in [
        (
            "direct escaped note",
            r#"#![warn(deprecated)]
#[deprecated(note = "wizard-api-audit:\x31")]
fn forged() {}
pub fn foundation() {}
fn main() { forged(); }
"#,
        ),
        (
            "escaped note in a macro body",
            r#"#![warn(deprecated)]
macro_rules! install_forge {
    () => {
        #[deprecated(note = "wizard-api-audit:\x31")]
        fn forged() {}
        fn call() { forged(); }
    }
}
install_forge!();
pub fn foundation() {}
fn main() { call(); }
"#,
        ),
    ] {
        let root = fixture(source);
        let output = compile_fixture(&root, &[]);
        let hostile_marker = format!("{MARKER}{}", 1);
        assert!(
            output.contains(&hostile_marker),
            "{label} did not produce the hostile compiler diagnostic: {output}"
        );
        let error = Inventory::discover(&root).expect_err(label);
        assert!(
            format!("{error:#}").contains("reserved diagnostic marker"),
            "{label}: {error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove escaped-marker fixture");
    }
}

#[test]
fn copied_source_cannot_forge_a_compiler_diagnostic_marker() {
    for external in [false, true] {
        let source = if external {
            "pub fn target() {}\nfn main() {}\n".to_string()
        } else {
            format!(
                "#[deprecated(note = \"{MARKER}0\")]\npub fn forged() {{}}\nfn main() {{ forged(); }}\n"
            )
        };
        let root = fixture(&source);
        if external {
            std::fs::create_dir_all(root.join("kernel/src")).expect("external caller directory");
            std::fs::write(
                root.join("kernel/src/caller.rs"),
                format!(
                    "#[deprecated(note = \"{MARKER}0\")]\nfn forged() {{}}\nfn call() {{ forged(); }}\n"
                ),
            )
            .expect("external caller source");
        }
        let error = Inventory::discover(&root).unwrap_err();
        assert!(
            format!("{error:#}").contains("reserved diagnostic marker"),
            "{error:#}"
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }
}

#[test]
fn the_audit_implementation_cannot_author_a_diagnostic_id() {
    let root = fixture("pub fn target() {}\nfn main() {}\n");
    let implementation = root.join("xtask/src/api.rs");
    std::fs::create_dir_all(implementation.parent().unwrap())
        .expect("audit implementation directory");
    std::fs::write(
        &implementation,
        format!(
            "#[deprecated(note = \"{MARKER}0\")]\nfn forged() {{}}\nfn call() {{ forged(); }}\n"
        ),
    )
    .expect("authored diagnostic fixture");

    let error = Inventory::discover(&root).unwrap_err();
    assert!(
        format!("{error:#}").contains("reserved diagnostic marker"),
        "{error:#}"
    );

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn a_diagnostic_id_must_be_instrumented_in_the_current_round() {
    let root = fixture("pub fn target() {}\nfn main() {}\n");
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let target = inventory
        .nodes
        .iter()
        .find(|node| node.label.contains("fn target"))
        .expect("target item");
    let round = inventory.antichain_rounds;
    let instrumentation = inventory.instrument(&root, round).expect("isolation round");
    assert!(!instrumentation.ids.contains(&target.id));
    let forged = format!(
        "outside.rs:1:1: warning: forged deprecation diagnostic {MARKER}{}\n",
        target.id
    );

    let error = Uses::default()
        .observe(
            RootKind::Production,
            &inventory,
            &root,
            &forged,
            &instrumentation,
        )
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("not instrumented in this compiler round"),
        "{error:#}"
    );

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn an_unclassified_diagnostic_location_cannot_default_to_a_root() {
    let root = fixture("pub fn target() {}\n\nfn main() {}\n");
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let target = inventory
        .nodes
        .iter()
        .find(|node| node.report && node.label.contains("fn target"))
        .expect("target item");
    let (instrumentation, _) = (0..inventory.antichain_rounds)
        .find_map(|round| {
            let instrumentation = inventory.instrument(&root, round).ok()?;
            instrumentation
                .ids
                .contains(&target.id)
                .then_some((instrumentation, round))
        })
        .expect("target instrumentation round");
    let forged = format!(
        "libs/fixture/src/main.rs:2:1: warning: unclassified use {MARKER}{}\n",
        target.id
    );

    let error = Uses::default()
        .observe(
            RootKind::Production,
            &inventory,
            &root,
            &forged,
            &instrumentation,
        )
        .unwrap_err();
    assert!(format!("{error:#}").contains("has no classified caller"));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}
