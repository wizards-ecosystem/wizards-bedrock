use super::*;

#[derive(Clone, Debug)]
pub(super) struct CompilerIdentity {
    pub(super) path: PathBuf,
    pub(super) host_target: String,
    pub(super) host_linker: PathBuf,
    pub(super) link_editor: Option<LinkEditor>,
    pub(super) tool_path: std::ffi::OsString,
    compiler_digest: [u8; 32],
    linker_digest: [u8; 32],
    pub(super) input_authority: CompilerInputAuthority,
}

#[derive(Clone, Debug)]
pub(super) struct LinkEditor {
    pub(super) search_directory: PathBuf,
    path: PathBuf,
    digest: [u8; 32],
}

#[derive(Clone, Debug)]
pub(super) struct CompilerInputAuthority {
    pub(super) sysroot: PathBuf,
    cargo: PathBuf,
    compiler: PathBuf,
    targets: Vec<String>,
    runtime_snapshot: BTreeMap<PathBuf, [u8; 32]>,
    target_library_snapshot: BTreeMap<PathBuf, [u8; 32]>,
}

fn digest_compiler_executable(path: &Path, label: &str) -> Result<[u8; 32]> {
    stream_file_digest(path, label)
}

pub(super) fn stream_file_digest(path: &Path, label: &str) -> Result<[u8; 32]> {
    let file = crate::mutate::workspace::open_real_file(path, label)?;
    let (_, digest) = crate::sha256::digest_reader(file)
        .with_context(|| format!("reading {label} {}", path.display()))?;
    Ok(digest)
}

pub(super) fn read_bounded_authority_file(path: &Path, label: &str) -> Result<Vec<u8>> {
    crate::qualification::read_bounded_file(path, MAX_COMPILER_AUTHORITY_TEXT_BYTES as u64, label)
}

pub(super) fn read_bounded_authority_text(path: &Path, label: &str) -> Result<String> {
    String::from_utf8(read_bounded_authority_file(path, label)?)
        .with_context(|| format!("{label} {} is not UTF-8", path.display()))
}

pub(super) fn host_linker(
    host_target: &str,
    sysroot: &Path,
    search_path: Option<&std::ffi::OsStr>,
) -> Result<(PathBuf, Option<PathBuf>)> {
    if !host_target.contains("-windows-") {
        return platform_c_driver(search_path);
    }
    let linker = std::fs::canonicalize(
        sysroot
            .join("lib/rustlib")
            .join(host_target)
            .join("bin")
            .join("rust-lld.exe"),
    )
    .context("resolving the API audit host linker inside the compiler sysroot")?;
    ensure!(
        linker.starts_with(sysroot),
        "the API audit host linker escaped the compiler sysroot"
    );
    Ok((linker, None))
}

fn platform_c_driver(search_path: Option<&std::ffi::OsStr>) -> Result<(PathBuf, Option<PathBuf>)> {
    let directory = search_path
        .into_iter()
        .flat_map(std::env::split_paths)
        .filter(|directory| directory.is_absolute())
        .find(|directory| directory.join("cc").is_file() && directory.join("ld").is_file())
        .context(
            "the API audit host linker `cc` and its `ld` are not in one absolute PATH directory",
        )?;
    let driver = directory.join("cc");
    let driver = std::fs::canonicalize(&driver)
        .with_context(|| format!("resolving the API audit host linker {}", driver.display()))?;
    Ok((driver, Some(directory)))
}

pub(super) fn compiler_identity(source_root: &Path) -> Result<CompilerIdentity> {
    let cargo = std::fs::canonicalize(env!("CARGO"))
        .with_context(|| format!("resolving API audit Cargo executable {}", env!("CARGO")))?;
    let compiler_name = if cfg!(windows) { "rustc.exe" } else { "rustc" };
    let compiler = cargo
        .parent()
        .context("the API audit Cargo executable has no toolchain directory")?
        .join(compiler_name);
    let compiler = std::fs::canonicalize(&compiler).with_context(|| {
        format!(
            "resolving the API audit compiler beside Cargo at {}",
            compiler.display()
        )
    })?;
    let mut version_command = Command::new(&compiler);
    version_command.current_dir(source_root).arg("-vV");
    let result = crate::bounded_process::output_with_limit(
        version_command,
        TOOL_IDENTITY_TIMEOUT,
        "API audit compiler identity",
        MAX_TOOL_IDENTITY_BYTES,
    )
    .context("querying the API audit compiler host target")?;
    ensure!(
        result.status.success(),
        "querying the API audit compiler host target failed ({}): {}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    let version = std::str::from_utf8(&result.stdout)
        .context("the API audit compiler version is not UTF-8")?;
    let targets = version
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect::<Vec<_>>();
    let [target] = targets.as_slice() else {
        bail!("the API audit compiler did not report exactly one host target");
    };
    ensure!(
        !target.is_empty()
            && target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "the API audit compiler returned malformed host target `{target}`"
    );
    let mut sysroot_command = Command::new(&compiler);
    sysroot_command
        .current_dir(source_root)
        .args(["--print", "sysroot"]);
    let sysroot = crate::bounded_process::output_with_limit(
        sysroot_command,
        TOOL_IDENTITY_TIMEOUT,
        "API audit compiler sysroot",
        MAX_TOOL_IDENTITY_BYTES,
    )
    .context("querying the API audit compiler sysroot")?;
    ensure!(
        sysroot.status.success(),
        "querying the API audit compiler sysroot failed ({}): {}",
        sysroot.status,
        String::from_utf8_lossy(&sysroot.stderr)
    );
    let sysroot = std::str::from_utf8(&sysroot.stdout)
        .context("the API audit compiler sysroot is not UTF-8")?
        .trim();
    ensure!(
        !sysroot.is_empty(),
        "the API audit compiler returned an empty sysroot"
    );
    let sysroot = std::fs::canonicalize(sysroot)
        .with_context(|| format!("canonicalizing compiler sysroot `{sysroot}`"))?;
    let toolchain = compiler
        .parent()
        .and_then(Path::parent)
        .context("the API audit compiler is outside a toolchain root")?;
    ensure!(
        sysroot == toolchain,
        "the API audit compiler reported a sysroot outside its exact toolchain"
    );
    let (host_linker, link_editor_directory) =
        host_linker(target, &sysroot, std::env::var_os("PATH").as_deref())?;
    let host_linker = crate::cargo::child_process_path(&host_linker);
    let tool_path = std::env::join_paths([compiler
        .parent()
        .context("the API audit compiler has no binary directory")?])
    .context("constructing the API audit tool PATH")?;
    let compiler_digest = digest_compiler_executable(&compiler, "compiler")?;
    let linker_digest = digest_compiler_executable(&host_linker, "linker")?;
    let link_editor = link_editor_directory
        .map(|search_directory| -> Result<_> {
            let path = search_directory.join("ld");
            let path = std::fs::canonicalize(&path).with_context(|| {
                format!("resolving the API audit link editor {}", path.display())
            })?;
            let digest = digest_compiler_executable(&path, "link editor")?;
            Ok(LinkEditor {
                search_directory,
                path,
                digest,
            })
        })
        .transpose()?;
    let input_authority = CompilerInputAuthority::capture(&sysroot, &cargo, &compiler, target)?;
    Ok(CompilerIdentity {
        path: compiler,
        host_target: target.to_string(),
        host_linker,
        link_editor,
        tool_path,
        compiler_digest,
        linker_digest,
        input_authority,
    })
}

impl CompilerInputAuthority {
    fn capture(sysroot: &Path, cargo: &Path, compiler: &Path, host_target: &str) -> Result<Self> {
        let sysroot = std::fs::canonicalize(sysroot)
            .with_context(|| format!("canonicalizing compiler sysroot {}", sysroot.display()))?;
        let cargo = std::fs::canonicalize(cargo)
            .with_context(|| format!("canonicalizing Cargo executable {}", cargo.display()))?;
        let compiler = std::fs::canonicalize(compiler).with_context(|| {
            format!("canonicalizing compiler executable {}", compiler.display())
        })?;
        ensure!(
            cargo.starts_with(&sysroot) && compiler.starts_with(&sysroot),
            "Cargo or rustc escaped the exact compiler sysroot"
        );
        let mut targets = [host_target, crate::paths::KERNEL_TARGET]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        targets.sort();
        targets.dedup();
        let runtime_snapshot = compiler_runtime_snapshot(&sysroot, &cargo, &compiler, &targets)?;
        let target_library_snapshot = compiler_target_library_snapshot(&sysroot, &targets)?;
        Ok(Self {
            sysroot,
            cargo,
            compiler,
            targets,
            runtime_snapshot,
            target_library_snapshot,
        })
    }

    fn require_unchanged(&self) -> Result<()> {
        self.require_runtime_unchanged()?;
        self.require_target_libraries_unchanged()
    }

    fn require_runtime_unchanged(&self) -> Result<()> {
        let current =
            compiler_runtime_snapshot(&self.sysroot, &self.cargo, &self.compiler, &self.targets)?;
        require_compiler_snapshot_unchanged(&self.runtime_snapshot, &current)
    }

    fn require_target_libraries_unchanged(&self) -> Result<()> {
        let current = compiler_target_library_snapshot(&self.sysroot, &self.targets)?;
        require_compiler_snapshot_unchanged(&self.target_library_snapshot, &current)
    }
}

pub(super) fn require_compiler_snapshot_unchanged(
    expected: &BTreeMap<PathBuf, [u8; 32]>,
    current: &BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    if current == expected {
        return Ok(());
    }
    let changed = expected
        .keys()
        .chain(current.keys())
        .find(|path| expected.get(*path) != current.get(*path))
        .context("compiler authority changed without naming an input")?;
    bail!(
        "compiler input {} changed while the API audit was running",
        changed.display()
    )
}

pub(super) fn compiler_runtime_snapshot(
    sysroot: &Path,
    cargo: &Path,
    compiler: &Path,
    targets: &[String],
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutate::workspace::canonical_existing_real_directory(
        sysroot,
        "API audit compiler sysroot",
    )?;
    let mut snapshot = BTreeMap::new();
    for executable in [cargo, compiler] {
        ensure!(
            executable.starts_with(sysroot),
            "compiler executable {} escaped the exact sysroot",
            executable.display()
        );
        snapshot_marker_free_file(
            sysroot,
            executable,
            &mut snapshot,
            "compiler executable authority",
        )?;
    }

    let bin = sysroot.join("bin");
    crate::mutate::workspace::canonical_existing_real_directory(
        &bin,
        "API audit compiler runtime directory",
    )?;
    for entry in std::fs::read_dir(&bin)
        .with_context(|| format!("reading compiler runtime directory {}", bin.display()))?
    {
        let entry = entry.with_context(|| format!("reading an entry in {}", bin.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            continue;
        }
        let runtime_library = entry.path().extension().is_some_and(|extension| {
            matches!(
                extension.to_string_lossy().to_ascii_lowercase().as_str(),
                "dll" | "dylib" | "so"
            )
        });
        ensure!(
            kind.is_file() || !runtime_library,
            "compiler runtime authority {} is not a regular file",
            entry.path().display()
        );
        if runtime_library {
            snapshot_marker_free_file(
                sysroot,
                &entry.path(),
                &mut snapshot,
                "compiler runtime authority",
            )?;
        }
    }

    let top_level_lib = sysroot.join("lib");
    crate::mutate::workspace::canonical_existing_real_directory(
        &top_level_lib,
        "API audit compiler library directory",
    )?;
    for entry in std::fs::read_dir(&top_level_lib).with_context(|| {
        format!(
            "reading compiler library directory {}",
            top_level_lib.display()
        )
    })? {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", top_level_lib.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_file() {
            snapshot_marker_free_file(
                sysroot,
                &entry.path(),
                &mut snapshot,
                "compiler runtime authority",
            )?;
        } else {
            ensure!(
                kind.is_dir(),
                "compiler runtime authority {} is not a regular file or directory",
                entry.path().display()
            );
        }
    }

    snapshot_compiler_target_runtime(sysroot, targets, &mut snapshot)?;
    Ok(snapshot)
}

fn snapshot_compiler_target_runtime(
    sysroot: &Path,
    targets: &[String],
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    for target in targets {
        let target_root = sysroot.join("lib/rustlib").join(target);
        crate::mutate::workspace::canonical_existing_real_directory(
            &target_root,
            "API audit compiler target root",
        )?;
        for entry in std::fs::read_dir(&target_root)
            .with_context(|| format!("reading compiler target root {}", target_root.display()))?
        {
            let entry =
                entry.with_context(|| format!("reading an entry in {}", target_root.display()))?;
            if entry.file_name() == "lib" {
                continue;
            }
            let kind = entry
                .file_type()
                .with_context(|| format!("reading the type of {}", entry.path().display()))?;
            if kind.is_dir() {
                snapshot_marker_free_tree(
                    sysroot,
                    &entry.path(),
                    snapshot,
                    "compiler target runtime authority",
                )?;
            } else if kind.is_file() {
                snapshot_marker_free_file(
                    sysroot,
                    &entry.path(),
                    snapshot,
                    "compiler target runtime authority",
                )?;
            } else {
                bail!(
                    "compiler target runtime authority {} is not a regular file or directory",
                    entry.path().display()
                );
            }
        }
    }
    Ok(())
}

pub(super) fn compiler_target_library_snapshot(
    sysroot: &Path,
    targets: &[String],
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutate::workspace::canonical_existing_real_directory(
        sysroot,
        "API audit compiler sysroot",
    )?;
    let mut snapshot = BTreeMap::new();
    for target in targets {
        let target_lib = sysroot.join("lib/rustlib").join(target).join("lib");
        snapshot_marker_free_tree(
            sysroot,
            &target_lib,
            &mut snapshot,
            "compiler target library authority",
        )?;
    }
    Ok(snapshot)
}

impl CompilerIdentity {
    pub(super) fn require_unchanged(&self) -> Result<()> {
        ensure!(
            digest_compiler_executable(&self.path, "compiler")? == self.compiler_digest,
            "the API audit compiler changed while the audit was running"
        );
        ensure!(
            digest_compiler_executable(&self.host_linker, "linker")? == self.linker_digest,
            "the API audit host linker changed while the audit was running"
        );
        if let Some(link_editor) = &self.link_editor {
            ensure!(
                digest_compiler_executable(&link_editor.path, "link editor")? == link_editor.digest,
                "the API audit link editor changed while the audit was running"
            );
        }
        Ok(())
    }

    pub(super) fn require_input_authority_unchanged(&self) -> Result<()> {
        self.input_authority.require_unchanged()
    }

    pub(super) fn require_target_libraries_unchanged(&self) -> Result<()> {
        self.input_authority.require_target_libraries_unchanged()
    }
}
