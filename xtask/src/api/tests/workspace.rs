use super::*;
use crate::test_support::{create_directory_symlink, create_file_symlink};

#[test]
fn compiler_semantic_environment_is_refused_before_the_audit() {
    for name in [
        "RUSTC",
        "rustflags",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_BUILD_TARGET",
        "cargo_encoded_rustflags",
        "CARGO_PROFILE_DEV_DEBUG",
        "CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS",
    ] {
        assert!(
            rejected_audit_environment_variable(name),
            "{name} escaped the compiler environment boundary"
        );
    }
    for name in [
        "PATH",
        "RUST_LOG",
        "RUST_BACKTRACE",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "CARGO_HOME",
        "CARGO_TARGET_DIR",
        "CARGO_TERM_COLOR",
        "CARGO_BIN_NAME",
        "CARGO_CRATE_NAME",
        "CARGO_MANIFEST_DIR",
        "CARGO_MANIFEST_PATH",
        "CARGO_PKG_NAME",
        "CARGO_PRIMARY_PACKAGE",
    ] {
        assert!(
            !rejected_audit_environment_variable(name),
            "{name} was rejected despite not changing compiled semantics"
        );
    }
}

#[test]
fn cargo_commands_remove_ambient_rust_and_cargo_semantics_before_pinning_their_own() {
    let mut command = Command::new("cargo");
    for name in [
        "RUSTFLAGS",
        "RUSTC_WRAPPER",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER",
        "TARGET",
        "CC_x86_64_unknown_linux_gnu",
        "CFLAGS_x86_64-unknown-linux-gnu",
        "CXXFLAGS_x86_64_unknown_linux_gnu",
        "AR_x86_64_unknown_linux_gnu",
    ] {
        command.env(name, "hostile");
    }
    remove_ambient_cargo_semantics(
        &mut command,
        [
            "RUSTFLAGS",
            "RUSTC_WRAPPER",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER",
            "TARGET",
            "CC_x86_64_unknown_linux_gnu",
            "CFLAGS_x86_64-unknown-linux-gnu",
            "CXXFLAGS_x86_64_unknown_linux_gnu",
            "AR_x86_64_unknown_linux_gnu",
        ]
        .into_iter()
        .map(std::ffi::OsString::from),
    );
    let environment = command.get_envs().collect::<BTreeMap<_, _>>();
    for name in [
        "RUSTFLAGS",
        "RUSTC_WRAPPER",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER",
        "TARGET",
        "CC_x86_64_unknown_linux_gnu",
        "CFLAGS_x86_64-unknown-linux-gnu",
        "CXXFLAGS_x86_64_unknown_linux_gnu",
        "AR_x86_64_unknown_linux_gnu",
    ] {
        assert_eq!(environment.get(std::ffi::OsStr::new(name)), Some(&None));
    }
}

#[test]
fn every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    let compiler = compiler_identity(repository).expect("adjacent compiler identity");
    let cargo = std::fs::canonicalize(env!("CARGO")).expect("canonical Cargo executable");
    let expected_compiler = std::fs::canonicalize(cargo.parent().unwrap().join(if cfg!(windows) {
        "rustc.exe"
    } else {
        "rustc"
    }))
    .expect("canonical adjacent compiler");
    assert_eq!(compiler.path, expected_compiler);
    let private_home = repository.join("private-cargo-home");
    let private_registry = repository.join("private-registry");
    let mut command =
        audited_cargo_command(repository, &private_home, &private_registry, &compiler)
            .expect("audited Cargo command");
    let kernel_rustflags = cargo_encoded_rustflags(
        Some(crate::paths::KERNEL_TARGET),
        repository,
        &compiler.input_authority.sysroot,
        &private_registry,
    )
    .expect("kernel rustflags contract");
    assert_eq!(
        cargo_encoded_rustflags(
            None,
            repository,
            &compiler.input_authority.sysroot,
            &private_registry,
        )
        .unwrap(),
        std::ffi::OsString::new()
    );
    command.env("CARGO_ENCODED_RUSTFLAGS", &kernel_rustflags);
    let environment = command
        .get_envs()
        .map(|(name, value)| (name.to_os_string(), value.map(ToOwned::to_owned)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        environment.get(std::ffi::OsStr::new("RUSTC")),
        Some(&Some(compiler.path.clone().into_os_string()))
    );
    for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
        assert_eq!(
            environment.get(std::ffi::OsStr::new(wrapper)),
            Some(&Some(std::ffi::OsString::new()))
        );
    }
    assert_eq!(
        environment.get(std::ffi::OsStr::new("CARGO_HOME")),
        Some(&Some(private_home.into_os_string()))
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("PATH")),
        Some(&Some(compiler.tool_path.clone()))
    );
    for ambient in [
        "GCC_EXEC_PREFIX",
        "LIBRARY_PATH",
        "CPATH",
        "C_INCLUDE_PATH",
        "CPLUS_INCLUDE_PATH",
        "LD_RUN_PATH",
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "CCC_OVERRIDE_OPTIONS",
        "CC",
        "CFLAGS",
        "AR",
        "TARGET_CC",
        "HOST_CC",
        "CRATE_CC_NO_DEFAULTS",
        "CXX",
        "CXXFLAGS",
    ] {
        assert_eq!(
            environment.get(std::ffi::OsStr::new(ambient)),
            Some(&None),
            "{ambient} reached the audit's cargo subprocess"
        );
    }
    assert_eq!(
        environment.get(std::ffi::OsStr::new("COMPILER_PATH")),
        compiler
            .link_editor
            .as_ref()
            .map(|link_editor| Some(link_editor.search_directory.clone().into_os_string()))
            .as_ref()
    );
    let linker = format!(
        "target.{}.linker={:?}",
        compiler.host_target,
        compiler.host_linker.display().to_string()
    );
    assert!(
        command
            .get_args()
            .any(|argument| argument == linker.as_str())
    );
    assert_eq!(
        environment.get(std::ffi::OsStr::new("CARGO_ENCODED_RUSTFLAGS")),
        Some(&Some(kernel_rustflags))
    );
    assert!(command.get_args().any(|argument| {
        argument
            .to_string_lossy()
            .starts_with("source.api-audit-registry.directory=")
    }));
}

#[test]
fn checksum_verified_archives_form_an_immutable_isolated_cargo_source() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-private-cargo");
    let cargo = root.join("audit-cargo");
    let home = cargo.join("home");
    let registry = cargo.join("registry-source");
    std::fs::create_dir_all(root.join("src")).expect("private-Cargo fixture source");
    std::fs::create_dir_all(&home).expect("private-Cargo home");
    std::fs::create_dir_all(&registry).expect("private-Cargo directory source");
    std::fs::write(home.join("config.toml"), "\n").expect("empty private config");
    std::fs::create_dir_all(root.join(".cargo")).expect("hostile ancestor config directory");
    std::fs::write(
        root.join(".cargo/config.toml"),
        "[build]\nrustflags = [\"--cfg\", \"manufactured\"]\n",
    )
    .expect("hostile ancestor config");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nanyhow = \"=1.0.104\"\n",
    )
    .expect("private-Cargo fixture manifest");
    std::fs::write(
        root.join("Cargo.lock"),
        r#"# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "anyhow"
version = "1.0.104"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470"

[[package]]
name = "fixture"
version = "0.1.0"
dependencies = [
 "anyhow",
]
"#,
    )
    .expect("private-Cargo fixture lockfile");
    std::fs::write(
        root.join("src/lib.rs"),
        "#[cfg(manufactured)] compile_error!(\"ancestor Cargo config escaped isolation\");\npub fn live() {}\n",
    )
    .expect("private-Cargo fixture library");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root");
    let compiler = compiler_identity(repository).expect("adjacent compiler identity");
    let execution = cargo_execution_root(&root).expect("filesystem-root execution boundary");
    materialize_locked_registry_source(&root, &home, &registry, &execution, &compiler)
        .expect("materialize verified archive");
    let baseline =
        cargo_input_snapshot(&cargo, &home, &registry).expect("private Cargo authority baseline");
    let output = audited_cargo_command(&execution, &home, &registry, &compiler)
        .expect("isolated Cargo check")
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env("CARGO_ENCODED_RUSTFLAGS", "")
        .args(["check", "--offline", "--locked"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("run isolated Cargo check");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        cargo_input_snapshot(&cargo, &home, &registry).expect("unchanged private Cargo authority"),
        baseline
    );
    let package = locked_registry_packages(&root.join("Cargo.lock"))
        .expect("fixture locked package")
        .pop()
        .expect("one registry package");
    let library = registry.join("anyhow-1.0.104").join("src/lib.rs");
    let mut tampered = std::fs::read_to_string(&library).expect("materialized library");
    tampered.push_str("\n// tampered\n");
    std::fs::write(&library, tampered).expect("tamper materialized source");
    assert!(
        validate_materialized_registry_package(&registry.join("anyhow-1.0.104"), &package).is_err()
    );
    std::fs::remove_dir_all(root).expect("remove private-Cargo fixture");
}

#[test]
fn authenticated_registry_source_cannot_mint_a_diagnostic_marker() {
    for (label, source) in [
        (
            "raw marker",
            format!("#[deprecated(note = \"{MARKER}0\")]\npub fn forged() {{}}\n"),
        ),
        (
            "decoded marker",
            "#[deprecated(note = \"wizard-api-audit:\\x30\")]\npub fn forged() {}\n".to_string(),
        ),
    ] {
        let root = crate::test_support::reserve_temp_directory("wizard-api-registry-marker");
        let package_root = root.join("fixture-1.0.0");
        std::fs::create_dir_all(package_root.join("src"))
            .expect("registry marker fixture directory");
        let manifest = b"[package]\nname = \"fixture\"\nversion = \"1.0.0\"\n";
        std::fs::write(package_root.join("Cargo.toml"), manifest)
            .expect("registry marker manifest");
        std::fs::write(package_root.join("src/lib.rs"), source.as_bytes())
            .expect("registry marker source");
        let manifest_digest = crate::sha256::hex(&crate::sha256::digest(manifest));
        let source_digest = crate::sha256::hex(&crate::sha256::digest(source.as_bytes()));
        let package_digest = "00".repeat(32);
        std::fs::write(
            package_root.join(".cargo-checksum.json"),
            format!(
                "{{\"$comment\":\"fixture\",\"files\":{{\"Cargo.toml\":\"{manifest_digest}\",\"src/lib.rs\":\"{source_digest}\"}},\"package\":\"{package_digest}\"}}"
            ),
        )
        .expect("registry checksum metadata");
        let package = LockedRegistryPackage {
            name: "fixture".to_string(),
            version: "1.0.0".to_string(),
            checksum: package_digest,
        };

        let error =
            validate_materialized_registry_package(&package_root, &package).expect_err(label);
        assert!(
            format!("{error:#}").contains("reserved diagnostic marker"),
            "{label}: {error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove registry marker fixture");
    }
}

#[test]
fn redirected_api_audit_paths_are_refused_without_touching_outside_bytes() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-path-fixture");
    let outside = root.join("outside");
    let target = root.join("target-link");
    std::fs::create_dir_all(&outside).unwrap();
    create_directory_symlink(&outside, &target);
    let paths = Paths::new(root.join("repository"), Some(&target));
    assert!(prepare_audit_target(&paths).is_err());
    assert!(!outside.join("api-audit").exists());
    std::fs::remove_dir_all(&root).unwrap();

    for redirected in ["workspace", "build"] {
        let root = crate::test_support::reserve_temp_directory(&format!(
            "wizard-api-{redirected}-fixture"
        ));
        let target = root.join("target");
        let state = target.join("api-audit");
        let outside = root.join("outside");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep"), "outside").unwrap();
        create_directory_symlink(&outside, &state.join(redirected));

        assert!(reset_audit_directory(&state, redirected).is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("keep")).unwrap(),
            "outside"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    let root = crate::test_support::reserve_temp_directory("wizard-api-lock-fixture");
    let target = root.join("target");
    let state = target.join("api-audit");
    let outside = root.join("outside-lock");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(&outside, "outside").unwrap();
    create_file_symlink(&outside, &state.join("LOCK"));

    assert!(open_audit_lock(&state).is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn api_audit_copy_never_writes_through_an_external_link() {
    let root = crate::test_support::reserve_temp_directory("wizard-api-copy-link");
    let source = root.join("source");
    let workspace = root.join("workspace");
    let sentinel = root.join("outside-sentinel");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(source.join("input"), b"copied source").unwrap();
    std::fs::write(&sentinel, b"outside authority").unwrap();

    let copied = workspace.join("copied");
    std::fs::hard_link(&sentinel, &copied).unwrap();
    let hardlink = copy_file(&source.join("input"), &copied);
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"outside authority",
        "API audit copy truncated an external hardlink"
    );
    assert!(hardlink.is_err(), "an existing copied file was accepted");
    std::fs::remove_file(&copied).unwrap();

    create_file_symlink(&sentinel, &copied);
    let symlink = copy_file(&source.join("input"), &copied);
    assert_eq!(
        std::fs::read(&sentinel).unwrap(),
        b"outside authority",
        "API audit copy followed an external symlink"
    );
    assert!(symlink.is_err(), "a redirected copied file was accepted");
    std::fs::remove_file(&copied).unwrap();

    let redirected_source = source.join("redirected-source");
    let copied_redirected_source = workspace.join("copied-redirected-source");
    create_file_symlink(&sentinel, &redirected_source);
    assert!(
        copy_file(&redirected_source, &copied_redirected_source).is_err(),
        "a top-level redirected source file was accepted"
    );
    assert!(!copied_redirected_source.exists());
    std::fs::remove_file(&redirected_source).unwrap();

    let outside_directory = root.join("outside-directory");
    let redirected_directory = workspace.join("redirected");
    std::fs::create_dir(&outside_directory).unwrap();

    let redirected_source_directory = source.join("redirected-directory");
    let copied_redirected_directory = workspace.join("copied-redirected-directory");
    create_directory_symlink(&outside_directory, &redirected_source_directory);
    assert!(
        copy_tree(&redirected_source_directory, &copied_redirected_directory).is_err(),
        "a top-level redirected source directory was accepted"
    );
    assert!(!copied_redirected_directory.exists());
    crate::test_support::remove_directory_link(&redirected_source_directory);

    create_directory_symlink(&outside_directory, &redirected_directory);
    assert!(copy_tree(&source, &redirected_directory).is_err());
    assert!(
        !outside_directory.join("input").exists(),
        "API audit recursion wrote through a redirected directory"
    );
    crate::test_support::remove_directory_link(&redirected_directory);

    copy_file(&source.join("input"), &copied).unwrap();
    assert_eq!(std::fs::read(&copied).unwrap(), b"copied source");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn api_authority_has_no_unbounded_production_file_reads() {
    let production = PRODUCTION
        .split_once("#[cfg(test)]\nmod tests")
        .map(|(production, _)| production)
        .expect("API audit production boundary");

    assert!(
        !production.contains("std::fs::read("),
        "API audit production still contains an unbounded byte read"
    );
    assert!(
        !production.contains("std::fs::read_to_string("),
        "API audit production still contains an unbounded text read"
    );
}

#[test]
fn source_and_cargo_snapshots_stream_plain_digests() {
    let source = PRODUCTION;
    let boundary = source
        .split_once("fn snapshot_file(")
        .and_then(|(_, source)| source.split_once("fn snapshot_marker_free_file("))
        .map(|(source, _)| source)
        .expect("plain snapshot file boundary");

    assert!(
        boundary.contains("stream_file_digest(path, \"API audit snapshot file\")?"),
        "plain source and Cargo snapshots do not stream their digest"
    );
}

#[test]
fn registry_archives_are_hashed_as_streams() {
    let source = PRODUCTION;
    let boundary = source
        .split_once("fn materialize_locked_registry_source(")
        .and_then(|(_, source)| source.split_once("fn registry_roots("))
        .map(|(source, _)| source)
        .expect("registry archive materialization boundary");

    assert!(
        boundary.contains("stream_file_digest(archive, \"API audit registry archive\")?"),
        "registry archives do not stream their authenticated digest"
    );
}

#[test]
fn registry_checksum_metadata_has_a_fixed_allocation_ceiling() {
    let source = PRODUCTION;
    let boundary = source
        .split_once("fn validate_materialized_registry_package(")
        .and_then(|(_, source)| source.split_once("fn collect_materialized_files("))
        .map(|(source, _)| source)
        .expect("materialized registry validation boundary");

    assert!(
        boundary.contains("read_bounded_authority_file("),
        "registry checksum metadata has no fixed allocation ceiling"
    );
}

#[test]
fn authenticated_registry_files_stream_digest_and_marker_checks() {
    let source = PRODUCTION;
    let boundary = source
        .split_once("fn validate_materialized_registry_package(")
        .and_then(|(_, source)| source.split_once("fn collect_materialized_files("))
        .map(|(source, _)| source)
        .expect("materialized registry validation boundary");

    assert!(
        boundary.contains(
            "stream_marker_free_digest(&path, \"authenticated registry compiler input\")?"
        ),
        "authenticated registry inputs do not combine streamed digest and marker checks"
    );
}

#[test]
fn authority_semantic_reads_enforce_a_fixed_allocation_ceiling() {
    let root = fixture("");
    let path = root.join("oversized-authority.txt");
    let file = std::fs::File::create(&path).expect("create oversized authority fixture");
    file.set_len(MAX_COMPILER_AUTHORITY_TEXT_BYTES as u64 + 1)
        .expect("size oversized authority fixture");

    let error = read_bounded_authority_text(&path, "test compiler authority")
        .expect_err("oversized semantic authority must fail before allocation");
    assert!(format!("{error:#}").contains("allocation limit"));

    std::fs::remove_dir_all(root).expect("remove authority allocation fixture");
}

#[test]
fn streamed_compiler_authority_preserves_digest_identity() {
    let root = fixture("");
    let authority = root.join("compiler-runtime.txt");
    let bytes = b"marker-free compiler authority";
    std::fs::write(&authority, bytes).expect("write compiler authority");
    assert_eq!(
        stream_marker_free_digest(&authority, "test compiler authority")
            .expect("stream compiler authority"),
        crate::sha256::digest(bytes)
    );

    std::fs::remove_dir_all(root).expect("remove compiler authority digest fixture");
}

#[test]
fn streamed_compiler_authority_admits_marker_free_non_rust_binary() {
    let root = fixture("");
    let binary = root.join("compiler-runtime.dll");
    let bytes = [0xff, 1, 2, 3, 4, 5];
    std::fs::write(&binary, bytes).expect("write binary compiler authority");

    assert!(
        stream_marker_free_digest_with_limit(&binary, "test compiler authority", 2).is_ok(),
        "marker-free non-Rust binary compiler authority must remain admissible"
    );

    std::fs::remove_dir_all(root).expect("remove binary compiler authority fixture");
}

#[test]
fn streamed_compiler_authority_accepts_utf8_split_across_chunks() {
    let root = fixture("");
    let source = root.join("split-utf8.rs");
    let mut bytes = b"//".to_vec();
    bytes.resize(COMPILER_AUTHORITY_BUFFER_BYTES - 1, b'x');
    bytes.extend_from_slice("é\n".as_bytes());
    std::fs::write(&source, &bytes).expect("write split UTF-8 compiler authority");

    assert_eq!(
        stream_marker_free_digest(&source, "test compiler authority")
            .expect("stream valid UTF-8 split across chunks"),
        crate::sha256::digest(&bytes)
    );

    std::fs::remove_dir_all(root).expect("remove split UTF-8 compiler authority fixture");
}

#[test]
fn streamed_compiler_authority_rejects_oversized_valid_utf8() {
    let root = fixture("");
    let text = root.join("compiler-runtime.txt");
    std::fs::write(&text, b"valid text").expect("write text compiler authority");
    let oversized =
        stream_marker_free_digest_with_limit(&text, "test compiler authority", b"valid".len())
            .expect_err("oversized valid text must fail closed");
    assert!(format!("{oversized:#}").contains("semantic scan ceiling"));

    std::fs::remove_dir_all(root).expect("remove text compiler authority fixture");
}

#[test]
fn streamed_compiler_authority_rejects_cross_chunk_raw_markers() {
    let root = fixture("");
    let marked = root.join("marked-runtime.dll");
    let mut marked_bytes = vec![b'x'; COMPILER_AUTHORITY_BUFFER_BYTES - 2];
    marked_bytes.extend_from_slice(MARKER.as_bytes());
    marked_bytes.extend_from_slice(b"7\xff");
    std::fs::write(&marked, marked_bytes).expect("write marked compiler authority");
    let marked = stream_marker_free_digest_with_limit(
        &marked,
        "test compiler authority",
        MAX_COMPILER_AUTHORITY_TEXT_BYTES,
    )
    .expect_err("a cross-chunk raw marker must be refused");
    assert!(format!("{marked:#}").contains("reserved diagnostic marker"));

    std::fs::remove_dir_all(root).expect("remove marked compiler authority fixture");
}
