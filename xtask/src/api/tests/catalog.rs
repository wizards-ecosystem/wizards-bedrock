use super::*;

#[test]
fn cargo_metadata_not_a_conventional_filename_defines_a_target_root() {
    let root = fixture("#![warn(deprecated)]\npub fn orphan() {}\nfn main() { orphan(); }\n");
    let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
    let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
        .expect("canonical fixture source");
    let escaped = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    let metadata = format!(
        "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture_core\",\"kind\":[\"lib\"],\"crate_types\":[\"lib\"],\"src_path\":\"{}\",\"test\":true}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
        escaped(&canonical),
        escaped(&source)
    );
    let catalog = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
        .expect("fixture metadata");
    let inventory = Inventory::discover_from_catalog(&root, &catalog).expect("metadata inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &["--crate-type", "lib"]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read library diagnostics");
    }
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "orphan"),
        "a library at src/main.rs acquired an executable root"
    );
    std::fs::remove_dir_all(root).expect("remove metadata fixture");
}

#[test]
fn active_procedural_macros_are_outside_the_audit_trust_boundary() {
    let root = fixture("fn main() {}\n");
    let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
    let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
        .expect("canonical fixture source");
    let escaped = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    let packages = format!(
        "[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"derive-id\",\"name\":\"untrusted-derive\",\"targets\":[{{\"name\":\"untrusted_derive\",\"kind\":[\"proc-macro\"],\"crate_types\":[\"proc-macro\"],\"src_path\":\"outside.rs\",\"test\":false}}]}}]",
        escaped(&source)
    );
    let inactive = format!(
        "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":{},\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
        escaped(&canonical),
        packages
    );
    CargoCatalog::parse(&root, &root, inactive.as_bytes(), &["fixture"], true)
        .expect("an unresolved procedural macro is inactive");
    let active = format!(
        "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":{},\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"derive-id\"],\"deps\":[{{\"name\":\"untrusted_derive\",\"pkg\":\"derive-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"derive-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
        escaped(&canonical),
        packages
    );
    let error = CargoCatalog::parse(&root, &root, active.as_bytes(), &["fixture"], true)
        .expect_err("active procedural macro must be refused");
    assert!(format!("{error:#}").contains("untrusted-derive"));
    std::fs::remove_dir_all(root).expect("remove procedural-macro fixture");
}

#[test]
fn unreviewed_dependency_build_scripts_are_outside_the_audit_authority() {
    let root = fixture("fn main() {}\n");
    let build = root.join("evil-build.rs");
    std::fs::write(
        &build,
        r#"fn main() {
    let source = std::fs::read_to_string("libs/fixture/src/main.rs").unwrap();
    let generated = std::env::temp_dir().join("api-audit-generated.rs");
    std::fs::write(generated, source).unwrap();
    println!("cargo:rerun-if-changed=fixed");
}"#,
    )
    .expect("hostile dependency build script");
    let compiled = Command::new("rustc")
        .current_dir(&root)
        .args([
            "--edition=2024",
            "evil-build.rs",
            "--emit=metadata",
            "-o",
            "evil-build.rmeta",
        ])
        .output()
        .expect("compile hostile dependency build script");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    std::fs::write(
        root.join("Cargo.lock"),
        r#"version = 4

[[package]]
name = "anyhow"
version = "1.0.104"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470"
"#,
    )
    .expect("fixture lockfile");

    let active = BTreeSet::from(["evil-id".to_string()]);
    let scripts = BTreeMap::from([(
        "evil-id".to_string(),
        DependencyBuildScript {
            name: "evil-build".to_string(),
            version: "0.1.0".to_string(),
            registry: false,
        },
    )]);
    let direct_error = validate_dependency_build_scripts(&root, &active, &scripts)
        .expect_err("an active non-registry build script must be refused directly");
    assert!(format!("{direct_error:#}").contains("not a checksum-bound crates.io package"));

    let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
    let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
        .expect("canonical fixture source");
    let escaped = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    let metadata = format!(
        "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"evil-id\",\"name\":\"evil-build\",\"version\":\"0.1.0\",\"source\":null,\"targets\":[{{\"name\":\"build-script-build\",\"kind\":[\"custom-build\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":false}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"evil-id\"],\"deps\":[{{\"name\":\"evil_build\",\"pkg\":\"evil-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"evil-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
        escaped(&canonical),
        escaped(&source),
        escaped(&build),
    );
    let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
        .expect_err("unreviewed dependency build script must be refused");
    assert!(
        format!("{error:#}")
            .contains("active dependency build script evil-build 0.1.0 is not a checksum-bound")
    );
    std::fs::remove_dir_all(root).expect("remove dependency-build fixture");
}

#[test]
fn active_path_packages_are_outside_the_compiler_input_authority() {
    let root = fixture("fn main() {}\n");
    let external = root.with_extension("external-package");
    std::fs::create_dir_all(external.join("src")).expect("external package directory");
    std::fs::write(
        external.join("Cargo.toml"),
        "[package]\nname = \"external\"\nversion = \"0.1.0\"\n",
    )
    .expect("external package manifest");
    std::fs::write(external.join("src/lib.rs"), "pub fn forged() {}\n")
        .expect("external package source");
    let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
    let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
        .expect("canonical fixture source");
    let external_manifest =
        std::fs::canonicalize(external.join("Cargo.toml")).expect("canonical external manifest");
    let external_source =
        std::fs::canonicalize(external.join("src/lib.rs")).expect("canonical external source");
    let escaped = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    let metadata = format!(
        "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"external-id\",\"name\":\"external\",\"version\":\"0.1.0\",\"source\":null,\"manifest_path\":\"{}\",\"targets\":[{{\"name\":\"external\",\"kind\":[\"lib\"],\"crate_types\":[\"lib\"],\"src_path\":\"{}\",\"test\":true}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"external-id\"],\"deps\":[{{\"name\":\"external\",\"pkg\":\"external-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"external-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
        escaped(&canonical),
        escaped(&source),
        escaped(&external_manifest),
        escaped(&external_source),
    );

    let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
        .expect_err("active path package must be refused");
    assert!(
        format!("{error:#}").contains("not a checksum-bound crates.io package"),
        "{error:#}"
    );
    std::fs::remove_dir_all(external).expect("remove external package fixture");
    std::fs::remove_dir_all(root).expect("remove path-package fixture");
}

#[test]
fn active_dependencies_cannot_replace_language_crate_roots() {
    let root = fixture("fn main() {}\n");
    let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
    let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
        .expect("canonical fixture source");
    let escaped = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    let packages = format!(
        "[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"evil-id\",\"name\":\"evil\",\"targets\":[]}}]",
        escaped(&source)
    );
    for alias in RESERVED_LANGUAGE_ROOTS {
        let metadata = format!(
            "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":{},\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"evil-id\"],\"deps\":[{{\"name\":\"{}\",\"pkg\":\"evil-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"evil-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
            escaped(&canonical),
            packages,
            alias
        );
        let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
            .expect_err("reserved dependency alias must be refused");
        assert!(format!("{error:#}").contains(alias), "{error:#}");
    }
    std::fs::remove_dir_all(root).expect("remove reserved-root metadata fixture");
}

#[test]
fn direct_normal_dependencies_require_every_identity_and_kind() {
    let target = |package: &str, package_id: &str| CargoTarget {
        package: package.to_string(),
        package_id: package_id.to_string(),
        name: package.to_string(),
        relative: PathBuf::from(format!("libs/{package}/src/lib.rs")),
        kind: CargoTargetKind::Library,
        test: true,
        executable: false,
    };
    let catalog = CargoCatalog {
        packages: BTreeMap::from([
            ("root".to_string(), vec![target("root", "root-id")]),
            (
                "dependency".to_string(),
                vec![target("dependency", "dependency-id")],
            ),
            ("other".to_string(), vec![target("other", "other-id")]),
        ]),
        resolved: BTreeMap::from([(
            "root-id".to_string(),
            vec![
                ResolvedDependency {
                    name: "good".to_string(),
                    package_id: "dependency-id".to_string(),
                    normal: true,
                    build: false,
                    dev: false,
                },
                ResolvedDependency {
                    name: "build-only".to_string(),
                    package_id: "dependency-id".to_string(),
                    normal: false,
                    build: true,
                    dev: false,
                },
                ResolvedDependency {
                    name: "dev-only".to_string(),
                    package_id: "dependency-id".to_string(),
                    normal: false,
                    build: false,
                    dev: true,
                },
                ResolvedDependency {
                    name: "wrong-package".to_string(),
                    package_id: "other-id".to_string(),
                    normal: true,
                    build: false,
                    dev: false,
                },
                ResolvedDependency {
                    name: "wrong-alias".to_string(),
                    package_id: "dependency-id".to_string(),
                    normal: true,
                    build: false,
                    dev: false,
                },
            ],
        )]),
    };

    assert!(catalog.has_direct_normal_dependency("root", "good", "dependency"));
    assert!(!catalog.has_direct_normal_dependency("missing", "good", "dependency"));
    assert!(!catalog.has_direct_normal_dependency("root", "good", "missing"));
    assert!(!catalog.has_direct_normal_dependency("root", "build-only", "dependency"));
    assert!(!catalog.has_direct_normal_dependency("root", "dev-only", "dependency"));
    assert!(!catalog.has_direct_normal_dependency("root", "wrong-package", "dependency"));
    assert!(!catalog.has_direct_normal_dependency("root", "missing-alias", "dependency"));
}

#[test]
fn resolved_dependency_kinds_are_exact() {
    let package_names = BTreeMap::from([
        ("root-id".to_string(), "root".to_string()),
        ("dependency-id".to_string(), "dependency".to_string()),
    ]);
    let parse = |kinds: &str| {
        let metadata = format!(
            "{{\"resolve\":{{\"nodes\":[{{\"id\":\"root-id\",\"dependencies\":[\"dependency-id\"],\"deps\":[{{\"name\":\"dependency\",\"pkg\":\"dependency-id\",\"dep_kinds\":{kinds}}}]}}]}}}}"
        );
        let value = crate::json::parse_json(metadata.as_bytes(), "dependency-kind fixture")?;
        let document = crate::json::json_object(&value, "dependency-kind fixture")?;
        parse_resolved_dependencies(document, &package_names)
    };

    let resolved = parse(r#"[{"kind":null},{"kind":"build"},{"kind":"dev"}]"#)
        .expect("all supported dependency kinds");
    assert_eq!(
        resolved.get("root-id").expect("root dependency node"),
        &[ResolvedDependency {
            name: "dependency".to_string(),
            package_id: "dependency-id".to_string(),
            normal: true,
            build: true,
            dev: true,
        }]
    );

    let unsupported =
        parse(r#"[{"kind":"unknown"}]"#).expect_err("an unknown dependency kind must be refused");
    assert!(format!("{unsupported:#}").contains("unsupported dependency kind `unknown`"));
    let non_string =
        parse(r#"[{"kind":true}]"#).expect_err("a non-string dependency kind must be refused");
    assert!(format!("{non_string:#}").contains("dependency kind is not null or a string"));
    let absent = parse("[]").expect_err("an absent dependency kind must be refused");
    assert!(format!("{absent:#}").contains("has no dependency kind"));
}

#[test]
fn dependency_activation_and_reserved_aliases_share_one_exact_policy() {
    let dependency = |name: &str, package_id: &str, normal, build, dev| ResolvedDependency {
        name: name.to_string(),
        package_id: package_id.to_string(),
        normal,
        build,
        dev,
    };
    let resolved = BTreeMap::from([
        (
            "root-id".to_string(),
            vec![
                dependency("normal", "normal-id", true, false, false),
                dependency("build", "build-id", false, true, false),
                dependency("root-dev", "root-dev-id", false, false, true),
            ],
        ),
        (
            "normal-id".to_string(),
            vec![dependency(
                "transitive-dev",
                "transitive-dev-id",
                false,
                false,
                true,
            )],
        ),
        ("build-id".to_string(), Vec::new()),
        ("root-dev-id".to_string(), Vec::new()),
        ("transitive-dev-id".to_string(), Vec::new()),
    ]);
    let roots = BTreeSet::from(["root-id".to_string()]);
    let package_names = BTreeMap::from([
        ("root-id".to_string(), "root".to_string()),
        ("normal-id".to_string(), "normal".to_string()),
        ("build-id".to_string(), "build".to_string()),
        ("root-dev-id".to_string(), "root-dev".to_string()),
        (
            "transitive-dev-id".to_string(),
            "transitive-dev".to_string(),
        ),
    ]);

    let production =
        active_resolved_packages(&resolved, &roots, false).expect("production dependency closure");
    assert_eq!(
        production,
        BTreeSet::from([
            "root-id".to_string(),
            "normal-id".to_string(),
            "build-id".to_string(),
        ])
    );
    let verification =
        active_resolved_packages(&resolved, &roots, true).expect("verification dependency closure");
    assert_eq!(
        verification,
        BTreeSet::from([
            "root-id".to_string(),
            "normal-id".to_string(),
            "build-id".to_string(),
            "root-dev-id".to_string(),
        ])
    );

    let rejects = |owner: &str, alias: &str, include_root_dev_dependencies: bool| {
        let mut candidate = resolved.clone();
        candidate
            .get_mut(owner)
            .expect("dependency owner")
            .iter_mut()
            .find(|dependency| dependency.name == alias)
            .expect("dependency alias")
            .name = "core".to_string();
        let active = active_resolved_packages(&candidate, &roots, include_root_dev_dependencies)
            .expect("candidate dependency closure");
        reject_reserved_dependency_roots(
            &candidate,
            &active,
            &roots,
            include_root_dev_dependencies,
            &package_names,
        )
        .is_err()
    };
    assert!(rejects("root-id", "normal", false));
    assert!(rejects("root-id", "build", false));
    assert!(!rejects("root-id", "root-dev", false));
    assert!(rejects("root-id", "root-dev", true));
    assert!(!rejects("normal-id", "transitive-dev", true));
}

#[test]
fn dependency_build_script_lock_identity_requires_name_and_version() {
    let root = fixture("fn main() {}\n");
    std::fs::write(
        root.join("Cargo.lock"),
        r#"version = 4

[[package]]
name = "anyhow"
version = "1.0.104"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470"

[[package]]
name = "other"
version = "1.0.104"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[package]]
name = "anyhow"
version = "9.9.9"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
"#,
    )
    .expect("identity fixture lockfile");
    let active = BTreeSet::from(["anyhow-id".to_string()]);
    let scripts = BTreeMap::from([(
        "anyhow-id".to_string(),
        DependencyBuildScript {
            name: "anyhow".to_string(),
            version: "1.0.104".to_string(),
            registry: true,
        },
    )]);

    validate_dependency_build_scripts(&root, &active, &scripts)
        .expect("the exact reviewed lock identity");
    std::fs::remove_dir_all(root).expect("remove lock identity fixture");
}

#[test]
fn only_the_exact_kernel_linker_build_script_is_trusted() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    let kernel = CargoTarget {
        package: crate::paths::KERNEL_PACKAGE.to_string(),
        package_id: "kernel-id".to_string(),
        name: "build-script-build".to_string(),
        relative: PathBuf::from("kernel/build.rs"),
        kind: CargoTargetKind::CustomBuild,
        test: false,
        executable: true,
    };
    validate_workspace_build_script(repository, crate::paths::KERNEL_PACKAGE, &kernel)
        .expect("current kernel build-script contract");

    let root = fixture("#[cfg(manufactured)] pub fn hidden() {}\nfn main() {}\n");
    let build_script = r#"fn main() {
    let source = std::fs::read_to_string("libs/fixture/src/main.rs").unwrap();
    if source.contains("wizard-api-audit:") {
        println!("cargo:rustc-cfg=manufactured");
    }
}
"#;
    std::fs::write(root.join("build.rs"), build_script).expect("hostile build script");
    let compiled = Command::new("rustc")
        .current_dir(&root)
        .args([
            "--edition=2024",
            "build.rs",
            "--emit=metadata",
            "-o",
            "build-script.rmeta",
        ])
        .output()
        .expect("compile hostile build script");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
    let main = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
        .expect("canonical fixture main");
    let build =
        std::fs::canonicalize(root.join("build.rs")).expect("canonical fixture build script");
    let escaped = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    let metadata = format!(
        "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}},{{\"name\":\"build-script-build\",\"kind\":[\"custom-build\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":false}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
        escaped(&canonical),
        escaped(&main),
        escaped(&build)
    );
    let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
        .expect_err("marker-sensitive workspace build script must be refused");
    assert!(format!("{error:#}").contains("exact build-script contract"));
    std::fs::remove_dir_all(root).expect("remove build-script fixture");
}

#[test]
fn test_harness_graphs_do_not_root_the_target_program_main() {
    assert!(
        !RootSet {
            production: true,
            verification: false,
        }
        .contains(RootKind::TestHarness)
    );
    assert!(
        RootSet {
            production: false,
            verification: true,
        }
        .contains(RootKind::TestHarness)
    );
}

#[test]
fn a_macro_generated_custom_harness_main_is_refused_from_its_manifest() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-custom-harness");
    std::fs::create_dir_all(root.join("libs/fixture/tests"))
        .expect("custom harness fixture directory");
    std::fs::create_dir_all(root.join("wost/src")).expect("empty custom harness wost source");
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"libs/fixture\"]\nresolver = \"3\"\n",
    )
    .expect("custom harness workspace manifest");
    std::fs::write(
        root.join("libs/fixture/Cargo.toml"),
        r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
autobins = false

[[test]]
name = "hostile"
path = "tests/hostile.rs"
harness = false
"#,
    )
    .expect("custom harness package manifest");
    std::fs::write(
        root.join("libs/fixture/tests/hostile.rs"),
        "pub fn hidden() {}\nmacro_rules! make_main { () => { fn main() { hidden(); } } }\nmake_main!();\n",
    )
    .expect("custom harness source");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    let compiler = compiler_identity(repository).expect("adjacent compiler identity");
    let compiled = isolated_test_cargo_command(&root, &compiler)
        .args([
            "test",
            "--offline",
            "--no-run",
            "--package",
            "fixture",
            "--test",
            "hostile",
        ])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("compile custom harness fixture");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let metadata = isolated_test_cargo_command(&root, &compiler)
        .args(["metadata", "--format-version=1", "--offline"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("read custom harness metadata");
    assert!(
        metadata.status.success(),
        "{}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let error = CargoCatalog::parse(&root, &root, &metadata.stdout, &["fixture"], true)
        .expect_err("a custom harness manifest must be refused");
    assert!(format!("{error:#}").contains("custom-harness"), "{error:#}");
    std::fs::remove_dir_all(root).expect("remove custom harness fixture");
}

#[test]
fn cargo_target_kinds_preserve_metadata_and_context() {
    let cases = [
        (
            "lib",
            CargoTargetKind::Library,
            Availability::Production,
            false,
            false,
        ),
        (
            "bin",
            CargoTargetKind::Bin,
            Availability::Production,
            true,
            true,
        ),
        (
            "example",
            CargoTargetKind::Example,
            Availability::Production,
            true,
            true,
        ),
        (
            "test",
            CargoTargetKind::Test,
            Availability::Verification,
            false,
            false,
        ),
        (
            "bench",
            CargoTargetKind::Bench,
            Availability::Verification,
            false,
            false,
        ),
        (
            "custom-build",
            CargoTargetKind::CustomBuild,
            Availability::Production,
            true,
            false,
        ),
    ];

    for (metadata, kind, availability, language_main, executable) in cases {
        assert_eq!(
            CargoTargetKind::parse(&[metadata]).expect("supported target kind"),
            kind
        );
        assert_eq!(kind.metadata_name(), metadata);
        let context = CargoTarget {
            package: "fixture".to_string(),
            package_id: "fixture-id".to_string(),
            name: "fixture-target".to_string(),
            relative: PathBuf::from("libs/fixture/src/lib.rs"),
            kind,
            test: true,
            executable,
        }
        .context();
        assert_eq!(context.availability, availability);
        assert_eq!(context.target_kind, kind);
        assert_eq!(context.language_main, language_main);
        assert_eq!(context.crate_name, "fixture_target");
    }

    let example = CargoTarget {
        package: "fixture".to_string(),
        package_id: "fixture-id".to_string(),
        name: "example".to_string(),
        relative: PathBuf::from("libs/fixture/examples/example.rs"),
        kind: CargoTargetKind::Example,
        test: false,
        executable: false,
    };
    assert!(!example.context().language_main);
}
