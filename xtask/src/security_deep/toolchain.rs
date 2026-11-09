use super::*;

pub(super) fn require_worker_probe_file(path: &Path, label: &str) -> Result<()> {
    ensure!(path.is_absolute(), "{label} path is not absolute");
    crate::mutate::workspace::open_real_file(path, label)?;
    Ok(())
}

pub(super) fn canonical_probe_path(path: &Path, label: &str) -> Result<PathBuf> {
    require_worker_probe_file(path, label)?;
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalizing {label} {}", path.display()))?;
    ensure!(canonical == path, "{label} is redirected");
    Ok(canonical)
}

pub(super) fn python_worker_policy_entries(
    runtime: &PythonRuntimeIdentity,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>)> {
    let root = canonical_directory(&runtime.root, "managed worker Python root")?;
    let mut reads = Vec::new();
    for name in ["python.exe", "python3.dll", "python312.dll"] {
        reads.push(canonical_probe_path(
            &root.join(name),
            "managed worker Python root file",
        )?);
    }
    for entry in bounded_sorted_directory_entries(&root, 64)? {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Python runtime root name is not UTF-8"))?;
        let lowercase = name.to_ascii_lowercase();
        if lowercase.starts_with("vcruntime140") && lowercase.ends_with(".dll") {
            reads.push(canonical_probe_path(
                &entry.path(),
                "managed worker Python runtime DLL",
            )?);
        }
    }
    let dlls = canonical_directory(&root.join("DLLs"), "managed worker Python DLL root")?;
    let lib = canonical_directory(&root.join("Lib"), "managed worker Python library root")?;
    reads.extend([dlls.clone(), lib.clone()]);
    let mut denies = Vec::new();
    collect_python_policy_exclusions(&dlls, 0, &mut denies)?;
    collect_python_policy_exclusions(&lib, 0, &mut denies)?;
    denies.sort();
    denies.dedup();
    let probes = vec![
        root.join("python.exe"),
        first_python_policy_file(&dlls, 0)?
            .context("managed worker Python DLL tree contains no probe file")?,
        first_python_policy_file(&lib, 0)?
            .context("managed worker Python library tree contains no probe file")?,
    ];
    ensure!(
        reads.iter().collect::<BTreeSet<_>>().len() == reads.len()
            && denies.iter().all(|denied| {
                reads.iter().any(|allowed| denied.starts_with(allowed))
                    && !reads.iter().any(|allowed| allowed.starts_with(denied))
            }),
        "managed worker Python policy is not a narrow identity-covered tree"
    );
    Ok((reads, denies, probes))
}

fn collect_python_policy_exclusions(
    directory: &Path,
    depth: usize,
    denied: &mut Vec<PathBuf>,
) -> Result<()> {
    ensure!(
        depth <= MAX_PYTHON_RUNTIME_DEPTH,
        "managed worker Python exclusion walk exceeds its depth limit"
    );
    let remaining = usize::try_from(MAX_PYTHON_RUNTIME_ENTRIES)
        .context("Python runtime entry limit does not fit usize")?
        .saturating_sub(denied.len());
    for entry in bounded_sorted_directory_entries(directory, remaining)? {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("inspecting Python policy entry {}", path.display()))?;
        if !crate::mutate::workspace::metadata_is_real_directory(&metadata) {
            continue;
        }
        let excluded = entry.file_name().to_str().is_some_and(|name| {
            name.eq_ignore_ascii_case("__pycache__") || name.eq_ignore_ascii_case("site-packages")
        });
        if excluded {
            denied.push(path);
        } else {
            collect_python_policy_exclusions(&path, depth + 1, denied)?;
        }
    }
    Ok(())
}

pub(super) fn first_python_policy_file(directory: &Path, depth: usize) -> Result<Option<PathBuf>> {
    ensure!(
        depth <= MAX_PYTHON_RUNTIME_DEPTH,
        "managed worker Python probe walk exceeds its depth limit"
    );
    for entry in bounded_sorted_directory_entries(directory, MAX_PYTHON_RUNTIME_ENTRIES as usize)? {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("inspecting Python probe entry {}", path.display()))?;
        if metadata.file_type().is_file() {
            return canonical_probe_path(&path, "managed worker Python probe").map(Some);
        }
        if crate::mutate::workspace::metadata_is_real_directory(&metadata)
            && !entry.file_name().to_str().is_some_and(|name| {
                name.eq_ignore_ascii_case("__pycache__")
                    || name.eq_ignore_ascii_case("site-packages")
            })
            && let Some(probe) = first_python_policy_file(&path, depth + 1)?
        {
            return Ok(Some(probe));
        }
    }
    Ok(None)
}

pub(super) fn require_worker_canaries(scratch: &OwnedScratch) -> Result<()> {
    ensure!(
        scratch.worker_allowed_canary == scratch.scan_root.join(WORKER_ALLOWED_CANARY_NAME)
            && scratch.worker_denied_canary == scratch.root.join(WORKER_DENIED_CANARY_NAME),
        "managed worker canary paths differ from their owned roots"
    );
    for (path, expected, label) in [
        (
            scratch.worker_allowed_canary.as_path(),
            WORKER_ALLOWED_CANARY,
            "managed worker allowed canary",
        ),
        (
            scratch.worker_denied_canary.as_path(),
            WORKER_DENIED_CANARY,
            "managed worker denied canary",
        ),
    ] {
        require_worker_probe_file(path, label)?;
        ensure!(
            read(path, 128, label)? == expected,
            "{label} content changed"
        );
    }
    Ok(())
}

pub(super) fn require_plugin_unchanged(package: &PluginPackage) -> Result<()> {
    ensure!(
        plugin_root()? == package.root,
        "installed Codex Security package path changed during execution"
    );
    let current = load_plugin_package()?;
    ensure!(
        current == *package,
        "installed Codex Security package changed during execution"
    );
    Ok(())
}

pub(super) fn resolve_toolchain(root: &Path, scratch: &OwnedScratch) -> Result<Toolchain> {
    let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    let local = canonical_directory(&PathBuf::from(local), "local application data")?;
    let node = identify_tool(
        &local
            .join("OpenAI")
            .join("Codex")
            .join("runtimes")
            .join("cua_node")
            .join("a5a02988362ca169")
            .join("bin")
            .join("node.exe"),
        &["--version"],
        NODE_BYTES,
        NODE_SHA256,
        NODE_VERSION,
        "Node",
        root,
        scratch,
    )?;
    let python_path = local
        .join("Programs")
        .join("Python")
        .join("Python312")
        .join("python.exe");
    let python_path = require_exact_executable_path(&python_path, "Python")?;
    let python_runtime_before = identify_python_runtime(&python_path)?;
    let python = identify_tool(
        &python_path,
        &["--version"],
        PYTHON_BYTES,
        PYTHON_SHA256,
        PYTHON_VERSION,
        "Python",
        root,
        scratch,
    )?;
    let python_runtime = identify_python_runtime(&python.path)?;
    ensure!(
        python_runtime_before == python_runtime
            && python.path.parent() == Some(python_runtime.root.as_path()),
        "qualified Python runtime changed during its version probe"
    );
    let codex_path = local
        .join("OpenAI")
        .join("Codex")
        .join("bin")
        .join("896a0166b774037a")
        .join("codex.exe");
    let codex_runtime_before = identify_codex_runtime(&codex_path)?;
    let codex = identify_tool(
        &codex_path,
        &["--version"],
        CODEX_BYTES,
        CODEX_SHA256,
        CODEX_VERSION,
        "Codex",
        root,
        scratch,
    )?;
    let codex_runtime = identify_codex_runtime(&codex.path)?;
    ensure!(
        codex_runtime_before == codex_runtime
            && codex.path.parent() == Some(codex_runtime.root.as_path()),
        "qualified Codex runtime changed during its version probe"
    );
    Ok(Toolchain {
        node,
        python,
        python_runtime,
        codex,
        codex_runtime,
    })
}

pub(super) fn identify_codex_runtime(executable: &Path) -> Result<CodexRuntimeIdentity> {
    let root = executable
        .parent()
        .context("Codex executable has no runtime root")?;
    let root = canonical_directory(root, "Codex runtime root")?;
    ensure!(
        executable == root.join("codex.exe"),
        "Codex executable is not the qualified runtime entrypoint"
    );
    let expected_names: BTreeSet<&str> = CODEX_RUNTIME_FILES
        .iter()
        .map(|(name, _, _)| *name)
        .collect();
    let mut actual_names = BTreeSet::new();
    for entry in std::fs::read_dir(&root).context("listing Codex runtime root")? {
        let entry = entry.context("reading Codex runtime root")?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Codex runtime entry is not UTF-8"))?;
        ensure!(
            actual_names.insert(name),
            "Codex runtime entry is duplicated"
        );
    }
    require_codex_runtime_inventory(&actual_names, &expected_names)?;

    let mut folded = sha256::Hasher::new();
    let mut bytes = 0u64;
    for (name, expected_bytes, expected_sha256) in CODEX_RUNTIME_FILES {
        let path = root.join(name);
        ensure!(
            require_exact_executable_path(&path, "Codex runtime executable")? == path,
            "Codex runtime executable is redirected"
        );
        let (length, digest) =
            measure_file(&path, MAX_EXECUTABLE_BYTES, "Codex runtime executable")?;
        require_codex_runtime_file_fact(name, length, &digest, expected_bytes, expected_sha256)?;
        bytes = bytes
            .checked_add(length)
            .context("Codex runtime byte count overflowed")?;
        folded.update(&[0])?;
        folded.update(&(name.len() as u64).to_be_bytes())?;
        folded.update(name.as_bytes())?;
        folded.update(&length.to_be_bytes())?;
        folded.update(&digest)?;
    }
    let identity = CodexRuntimeIdentity {
        root,
        entries: CODEX_RUNTIME_FILES.len() as u64,
        bytes,
        sha256: sha256::hex(&folded.finish()),
    };
    require_codex_runtime_identity(&identity)?;
    Ok(identity)
}

pub(super) fn require_codex_runtime_inventory(
    actual: &BTreeSet<String>,
    expected: &BTreeSet<&str>,
) -> Result<()> {
    ensure!(
        actual.iter().map(String::as_str).collect::<BTreeSet<_>>() == *expected,
        "Codex runtime tree differs from its exact four-file inventory"
    );
    Ok(())
}

pub(super) fn require_codex_runtime_file_fact(
    name: &str,
    length: u64,
    digest: &[u8; 32],
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<()> {
    ensure!(
        length == expected_bytes && sha256::hex(digest) == expected_sha256,
        "Codex runtime executable {name} differs from the qualified bytes"
    );
    Ok(())
}

pub(super) fn require_codex_runtime_identity(identity: &CodexRuntimeIdentity) -> Result<()> {
    ensure!(
        identity.entries == 4
            && identity.bytes == CODEX_RUNTIME_BYTES
            && identity.sha256 == CODEX_RUNTIME_SHA256,
        "Codex runtime tree identity differs from the qualified runtime"
    );
    Ok(())
}

pub(super) fn identify_python_runtime(executable: &Path) -> Result<PythonRuntimeIdentity> {
    let root = executable
        .parent()
        .context("Python executable has no runtime root")?;
    let root = canonical_directory(root, "Python runtime root")?;
    require_absent_python_startup_controls(&root)?;
    let mut selected = Vec::new();
    for name in ["python.exe", "python3.dll", "python312.dll"] {
        let path = root.join(name);
        ensure!(path.is_file(), "Python runtime is missing {name}");
        selected.push(path);
    }
    for entry in std::fs::read_dir(&root).context("listing Python runtime root")? {
        let entry = entry.context("reading Python runtime root")?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("Python runtime root name is not UTF-8")?;
        let lowercase = name.to_ascii_lowercase();
        if lowercase.starts_with("vcruntime140") && lowercase.ends_with(".dll") {
            selected.push(entry.path());
        }
    }
    ensure!(
        selected.iter().any(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.eq_ignore_ascii_case("vcruntime140.dll"))
        }),
        "Python runtime has no vcruntime140.dll"
    );
    selected.push(root.join("DLLs"));
    selected.push(root.join("Lib"));
    let mut entries = Vec::new();
    for path in selected {
        collect_python_runtime_entries(&root, &path, 0, &mut entries)?;
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries.dedup_by(|left, right| left.0 == right.0);
    let mut folded = sha256::Hasher::new();
    let mut bytes = 0u64;
    for (relative, path, is_directory) in &entries {
        folded.update(&[u8::from(*is_directory)])?;
        folded.update(&(relative.len() as u64).to_be_bytes())?;
        folded.update(relative.as_bytes())?;
        if !is_directory {
            let (length, digest) =
                measure_file(path, MAX_PLUGIN_FILE_BYTES, "Python runtime dependency")?;
            bytes = bytes
                .checked_add(length)
                .context("Python runtime byte count overflowed")?;
            ensure!(
                bytes <= MAX_PYTHON_RUNTIME_BYTES,
                "Python runtime exceeds its byte limit"
            );
            folded.update(&length.to_be_bytes())?;
            folded.update(&digest)?;
        }
    }
    Ok(PythonRuntimeIdentity {
        root,
        entries: entries.len() as u64,
        bytes,
        sha256: sha256::hex(&folded.finish()),
    })
}

fn require_absent_python_startup_controls(root: &Path) -> Result<()> {
    let parent = root.parent().context("Python runtime root has no parent")?;
    for directory in [root, parent] {
        for name in [
            "python312.zip",
            "python312._pth",
            "python._pth",
            "pyvenv.cfg",
        ] {
            let path = directory.join(name);
            ensure!(
                std::fs::symlink_metadata(&path)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
                "Python runtime has an unqualified startup control {}",
                path.display()
            );
        }
    }
    Ok(())
}

pub(super) fn installed_python_runtime() -> Result<PythonRuntimeIdentity> {
    let local = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    let local = canonical_directory(&PathBuf::from(local), "local application data")?;
    let supplied = local
        .join("Programs")
        .join("Python")
        .join("Python312")
        .join("python.exe");
    let python = std::fs::canonicalize(&supplied)
        .with_context(|| format!("canonicalizing qualified Python {}", supplied.display()))?;
    ensure!(
        python == supplied,
        "qualified Python executable is redirected"
    );
    let (bytes, digest) = measure_file(&python, MAX_EXECUTABLE_BYTES, "qualified Python")?;
    ensure!(
        bytes == PYTHON_BYTES && sha256::hex(&digest) == PYTHON_SHA256,
        "qualified Python executable differs"
    );
    let runtime = identify_python_runtime(&python)?;
    ensure!(
        python.parent() == Some(runtime.root.as_path()),
        "qualified Python runtime is not the executable parent"
    );
    Ok(runtime)
}

pub(super) fn git_identity(git: &crate::git::Git) -> Result<GitIdentity> {
    let fields = git.fields();
    ensure!(
        fields[0].0 == "tool.git.bytes"
            && fields[1].0 == "tool.git.path"
            && fields[2].0 == "tool.git.sha256"
            && fields[3].0 == "tool.git.version",
        "resolved Git field order differs"
    );
    let bytes = fields[0]
        .1
        .parse::<u64>()
        .context("resolved Git byte count is malformed")?;
    let path = PathBuf::from(&fields[1].1);
    ensure!(
        path == git.path() && path.is_absolute(),
        "resolved Git path differs"
    );
    require_sha256(&fields[2].1, "resolved Git digest")?;
    ensure!(
        fields[3].1.starts_with("git version ") && !fields[3].1.contains(['\r', '\n', '\0']),
        "resolved Git version differs"
    );
    Ok(GitIdentity {
        path,
        bytes,
        sha256: fields[2].1.clone(),
        version: fields[3].1.clone(),
    })
}

fn collect_python_runtime_entries(
    root: &Path,
    path: &Path,
    depth: usize,
    entries: &mut Vec<(String, PathBuf, bool)>,
) -> Result<()> {
    ensure!(
        depth <= MAX_PYTHON_RUNTIME_DEPTH,
        "Python runtime exceeds its depth limit"
    );
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspecting Python runtime entry {}", path.display()))?;
    ensure!(
        !metadata.file_type().is_symlink() && !metadata_is_reparse_point(&metadata),
        "Python runtime contains a redirected entry"
    );
    let relative = path
        .strip_prefix(root)
        .context("Python runtime entry escaped its root")?;
    let parts = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .context("Python runtime entry is not UTF-8")?;
    if parts.iter().any(|part| {
        part.eq_ignore_ascii_case("__pycache__") || part.eq_ignore_ascii_case("site-packages")
    }) {
        return Ok(());
    }
    let relative = parts.join("/");
    ensure!(!relative.is_empty(), "Python runtime entry has no name");
    let is_directory = crate::mutate::workspace::metadata_is_real_directory(&metadata);
    ensure!(
        is_directory || metadata.file_type().is_file(),
        "Python runtime contains a special entry"
    );
    entries.push((relative, path.to_path_buf(), is_directory));
    ensure!(
        entries.len() as u64 <= MAX_PYTHON_RUNTIME_ENTRIES,
        "Python runtime exceeds its entry limit"
    );
    if is_directory {
        let remaining = usize::try_from(MAX_PYTHON_RUNTIME_ENTRIES)
            .context("Python runtime entry limit does not fit usize")?
            .saturating_sub(entries.len());
        let children = bounded_sorted_directory_entries(path, remaining)?;
        for child in children {
            collect_python_runtime_entries(root, &child.path(), depth + 1, entries)?;
        }
    }
    Ok(())
}

pub(super) fn bounded_sorted_directory_entries(
    directory: &Path,
    remaining: usize,
) -> Result<Vec<std::fs::DirEntry>> {
    let mut children = Vec::new();
    for child in std::fs::read_dir(directory)
        .with_context(|| format!("listing Python runtime directory {}", directory.display()))?
    {
        ensure!(
            children.len() < remaining,
            "Python runtime exceeds its entry limit"
        );
        children.push(child.with_context(|| {
            format!("reading Python runtime directory {}", directory.display())
        })?);
    }
    children.sort_by_key(std::fs::DirEntry::file_name);
    Ok(children)
}

#[allow(clippy::too_many_arguments)]
fn identify_tool(
    path: &Path,
    arguments: &[&str],
    expected_bytes: u64,
    expected_sha256: &str,
    expected_version: &str,
    label: &str,
    root: &Path,
    scratch: &OwnedScratch,
) -> Result<ToolIdentity> {
    let canonical = require_exact_executable_path(path, label)?;
    let (bytes, digest) = measure_file(&canonical, MAX_EXECUTABLE_BYTES, label)?;
    let sha256 = sha256::hex(&digest);
    ensure!(
        bytes == expected_bytes && sha256 == expected_sha256,
        "{label} executable differs from the qualified runtime"
    );
    let mut command = Command::new(&canonical);
    command.args(arguments).current_dir(root);
    apply_base_environment(&mut command, scratch, None)?;
    if label == "Codex" {
        require_private_credential_projection(&scratch.codex_home())?;
    }
    let output = crate::bounded_process::output_with_limit(
        command,
        VERSION_TIMEOUT,
        &format!("{label} version probe"),
        MAX_VERSION_BYTES,
    )?;
    if label == "Codex" {
        require_private_credential_projection(&scratch.codex_home())?;
    }
    ensure!(
        output.status.success(),
        "{label} version probe failed ({})",
        output.status
    );
    let version = single_line_output(&output.stdout, &output.stderr, label)?;
    ensure!(
        version == expected_version,
        "{label} version differs from the qualified runtime"
    );
    Ok(ToolIdentity {
        path: canonical,
        bytes,
        sha256,
        version,
    })
}

pub(super) fn require_exact_executable_path(path: &Path, label: &str) -> Result<PathBuf> {
    let parent = path
        .parent()
        .with_context(|| format!("{label} executable has no parent"))?;
    ensure!(
        canonical_directory(parent, &format!("{label} executable parent"))? == parent,
        "{label} executable parent is redirected"
    );
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspecting {label} executable {}", path.display()))?;
    ensure!(
        metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && !metadata_is_reparse_point(&metadata),
        "{label} executable is redirected or special"
    );
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalizing {label} executable {}", path.display()))?;
    ensure!(canonical == path, "{label} executable path is redirected");
    Ok(canonical)
}

fn single_line_output(stdout: &[u8], stderr: &[u8], label: &str) -> Result<String> {
    let selected = if stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        stdout
    } else {
        stderr
    };
    let value = std::str::from_utf8(selected)
        .with_context(|| format!("{label} version is not UTF-8"))?
        .trim();
    ensure!(
        !value.is_empty() && !value.contains(['\r', '\n', '\0']),
        "{label} printed no single-line version identity"
    );
    Ok(value.to_string())
}

pub(super) fn measure_file(path: &Path, maximum: u64, label: &str) -> Result<(u64, [u8; 32])> {
    let mut file = crate::mutate::workspace::open_real_file(path, label)?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{label} exceeds its {maximum}-byte limit"
    );
    let (bytes, digest) = sha256::digest_reader((&mut file).take(maximum + 1))
        .with_context(|| format!("hashing {label} {}", path.display()))?;
    ensure!(
        bytes == initial
            && file
                .metadata()
                .with_context(|| format!("remeasuring {label} {}", path.display()))?
                .len()
                == initial,
        "{label} changed while it was measured"
    );
    Ok((bytes, digest))
}

pub(super) fn require_chatgpt_login(
    codex: &ToolIdentity,
    scratch: &OwnedScratch,
    root: &Path,
) -> Result<()> {
    let mut command = Command::new(&codex.path);
    command.args(["login", "status"]).current_dir(root);
    apply_base_environment(&mut command, scratch, None)?;
    require_private_credential_projection(&scratch.codex_home())?;
    let output = crate::bounded_process::output_with_limit(
        command,
        VERSION_TIMEOUT,
        "Codex ChatGPT login probe",
        MAX_VERSION_BYTES,
    )?;
    require_private_credential_projection(&scratch.codex_home())?;
    ensure!(
        output.status.success(),
        "Codex ChatGPT login probe failed ({})",
        output.status
    );
    require_chatgpt_login_output(&output.stdout, &output.stderr)
}

pub(super) fn require_chatgpt_login_output(stdout: &[u8], stderr: &[u8]) -> Result<()> {
    let stdout = std::str::from_utf8(stdout).context("Codex login stdout is not UTF-8")?;
    let stderr = std::str::from_utf8(stderr).context("Codex login stderr is not UTF-8")?;
    let statuses: Vec<_> = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .filter(|line| line.starts_with("Logged in using "))
        .collect();
    ensure!(
        statuses.as_slice() == ["Logged in using ChatGPT"],
        "owned Codex Security qualification requires unambiguous ChatGPT authentication"
    );
    Ok(())
}
