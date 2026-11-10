use super::*;

pub(crate) const PYTHON_RUNPY_BOOTSTRAP: &str = "import runpy,sys;p=sys.argv.pop(1);d=sys.argv.pop(1);sys.path.insert(0,d);sys.argv[0]=p;runpy.run_path(p,run_name='__main__')";

pub(crate) fn private_python_child_command(
    executable: &Path,
    pycache: &Path,
    script: &Path,
    scripts: &Path,
    script_arguments: &[OsString],
) -> Result<Command> {
    let pycache = pycache
        .to_str()
        .context("private Python bytecode directory is not UTF-8")?;
    let mut command = Command::new(executable);
    command
        .args(["-I", "-S", "-B", "-X", "utf8", "-X"])
        .arg(format!("pycache_prefix={pycache}"))
        .args(["-c", PYTHON_RUNPY_BOOTSTRAP])
        .arg(script)
        .arg(scripts)
        .args(script_arguments);
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("PYTHON")
        {
            command.env_remove(name);
        }
    }
    command
        .env_remove(WRAPPER_MARKER_ENV)
        .env_remove(WRAPPER_REAL_ENV)
        .env_remove(WRAPPER_OVERRIDES_ENV)
        .env_remove(WRAPPER_REPOSITORY_ENV)
        .env_remove(WRAPPER_NODE_ENV)
        .env_remove(WRAPPER_LIVE_AUTH_ENV)
        .env_remove(WRAPPER_PROVIDER_PERMISSION_ENV)
        .env_remove(WRAPPER_WORKER_PERMISSION_ENV)
        .env_remove(WRAPPER_TRANSITIONS_ENV)
        .env_remove("CODEX_CLI_PATH")
        .env_remove(PYTHON_WRAPPER_REAL_ENV)
        .env_remove(PYTHON_WRAPPER_PLUGIN_ENV)
        .env_remove(PYTHON_WRAPPER_PYCACHE_ENV)
        .env_remove(PYTHON_WRAPPER_RUNTIME_ENV)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONSAFEPATH", "1")
        .env("PYTHONUTF8", "1");
    Ok(command)
}

pub(crate) fn validate_python_wrapper_arguments<'arguments>(
    arguments: &'arguments [OsString],
    mirror: &Path,
    repository: &Path,
    scan_root: &Path,
    state_root: &Path,
) -> Result<(PathBuf, &'arguments [OsString])> {
    let script_index = if arguments.first().is_some_and(|argument| argument == "-I") {
        ensure!(
            arguments.get(1).is_some_and(|argument| argument == "-X")
                && arguments.get(2).is_some_and(|argument| argument == "utf8"),
            "private Python renderer flags differ"
        );
        3
    } else {
        0
    };
    let supplied = arguments
        .get(script_index)
        .context("private Python invocation has no script")?;
    require_nonempty_argument(supplied, "private Python script")?;
    let supplied = PathBuf::from(supplied);
    ensure!(
        supplied.is_absolute(),
        "private Python script path is not absolute"
    );
    let script = std::fs::canonicalize(&supplied).with_context(|| {
        format!(
            "canonicalizing private Python script {}",
            supplied.display()
        )
    })?;
    ensure!(script == supplied, "private Python script is redirected");
    require_integrity_listed_script(mirror, &script)?;
    let script_arguments = &arguments[script_index + 1..];
    for argument in script_arguments {
        require_nonempty_argument(argument, "private Python script argument")?;
    }
    let scripts = canonical_directory(
        &mirror.join("scripts"),
        "private Python plugin scripts directory",
    )?;
    if script == scripts.join(PYTHON_INVENTORY_SCRIPT) {
        ensure!(script_index == 0, "private Python inventory flags differ");
        validate_inventory_arguments(script_arguments, repository, scan_root)?;
    } else if script == scripts.join(PYTHON_RENDERER_SCRIPT) {
        ensure!(
            script_index == 3 && script_arguments == [OsString::from("--input-json-stdin")],
            "private Python renderer arguments differ"
        );
    } else if script == scripts.join(PYTHON_WORKBENCH_SCRIPT) {
        ensure!(script_index == 0, "private Python workbench flags differ");
        validate_workbench_arguments(script_arguments, repository, scan_root, state_root)?;
    } else {
        bail!("private Python script is not an owned provider entrypoint");
    }
    Ok((script, script_arguments))
}

pub(crate) fn render_python_runtime_policy(runtime: &PythonRuntimeIdentity) -> Result<String> {
    let root = runtime
        .root
        .to_str()
        .context("qualified Python runtime root is not UTF-8")?;
    render_json(&object([
        ("bytes", JsonValue::Number(runtime.bytes.to_string())),
        ("entries", JsonValue::Number(runtime.entries.to_string())),
        ("root", text_value(root)),
        ("sha256", text_value(&runtime.sha256)),
    ]))
}

pub(crate) fn parse_python_runtime_policy(serialized: &str) -> Result<PythonRuntimeIdentity> {
    ensure!(
        !serialized.is_empty() && serialized.len() <= MAX_MCP_LINE_BYTES,
        "private Python runtime-tree policy is malformed"
    );
    let value = parse_json(serialized.as_bytes(), "private Python runtime-tree policy")?;
    let policy = json_object(&value, "private Python runtime-tree policy")?;
    require_json_fields(
        policy,
        &["bytes", "entries", "root", "sha256"],
        "private Python runtime-tree policy",
    )?;
    let root = PathBuf::from(string(
        policy,
        "root",
        "private Python runtime-tree policy",
    )?);
    ensure!(
        root.is_absolute(),
        "private Python runtime root is relative"
    );
    let sha256 = string(policy, "sha256", "private Python runtime-tree policy")?;
    require_sha256(sha256, "private Python runtime-tree digest")?;
    Ok(PythonRuntimeIdentity {
        root,
        entries: json_u64(
            json_field(policy, "entries", "private Python runtime-tree policy")?,
            "private Python runtime-tree entry count",
        )?,
        bytes: json_u64(
            json_field(policy, "bytes", "private Python runtime-tree policy")?,
            "private Python runtime-tree byte count",
        )?,
        sha256: sha256.to_string(),
    })
}

fn require_integrity_listed_script(mirror: &Path, script: &Path) -> Result<()> {
    let scripts = canonical_directory(
        &mirror.join("scripts"),
        "private Python plugin scripts directory",
    )?;
    ensure!(
        script.starts_with(&scripts) && script.extension().and_then(OsStr::to_str) == Some("py"),
        "private Python script is outside the mirrored scripts directory"
    );
    let integrity = read(
        &mirror.join("integrity.json"),
        MAX_PLUGIN_MANIFEST_BYTES,
        "private Python integrity manifest",
    )?;
    ensure!(
        integrity.len() as u64 == PLUGIN_INTEGRITY_BYTES
            && sha256::hex(&sha256::digest(&integrity)) == PLUGIN_INTEGRITY_SHA256,
        "private Python integrity manifest differs"
    );
    let manifest = parse_json(&integrity, "private Python integrity manifest")?;
    let manifest = json_object(&manifest, "private Python integrity manifest")?;
    let relative = script
        .strip_prefix(mirror)
        .context("private Python script is outside its mirror")?;
    let slash_path = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .context("private Python script path is not UTF-8")?
        .join("/");
    let files = json_array(
        json_field(manifest, "files", "private Python integrity manifest")?,
        "private Python integrity files",
    )?;
    let mut expected = None;
    for entry in files {
        let entry = json_object(entry, "private Python integrity entry")?;
        if string(entry, "path", "private Python integrity entry")? == slash_path {
            ensure!(
                expected.is_none(),
                "private Python script is duplicated in integrity manifest"
            );
            expected = Some(string(entry, "sha256", "private Python integrity entry")?);
        }
    }
    let expected = expected.context("private Python script is not integrity-listed")?;
    let (_, digest) = measure_file(script, MAX_PLUGIN_FILE_BYTES, "private Python script")?;
    ensure!(
        sha256::hex(&digest) == expected,
        "private Python script differs from its integrity entry"
    );
    Ok(())
}

pub(crate) fn validate_inventory_arguments(
    arguments: &[OsString],
    repository: &Path,
    scan_root: &Path,
) -> Result<()> {
    ensure!(
        arguments.len() == 6
            && arguments[0] == "--repo"
            && arguments[2] == "--scope"
            && arguments[3] == "."
            && arguments[4] == "--out",
        "private Python inventory arguments differ"
    );
    let supplied_repository = Path::new(&arguments[1]);
    ensure!(
        supplied_repository == repository,
        "private Python inventory repository differs"
    );
    let output = Path::new(&arguments[5]);
    require_owned_path(output, scan_root, "private Python inventory output")?;
    let relative = output
        .strip_prefix(scan_root)
        .context("private Python inventory output escaped its scan root")?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .context("private Python inventory output is not UTF-8")?;
    ensure!(
        components.len() == 5
            && components[2] == "artifacts"
            && components[3] == "02_discovery"
            && components[4] == "in_scope_files.txt",
        "private Python inventory output shape differs"
    );
    Ok(())
}
