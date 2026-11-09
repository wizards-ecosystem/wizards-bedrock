use super::*;

#[test]
fn release_is_the_default_profile() {
    assert_eq!(Profile::default(), Profile::Release);
}

#[test]
fn package_args_pairs_each_name_with_a_flag() {
    assert_eq!(
        package_args(&["wost", "wizard-kernel"]),
        ["--package", "wost", "--package", "wizard-kernel"]
    );
}

#[test]
fn kernel_builds_are_locked_to_the_recorded_dependencies() {
    for args in [
        kernel_build_args(Profile::Dev, &[]),
        kernel_build_args(Profile::Release, &[paths::SELFTEST_FEATURE]),
    ] {
        assert_eq!(
            args.iter().filter(|arg| arg.as_str() == "--locked").count(),
            1
        );
    }
}

#[test]
fn a_cargo_command_uses_the_paths_private_target() {
    let paths = Paths::new(
        PathBuf::from("workspace"),
        Some(std::path::Path::new("private-build")),
    );
    let command = command(&paths);
    assert_eq!(command.get_current_dir(), Some(paths.root()));
    assert!(command.get_envs().any(|(name, value)| {
        name == "CARGO_TARGET_DIR" && value == Some(paths.target_dir().as_os_str())
    }));
}

#[test]
fn kernel_builds_use_the_exact_codegen_and_virtual_path_contract() {
    let source = &crate::test_support::absolute_root("source checkout");
    let toolchain = &crate::test_support::absolute_root("rust toolchain");
    let cargo_home = &crate::test_support::absolute_root("cargo home");
    let encoded = kernel_encoded_rustflags(source, toolchain, cargo_home).unwrap();
    assert_eq!(
        encoded,
        OsString::from(format!(
            "-C\u{1f}code-model=kernel\u{1f}-C\u{1f}relocation-model=static\u{1f}\
                 --remap-path-prefix\u{1f}{}=/wizard/source\u{1f}\
                 --remap-path-prefix\u{1f}{}=/wizard/toolchain\u{1f}\
                 --remap-path-prefix\u{1f}{}=/wizard/cargo",
            source.display(),
            toolchain.display(),
            cargo_home.display()
        ))
    );
    assert!(kernel_encoded_rustflags(source, source, cargo_home).is_err());
    assert!(kernel_encoded_rustflags(Path::new("bad=root"), toolchain, cargo_home).is_err());

    let paths = Paths::new(
        source.to_path_buf(),
        Some(&crate::test_support::absolute_root("target")),
    );
    let toolchain = &pinned_toolchain(&["rustc", "rustdoc"]);
    let command = kernel_command_with_roots(
        &paths,
        [
            source.to_path_buf(),
            toolchain.to_path_buf(),
            cargo_home.to_path_buf(),
        ],
    )
    .unwrap();
    for (name, suffix) in [("RUSTC", "rustc"), ("RUSTDOC", "rustdoc")] {
        assert!(command.get_envs().any(|(observed, value)| {
            observed == name
                && value.is_some_and(|value| {
                    value.to_string_lossy().contains("rust toolchain")
                        && value.to_string_lossy().contains(suffix)
                })
        }));
    }
    for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
        assert!(
            command
                .get_envs()
                .any(|(observed, value)| { observed == wrapper && value == Some(OsStr::new("")) })
        );
    }
    let path = command
        .get_envs()
        .find_map(|(name, value)| (name == "PATH").then_some(value).flatten())
        .unwrap();
    assert_eq!(
        std::env::split_paths(path).next().as_deref(),
        Some(toolchain.join("bin").as_path())
    );
    std::fs::remove_dir_all(toolchain).unwrap();
}

fn pinned_toolchain(compilers: &[&str]) -> PathBuf {
    let root = crate::test_support::reserve_temp_directory("rust toolchain");
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    for compiler in compilers {
        std::fs::write(bin.join(crate::qemu::exe_name(compiler)), b"compiler").unwrap();
    }
    root
}

#[test]
fn a_missing_pinned_compiler_is_named_before_cargo_starts() {
    let source = crate::test_support::absolute_root("source checkout");
    let cargo_home = crate::test_support::absolute_root("cargo home");
    let paths = Paths::new(source.clone(), None);
    for (present, missing) in [("rustdoc", "rustc"), ("rustc", "rustdoc")] {
        let toolchain = pinned_toolchain(&[present]);
        let error = kernel_command_with_roots(
            &paths,
            [source.clone(), toolchain.clone(), cargo_home.clone()],
        )
        .unwrap_err()
        .to_string();
        let missing = toolchain.join("bin").join(crate::qemu::exe_name(missing));
        assert!(error.contains(&missing.display().to_string()), "{error}");
        std::fs::remove_dir_all(toolchain).unwrap();
    }
}

#[cfg(windows)]
#[test]
fn a_cargo_command_sheds_the_windows_verbatim_target_prefix() {
    let paths = Paths::new(
        PathBuf::from(r"\\?\C:\repo"),
        Some(std::path::Path::new(r"\\?\C:\repo\private-build")),
    );
    let command = command(&paths);
    assert!(command.get_envs().any(|(name, value)| {
        name == "CARGO_TARGET_DIR" && value == Some(std::ffi::OsStr::new(r"C:\repo\private-build"))
    }));
}

#[cfg(windows)]
#[test]
fn a_cargo_command_sheds_the_windows_verbatim_unc_target_prefix() {
    let paths = Paths::new(
        PathBuf::from(r"\\?\UNC\server\share\repo"),
        Some(std::path::Path::new(
            r"\\?\UNC\server\share\repo\private-build",
        )),
    );
    let command = command(&paths);
    assert!(command.get_envs().any(|(name, value)| {
        name == "CARGO_TARGET_DIR"
            && value == Some(std::ffi::OsStr::new(r"\\server\share\repo\private-build"))
    }));
}
