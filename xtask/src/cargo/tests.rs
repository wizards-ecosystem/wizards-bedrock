use super::*;

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
