mod overrides;
mod python;
mod workbench;

use super::*;

pub(super) use self::overrides::*;
pub(super) use self::python::*;
pub(super) use self::workbench::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PrivateWrapperKind {
    Codex,
    Python,
}

pub(super) fn classify_private_wrapper(arguments: &[OsString]) -> Result<PrivateWrapperKind> {
    let first = arguments
        .first()
        .context("private wrapper invocation has no runtime arguments")?;
    let codex_shape = first == "exec" || first == "--config" || first == "app-server";
    let python_shape = first == "-I" || Path::new(first).is_absolute();
    ensure!(
        codex_shape ^ python_shape,
        "private wrapper invocation has no unique runtime shape"
    );
    Ok(if codex_shape {
        PrivateWrapperKind::Codex
    } else {
        PrivateWrapperKind::Python
    })
}

pub(crate) fn dispatch_private_wrapper() -> Option<Result<i32>> {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == PRIVATE_CREDENTIAL_PROBE_ARGUMENT)
    {
        return Some(dispatch_private_credential_probe(&arguments));
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == PRIVATE_WRITE_PROBE_ARGUMENT)
    {
        return Some(dispatch_private_write_probe(&arguments));
    }
    if arguments.first().is_some_and(|argument| {
        argument == PRIVATE_WRITE_APPEND_PROBE_ARGUMENT
            || argument == PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT
            || argument == PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT
    }) {
        return Some(dispatch_private_existing_write_probe(&arguments));
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == PRIVATE_NETWORK_PROBE_ARGUMENT)
    {
        return Some(dispatch_private_network_probe(&arguments));
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT)
    {
        return Some(dispatch_private_network_connect_probe(&arguments));
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == PRIVATE_DESCENDANT_PROBE_ARGUMENT)
    {
        return Some(dispatch_private_descendant_probe(&arguments));
    }
    let marker = std::env::var_os(WRAPPER_MARKER_ENV)?;
    Some(match classify_private_wrapper(&arguments) {
        Ok(PrivateWrapperKind::Codex) => dispatch_private_wrapper_inner(&marker, &arguments),
        Ok(PrivateWrapperKind::Python) => dispatch_private_python_wrapper(&marker, &arguments),
        Err(error) => Err(error),
    })
}

pub(super) fn dispatch_private_credential_probe(arguments: &[OsString]) -> Result<i32> {
    ensure!(
        arguments.len() == 2 && arguments[0] == PRIVATE_CREDENTIAL_PROBE_ARGUMENT,
        "private credential probe invocation differs"
    );
    let path = Path::new(&arguments[1]);
    ensure!(
        path.is_absolute(),
        "private credential probe path is not absolute"
    );
    Ok(private_credential_probe_exit(
        open_private_credential_for_probe(path),
    ))
}

pub(super) fn private_credential_probe_exit(result: std::io::Result<()>) -> i32 {
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            PRIVATE_CREDENTIAL_DENIED_EXIT
        }
        Ok(()) => PRIVATE_CREDENTIAL_READABLE_EXIT,
        Err(_) => PRIVATE_CREDENTIAL_ERROR_EXIT,
    }
}

pub(super) fn dispatch_private_write_probe(arguments: &[OsString]) -> Result<i32> {
    ensure!(
        arguments.len() == 2 && arguments[0] == PRIVATE_WRITE_PROBE_ARGUMENT,
        "private write probe invocation differs"
    );
    let path = Path::new(&arguments[1]);
    ensure!(
        path.is_absolute(),
        "private write probe path is not absolute"
    );
    Ok(private_credential_probe_exit(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map(drop),
    ))
}

pub(super) fn dispatch_private_existing_write_probe(arguments: &[OsString]) -> Result<i32> {
    ensure!(
        arguments.len() == 2
            && matches!(
                arguments[0].to_str(),
                Some(PRIVATE_WRITE_APPEND_PROBE_ARGUMENT)
                    | Some(PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT)
                    | Some(PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT)
            ),
        "private existing-write probe invocation differs"
    );
    let path = Path::new(&arguments[1]);
    ensure!(
        path.is_absolute(),
        "private existing-write probe path is not absolute"
    );
    require_worker_probe_file(path, "private existing-write probe")?;
    let result = match arguments[0].to_str() {
        Some(PRIVATE_WRITE_APPEND_PROBE_ARGUMENT) => OpenOptions::new()
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(b"wizard append probe")),
        Some(PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT) => OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .map(drop),
        Some(PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT) => OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|mut file| file.write_all(b"wizard overwrite probe")),
        _ => bail!("private existing-write probe tag changed after validation"),
    };
    Ok(private_credential_probe_exit(result))
}

pub(super) fn dispatch_private_network_probe(arguments: &[OsString]) -> Result<i32> {
    ensure!(
        arguments.len() == 1 && arguments[0] == PRIVATE_NETWORK_PROBE_ARGUMENT,
        "private network probe invocation differs"
    );
    Ok(private_credential_probe_exit(
        std::net::TcpListener::bind("127.0.0.1:0").map(drop),
    ))
}

pub(super) fn parse_private_loopback_endpoint(value: &OsStr) -> Result<std::net::SocketAddr> {
    let value = value
        .to_str()
        .context("private network-connect endpoint is not UTF-8")?;
    let endpoint = value
        .parse::<std::net::SocketAddr>()
        .context("private network-connect endpoint is malformed")?;
    ensure!(
        endpoint.is_ipv4()
            && endpoint.ip().is_loopback()
            && endpoint.port() != 0
            && endpoint.to_string() == value,
        "private network-connect endpoint is not a canonical IPv4 loopback socket"
    );
    Ok(endpoint)
}

pub(super) fn dispatch_private_network_connect_probe(arguments: &[OsString]) -> Result<i32> {
    ensure!(
        arguments.len() == 2 && arguments[0] == PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT,
        "private network-connect probe invocation differs"
    );
    let endpoint = parse_private_loopback_endpoint(&arguments[1])?;
    Ok(private_credential_probe_exit(
        std::net::TcpStream::connect_timeout(&endpoint, Duration::from_secs(5)).map(drop),
    ))
}

pub(super) fn dispatch_private_descendant_probe(arguments: &[OsString]) -> Result<i32> {
    ensure!(
        arguments.len() == 2 && arguments[0] == PRIVATE_DESCENDANT_PROBE_ARGUMENT,
        "private descendant probe invocation differs"
    );
    let target = Path::new(&arguments[1]);
    ensure!(
        target.is_absolute(),
        "private descendant probe path is not absolute"
    );
    let executable = std::env::current_exe().context("resolving private descendant probe")?;
    let status = Command::new(executable)
        .arg(PRIVATE_CREDENTIAL_PROBE_ARGUMENT)
        .arg(target)
        .env_clear()
        .status()
        .context("running private descendant read probe")?;
    Ok(match status.code() {
        Some(PRIVATE_CREDENTIAL_DENIED_EXIT) => PRIVATE_CREDENTIAL_DENIED_EXIT,
        Some(_) => PRIVATE_CREDENTIAL_READABLE_EXIT,
        None => PRIVATE_CREDENTIAL_ERROR_EXIT,
    })
}

#[cfg(windows)]
fn open_private_credential_for_probe(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    OpenOptions::new()
        .access_mode(FILE_READ_DATA_ACCESS)
        .open(path)
        .map(drop)
}

#[cfg(not(windows))]
fn open_private_credential_for_probe(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "private credential probe requires Windows",
    ))
}

fn dispatch_private_wrapper_inner(marker: &OsStr, arguments: &[OsString]) -> Result<i32> {
    let marker = marker
        .to_str()
        .context("private Codex wrapper marker is not UTF-8")?;
    let source = marker
        .strip_prefix(WRAPPER_MARKER_PREFIX)
        .context("private Codex wrapper marker is malformed")?;
    ensure!(
        is_source_commit(source),
        "private Codex wrapper marker has an invalid source identity"
    );
    let real = PathBuf::from(
        std::env::var_os(WRAPPER_REAL_ENV)
            .context("private Codex wrapper has no real executable")?,
    );
    let canonical = std::fs::canonicalize(&real)
        .with_context(|| format!("canonicalizing private Codex executable {}", real.display()))?;
    ensure!(canonical == real, "private Codex executable is redirected");
    let current = std::env::current_exe().context("resolving the private Codex wrapper")?;
    ensure!(
        canonical != current,
        "private Codex wrapper would recursively invoke itself"
    );
    let runtime = identify_codex_runtime(&canonical)?;
    let serialized = std::env::var(WRAPPER_OVERRIDES_ENV)
        .context("private Codex wrapper has no serialized safety policy")?;
    let overrides = parse_wrapper_overrides(&serialized)?;
    let invocation = inject_wrapper_overrides(arguments, &overrides)?;
    let private_home = PathBuf::from(
        std::env::var_os("CODEX_HOME").context("private Codex wrapper has no private home")?,
    );
    require_private_credential_projection(&private_home)?;
    let status = if invocation.transition == WorkerTransitionKind::AppServerPassThrough {
        let worker = current_private_worker_directory()?;
        let mut command = private_codex_child_command(&canonical, &invocation.arguments);
        command.stdout(Stdio::piped());
        let mut child = command
            .spawn()
            .context("starting the private Codex app-server child")?;
        if let Err(error) = write_worker_transition_receipt(
            source,
            invocation.transition,
            WorkerTransitionStage::Launched,
            &worker.path,
            None,
            &invocation.arguments,
        ) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        if let Err(error) = relay_private_child_stdout(&mut child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        child
            .wait()
            .context("running the private Codex app-server child")?
    } else {
        let worker = invocation
            .worker
            .as_ref()
            .context("private Codex exec has no validated worker directory")?;
        let mut command = private_codex_child_command(&canonical, &invocation.arguments);
        command.stdout(Stdio::piped());
        let mut child = command
            .spawn()
            .context("starting the private Codex exec child")?;
        let thread = match relay_private_exec_stdout(source, &invocation, &worker.path, &mut child)
        {
            Ok(thread) => thread,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let status = child
            .wait()
            .context("running the private Codex exec child")?;
        if status.code() == Some(0) {
            write_worker_transition_receipt(
                source,
                invocation.transition,
                WorkerTransitionStage::ExitedZero,
                &worker.path,
                Some(&thread),
                &invocation.arguments,
            )?;
        }
        status
    };
    require_private_credential_projection(&private_home)?;
    ensure!(
        identify_codex_runtime(&canonical)? == runtime,
        "private Codex runtime changed during child execution"
    );
    let code = status
        .code()
        .context("private Codex child exited without an exit code")?;
    Ok(code)
}

pub(super) fn is_source_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn relay_private_child_stdout(child: &mut std::process::Child) -> Result<()> {
    let mut child_stdout = child
        .stdout
        .take()
        .context("private Codex child has no captured stdout")?;
    let mut parent_stdout = std::io::stdout().lock();
    std::io::copy(&mut child_stdout, &mut parent_stdout)
        .context("relaying private Codex child stdout")?;
    parent_stdout
        .flush()
        .context("flushing private Codex child stdout")
}

fn current_private_worker_directory() -> Result<WorkerDirectory> {
    let pycache = PathBuf::from(
        std::env::var_os(PYTHON_WRAPPER_PYCACHE_ENV)
            .context("private Codex wrapper has no Python bytecode directory")?,
    );
    let scratch = canonical_directory(
        pycache
            .parent()
            .context("private Python bytecode directory has no scratch parent")?,
        "private Codex transition scratch",
    )?;
    require_scratch_cleanup_target(&scratch)?;
    let current = std::env::current_dir().context("resolving private Codex worker directory")?;
    require_worker_directory(&current, &scratch.join("scans"))
}

fn relay_private_exec_stdout(
    source: &str,
    invocation: &InjectedCodexInvocation,
    worker: &Path,
    child: &mut std::process::Child,
) -> Result<String> {
    let mut child_stdout = child
        .stdout
        .take()
        .context("private Codex exec child has no captured stdout")?;
    let mut parent_stdout = std::io::stdout().lock();
    let mut chunk = [0u8; 8192];
    let mut line = Vec::new();
    let mut observed_thread = None;
    loop {
        let count = child_stdout
            .read(&mut chunk)
            .context("reading private Codex exec JSONL")?;
        if count == 0 {
            break;
        }
        for byte in &chunk[..count] {
            line.push(*byte);
            ensure!(
                line.len() <= MAX_MCP_LINE_BYTES,
                "private Codex exec JSONL line exceeds its size limit"
            );
            if *byte != b'\n' {
                continue;
            }
            relay_private_exec_line(
                source,
                invocation,
                worker,
                &line,
                &mut observed_thread,
                &mut parent_stdout,
            )?;
            line.clear();
        }
    }
    ensure!(
        line.is_empty(),
        "private Codex exec ended with an incomplete JSONL record"
    );
    parent_stdout
        .flush()
        .context("flushing private Codex exec JSONL")?;
    observed_thread.context("private Codex exec emitted no thread.started event")
}

pub(super) fn relay_private_exec_line(
    source: &str,
    invocation: &InjectedCodexInvocation,
    worker: &Path,
    line: &[u8],
    observed_thread: &mut Option<String>,
    output: &mut impl Write,
) -> Result<()> {
    let started =
        observe_private_exec_line(line, invocation.expected_thread.as_deref(), observed_thread)?;
    if let Some(thread) = started {
        write_worker_transition_receipt(
            source,
            invocation.transition,
            WorkerTransitionStage::Started,
            worker,
            Some(&thread),
            &invocation.arguments,
        )?;
    }
    output
        .write_all(line)
        .context("relaying private Codex exec JSONL")?;
    output
        .flush()
        .context("flushing private Codex exec JSONL event")
}

pub(super) fn observe_private_exec_line(
    line: &[u8],
    expected_thread: Option<&str>,
    observed_thread: &mut Option<String>,
) -> Result<Option<String>> {
    let json = line.strip_suffix(b"\n").unwrap_or(line);
    ensure!(
        !json.is_empty() && !json.contains(&b'\r') && !json.contains(&b'\0'),
        "private Codex exec JSONL framing differs"
    );
    let value = parse_json(json, "private Codex exec event")?;
    let event = json_object(&value, "private Codex exec event")?;
    let event_type = string(event, "type", "private Codex exec event")?;
    if event_type == "thread.started" {
        let thread = nonempty_string(event, "thread_id", "private Codex thread.started event")?;
        require_safe_identity(thread, "private Codex thread.started identity")?;
        ensure!(
            observed_thread.is_none(),
            "private Codex exec emitted duplicate thread.started events"
        );
        if let Some(expected) = expected_thread {
            ensure!(
                thread == expected,
                "private Codex resumed thread differs from its thread.started event"
            );
        }
        *observed_thread = Some(thread.to_string());
        return Ok(Some(thread.to_string()));
    }
    Ok(None)
}

pub(super) fn private_codex_child_command(executable: &Path, arguments: &[OsString]) -> Command {
    let mut command = Command::new(executable);
    command
        .args(arguments)
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
        .env_remove("PYTHON");
    command
}

fn dispatch_private_python_wrapper(marker: &OsStr, arguments: &[OsString]) -> Result<i32> {
    let marker = marker
        .to_str()
        .context("private Python wrapper marker is not UTF-8")?;
    let source = marker
        .strip_prefix(WRAPPER_MARKER_PREFIX)
        .context("private Python wrapper marker is malformed")?;
    ensure!(
        is_source_commit(source),
        "private Python wrapper marker has an invalid source identity"
    );
    let real = PathBuf::from(
        std::env::var_os(PYTHON_WRAPPER_REAL_ENV)
            .context("private Python wrapper has no real executable")?,
    );
    let real = std::fs::canonicalize(&real).with_context(|| {
        format!(
            "canonicalizing private Python executable {}",
            real.display()
        )
    })?;
    ensure!(
        real != std::env::current_exe().context("resolving the private Python wrapper")?,
        "private Python wrapper would recursively invoke itself"
    );
    let (bytes, digest) = measure_file(&real, MAX_EXECUTABLE_BYTES, "private Python executable")?;
    ensure!(
        bytes == PYTHON_BYTES && sha256::hex(&digest) == PYTHON_SHA256,
        "private Python executable differs from the qualified runtime"
    );
    let expected_runtime = parse_python_runtime_policy(
        &std::env::var(PYTHON_WRAPPER_RUNTIME_ENV)
            .context("private Python wrapper has no runtime-tree policy")?,
    )?;
    ensure!(
        real.parent() == Some(expected_runtime.root.as_path())
            && identify_python_runtime(&real)? == expected_runtime,
        "private Python runtime tree differs from the qualified runtime"
    );
    let mirror = PathBuf::from(
        std::env::var_os(PYTHON_WRAPPER_PLUGIN_ENV)
            .context("private Python wrapper has no plugin mirror")?,
    );
    let mirror = canonical_directory(&mirror, "private Python plugin mirror")?;
    let mirror_package = load_plugin_package_at(mirror.clone())?;
    require_mirror_exact(&mirror, &mirror_package)?;
    let pycache = PathBuf::from(
        std::env::var_os(PYTHON_WRAPPER_PYCACHE_ENV)
            .context("private Python wrapper has no bytecode directory")?,
    );
    let pycache = canonical_directory(&pycache, "private Python bytecode directory")?;
    let scratch = pycache
        .parent()
        .context("private Python bytecode directory has no scratch parent")?;
    require_scratch_cleanup_target(scratch)?;
    ensure!(
        pycache == scratch.join("pycache"),
        "private Python bytecode directory escaped owned scratch"
    );
    ensure!(
        mirror == scratch.join("plugin"),
        "private Python plugin mirror escaped owned scratch"
    );
    let repository = PathBuf::from(
        std::env::var_os(WRAPPER_REPOSITORY_ENV)
            .context("private Python wrapper has no target repository")?,
    );
    let repository = canonical_directory(&repository, "private Python target repository")?;
    let scan_root = PathBuf::from(
        std::env::var_os("CODEX_SECURITY_SCAN_ROOT")
            .context("private Python wrapper has no owned scan root")?,
    );
    let scan_root = canonical_directory(&scan_root, "private Python scan root")?;
    ensure!(
        scan_root == scratch.join("scans"),
        "private Python scan root escaped owned scratch"
    );
    let state_root = PathBuf::from(
        std::env::var_os("CODEX_SECURITY_STATE_DIR")
            .context("private Python wrapper has no owned state root")?,
    );
    let state_root = canonical_directory(&state_root, "private Python state root")?;
    ensure!(
        state_root == scratch.join("state"),
        "private Python state root escaped owned scratch"
    );
    let (script, script_arguments) = validate_python_wrapper_arguments(
        arguments,
        &mirror,
        &repository,
        &scan_root,
        &state_root,
    )?;
    let scripts = canonical_directory(
        &mirror.join("scripts"),
        "private Python plugin scripts directory",
    )?;
    let mut command =
        private_python_child_command(&real, &pycache, &script, &scripts, script_arguments)?;
    let status = command
        .status()
        .context("running the private Python child")?;
    ensure!(
        identify_python_runtime(&real)? == expected_runtime,
        "private Python runtime tree changed during helper execution"
    );
    require_mirror_exact(&mirror, &mirror_package)?;
    status
        .code()
        .context("private Python child exited without an exit code")
}
