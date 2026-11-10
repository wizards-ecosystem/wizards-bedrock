use super::*;

pub(super) fn require_worker_probe_positive_controls(scratch: &OwnedScratch) -> Result<()> {
    let root = scratch.scan_root.join("worker-probe-positive-controls");
    require_absent(&root, "managed worker positive-control directory")?;
    std::fs::create_dir(&root).context("creating managed worker positive-control directory")?;

    let create_target = root.join("create");
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&create_target)
        .context("opening managed worker create positive control")?
        .write_all(b"create")
        .context("writing managed worker create positive control")?;
    ensure!(
        std::fs::read(&create_target).context("reading managed worker create positive control")?
            == b"create",
        "managed worker create positive control did not change its target"
    );

    let append_target = root.join("append");
    std::fs::write(&append_target, b"before")
        .context("creating managed worker append positive control")?;
    OpenOptions::new()
        .append(true)
        .open(&append_target)
        .context("opening managed worker append positive control")?
        .write_all(b"-after")
        .context("writing managed worker append positive control")?;
    ensure!(
        std::fs::read(&append_target).context("reading managed worker append positive control")?
            == b"before-after",
        "managed worker append positive control did not change its target"
    );

    let truncate_target = root.join("truncate");
    std::fs::write(&truncate_target, b"before")
        .context("creating managed worker truncate positive control")?;
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&truncate_target)
        .context("opening managed worker truncate positive control")?;
    ensure!(
        std::fs::read(&truncate_target)
            .context("reading managed worker truncate positive control")?
            .is_empty(),
        "managed worker truncate positive control did not change its target"
    );

    let overwrite_target = root.join("overwrite");
    std::fs::write(&overwrite_target, b"before")
        .context("creating managed worker overwrite positive control")?;
    OpenOptions::new()
        .write(true)
        .open(&overwrite_target)
        .context("opening managed worker overwrite positive control")?
        .write_all(b"X")
        .context("writing managed worker overwrite positive control")?;
    ensure!(
        std::fs::read(&overwrite_target)
            .context("reading managed worker overwrite positive control")?
            == b"Xefore",
        "managed worker overwrite positive control did not change its target"
    );

    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("binding managed worker network positive control")?;
    let endpoint = listener
        .local_addr()
        .context("reading managed worker network positive-control endpoint")?;
    ensure!(
        endpoint.is_ipv4() && endpoint.ip().is_loopback() && endpoint.port() != 0,
        "managed worker network positive-control endpoint is not IPv4 loopback"
    );
    listener
        .set_nonblocking(true)
        .context("bounding managed worker network positive-control listener")?;
    let client = std::net::TcpStream::connect_timeout(&endpoint, Duration::from_secs(5))
        .context("connecting managed worker network positive control")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (server, peer) = loop {
        match listener.accept() {
            Ok(accepted) => break accepted,
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                return Err(error).context("accepting managed worker network positive control");
            }
        }
    };
    ensure!(
        peer.ip().is_loopback(),
        "managed worker network positive control accepted a non-loopback peer"
    );
    drop((client, server, listener));

    let metadata = std::fs::symlink_metadata(&root)
        .context("inspecting managed worker positive-control directory")?;
    remove_entry_no_follow(&root, &metadata)
        .context("removing managed worker positive-control directory")?;
    require_absent(&root, "managed worker positive-control directory")
}

fn require_existing_write_probe_unchanged(path: &Path, expected: &[u8]) -> Result<()> {
    require_worker_probe_file(path, "managed worker existing-write probe")?;
    ensure!(
        read(path, 128, "managed worker existing-write probe")? == expected,
        "managed worker existing-write probe changed despite a denied mutation"
    );
    Ok(())
}

pub(super) fn require_worker_filesystem_policy(
    tools: &Toolchain,
    scratch: &OwnedScratch,
    policy: &ManagedWorkerPolicy,
    shell_environment: &ShellEnvironmentPolicy,
) -> Result<WorkerProbeSummary> {
    let fixture = worker_probe_fixture(tools, scratch, policy)?;
    require_worker_canaries(scratch)?;
    require_worker_probe_positive_controls(scratch)?;
    let shell_environment_override = shell_environment.set_override()?;
    let runtime_before = identify_codex_runtime(&tools.codex.path)?;
    ensure!(
        runtime_before == tools.codex_runtime,
        "Codex sandbox runtime changed before credential preflight"
    );
    let mut summary = WorkerProbeSummary {
        allowed_reads: 0,
        denied_reads: 0,
        denied_write_creates: 0,
        denied_write_mutations: 0,
        denied_network_listeners: 0,
        denied_network_connections: 0,
        denied_descendants: 0,
    };
    for (probe_target, decision) in worker_read_probe_plan(policy) {
        let arguments = worker_read_probe_arguments(
            policy,
            &fixture,
            probe_target,
            &shell_environment_override,
        )?;
        require_worker_probe_execution(
            tools,
            scratch,
            policy,
            shell_environment,
            &arguments,
            WorkerProbeExpectation {
                label: decision.label(),
                exit: decision.exit(),
                disposition: decision.disposition(),
            },
        )?;
        match decision {
            WorkerReadDecision::Deny => summary.denied_reads += 1,
            WorkerReadDecision::Allow => summary.allowed_reads += 1,
        }
    }
    for probe_target in &policy.denied_write_probes {
        require_absent(probe_target, "managed worker denied write probe")?;
        let arguments = worker_private_probe_arguments(
            policy,
            &fixture,
            PRIVATE_WRITE_PROBE_ARGUMENT,
            Some(probe_target.as_os_str()),
            &shell_environment_override,
        )?;
        require_worker_probe_execution(
            tools,
            scratch,
            policy,
            shell_environment,
            &arguments,
            WorkerProbeExpectation {
                label: "Codex worker denied-write preflight",
                exit: PRIVATE_CREDENTIAL_DENIED_EXIT,
                disposition: "deny write",
            },
        )?;
        require_absent(probe_target, "managed worker denied write probe")?;
        summary.denied_write_creates += 1;
    }
    for probe_argument in [
        PRIVATE_WRITE_APPEND_PROBE_ARGUMENT,
        PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT,
        PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT,
    ] {
        require_existing_write_probe_unchanged(
            &policy.existing_write_probe,
            WORKER_ALLOWED_CANARY,
        )?;
        let arguments = worker_private_probe_arguments(
            policy,
            &fixture,
            probe_argument,
            Some(policy.existing_write_probe.as_os_str()),
            &shell_environment_override,
        )?;
        require_worker_probe_execution(
            tools,
            scratch,
            policy,
            shell_environment,
            &arguments,
            WorkerProbeExpectation {
                label: "Codex worker denied-existing-write preflight",
                exit: PRIVATE_CREDENTIAL_DENIED_EXIT,
                disposition: "deny existing-file mutation",
            },
        )?;
        require_existing_write_probe_unchanged(
            &policy.existing_write_probe,
            WORKER_ALLOWED_CANARY,
        )?;
        summary.denied_write_mutations += 1;
    }
    let network_arguments = worker_private_probe_arguments(
        policy,
        &fixture,
        PRIVATE_NETWORK_PROBE_ARGUMENT,
        None,
        &shell_environment_override,
    )?;
    require_worker_probe_execution(
        tools,
        scratch,
        policy,
        shell_environment,
        &network_arguments,
        WorkerProbeExpectation {
            label: "Codex worker denied-network preflight",
            exit: PRIVATE_CREDENTIAL_DENIED_EXIT,
            disposition: "deny network",
        },
    )?;
    summary.denied_network_listeners += 1;
    require_worker_denied_connect(
        tools,
        scratch,
        policy,
        shell_environment,
        &fixture,
        &shell_environment_override,
    )?;
    summary.denied_network_connections += 1;
    let descendant_arguments = worker_private_probe_arguments(
        policy,
        &fixture,
        PRIVATE_DESCENDANT_PROBE_ARGUMENT,
        Some(policy.denied_canary.as_os_str()),
        &shell_environment_override,
    )?;
    require_worker_probe_execution(
        tools,
        scratch,
        policy,
        shell_environment,
        &descendant_arguments,
        WorkerProbeExpectation {
            label: "Codex worker denied-descendant preflight",
            exit: PRIVATE_CREDENTIAL_DENIED_EXIT,
            disposition: "deny descendant read",
        },
    )?;
    summary.denied_descendants += 1;
    require_worker_canaries(scratch)?;
    ensure!(
        identify_codex_runtime(&tools.codex.path)? == runtime_before,
        "Codex sandbox runtime changed during credential preflight"
    );
    ensure!(
        summary.allowed_reads == policy.readable_probes.len()
            && summary.denied_reads == policy.deny_probes().len() + policy.denied_probes.len()
            && summary.denied_write_creates == policy.denied_write_probes.len()
            && summary.denied_write_mutations == 3
            && summary.denied_network_listeners == 1
            && summary.denied_network_connections == 1
            && summary.denied_descendants == 1,
        "managed worker preflight counts differ from the executed policy plan"
    );
    summary.require_complete()?;
    Ok(summary)
}

fn worker_probe_fixture(
    tools: &Toolchain,
    scratch: &OwnedScratch,
    policy: &ManagedWorkerPolicy,
) -> Result<PathBuf> {
    ensure!(
        cfg!(windows),
        "managed worker filesystem preflight requires Windows"
    );
    ensure!(
        canonical_directory(&policy.repository, "worker probe repository")? == policy.repository
            && canonical_directory(&policy.probe_directory, "worker probe directory")?
                == policy.probe_directory
            && policy.probe_directory == scratch.scan_root
            && policy.probe_directory != policy.repository,
        "worker probe repository or execution directory is redirected"
    );
    let supplied_fixture = std::env::current_exe().context("resolving credential probe fixture")?;
    let fixture = std::fs::canonicalize(&supplied_fixture).with_context(|| {
        format!(
            "canonicalizing credential probe fixture {}",
            supplied_fixture.display()
        )
    })?;
    ensure!(
        require_exact_executable_path(&fixture, "credential probe fixture")? == fixture,
        "credential probe fixture is redirected"
    );
    ensure!(
        fixture != tools.codex.path,
        "worker probe fixture resolves to the Codex runtime"
    );
    ensure!(
        policy.projected_auth == scratch.codex_home().join("auth.json")
            && policy.live_auth != policy.projected_auth,
        "worker probe targets differ from the owned credential policy"
    );
    Ok(fixture)
}

fn require_worker_denied_connect(
    tools: &Toolchain,
    scratch: &OwnedScratch,
    policy: &ManagedWorkerPolicy,
    shell_environment: &ShellEnvironmentPolicy,
    fixture: &Path,
    shell_environment_override: &str,
) -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("binding managed worker denied-connect control listener")?;
    let endpoint = listener
        .local_addr()
        .context("reading managed worker denied-connect control endpoint")?;
    ensure!(
        endpoint.is_ipv4() && endpoint.ip().is_loopback() && endpoint.port() != 0,
        "managed worker denied-connect endpoint is not IPv4 loopback"
    );
    let endpoint = endpoint.to_string();
    let connect_arguments = worker_private_probe_arguments(
        policy,
        fixture,
        PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT,
        Some(OsStr::new(&endpoint)),
        shell_environment_override,
    )?;
    require_worker_probe_execution(
        tools,
        scratch,
        policy,
        shell_environment,
        &connect_arguments,
        WorkerProbeExpectation {
            label: "Codex worker denied-network-connect preflight",
            exit: PRIVATE_CREDENTIAL_DENIED_EXIT,
            disposition: "deny network connect",
        },
    )?;
    drop(listener);
    Ok(())
}

pub(super) fn worker_read_probe_arguments(
    policy: &ManagedWorkerPolicy,
    fixture: &Path,
    probe_target: &Path,
    shell_environment: &str,
) -> Result<Vec<OsString>> {
    worker_private_probe_arguments(
        policy,
        fixture,
        PRIVATE_CREDENTIAL_PROBE_ARGUMENT,
        Some(probe_target.as_os_str()),
        shell_environment,
    )
}

pub(super) fn worker_private_probe_arguments(
    policy: &ManagedWorkerPolicy,
    fixture: &Path,
    probe_argument: &str,
    probe_target: Option<&OsStr>,
    shell_environment: &str,
) -> Result<Vec<OsString>> {
    let denied_read = |target: &Path| {
        policy.deny_probes().contains(&target)
            || policy.denied_probes.iter().any(|path| path == target)
    };
    let valid_probe = match (probe_argument, probe_target) {
        (PRIVATE_CREDENTIAL_PROBE_ARGUMENT, Some(value)) => {
            let target = Path::new(value);
            denied_read(target) || policy.readable_probes.iter().any(|path| path == target)
        }
        (PRIVATE_WRITE_PROBE_ARGUMENT, Some(value)) => {
            let target = Path::new(value);
            policy.denied_write_probes.iter().any(|path| path == target)
        }
        (
            PRIVATE_WRITE_APPEND_PROBE_ARGUMENT
            | PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT
            | PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT,
            Some(value),
        ) => Path::new(value) == policy.existing_write_probe,
        (PRIVATE_NETWORK_PROBE_ARGUMENT, None) => true,
        (PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT, Some(value)) => {
            parse_private_loopback_endpoint(value).is_ok()
        }
        (PRIVATE_DESCENDANT_PROBE_ARGUMENT, Some(value)) => denied_read(Path::new(value)),
        _ => false,
    };
    ensure!(
        policy.repository.is_absolute() && fixture.is_absolute() && valid_probe,
        "worker enforcement probe is not owned by the policy"
    );
    let worker_permission = effective_worker_named_permission_profile(policy)?;
    let mut arguments = vec![
        OsString::from("--no-daemon"),
        OsString::from("--strict-config"),
        OsString::from("--cd"),
        policy.probe_directory.as_os_str().to_owned(),
        OsString::from("sandbox"),
        OsString::from("--config"),
        OsString::from(format!(
            "default_permissions={}",
            quote_toml_string(WORKER_NAMED_PERMISSION_NAME)
        )),
        OsString::from("--config"),
        OsString::from(worker_permission),
        OsString::from("--config"),
        OsString::from("shell_environment_policy.inherit=\"none\""),
        OsString::from("--config"),
        OsString::from("shell_environment_policy.ignore_default_excludes=false"),
        OsString::from("--config"),
        OsString::from(shell_environment),
        OsString::from("--config"),
        OsString::from(WINDOWS_SANDBOX_OVERRIDE),
        OsString::from("--permission-profile"),
        OsString::from(WORKER_NAMED_PERMISSION_NAME),
        OsString::from("--include-managed-config"),
        OsString::from("--"),
        fixture.as_os_str().to_owned(),
        OsString::from(probe_argument),
    ];
    if let Some(target) = probe_target {
        arguments.push(target.to_owned());
    }
    Ok(arguments)
}

fn require_worker_probe_execution(
    tools: &Toolchain,
    scratch: &OwnedScratch,
    policy: &ManagedWorkerPolicy,
    shell_environment: &ShellEnvironmentPolicy,
    arguments: &[OsString],
    expected: WorkerProbeExpectation,
) -> Result<()> {
    let mut command = Command::new(&tools.codex.path);
    command.args(arguments).current_dir(&policy.probe_directory);
    apply_base_environment(&mut command, scratch, None)?;
    shell_environment.apply(&mut command);
    require_private_credential_projection(&scratch.codex_home())?;
    let output = crate::bounded_process::output_with_limit(
        command,
        VERSION_TIMEOUT,
        expected.label,
        MAX_VERSION_BYTES,
    )?;
    require_private_credential_projection(&scratch.codex_home())?;
    require_worker_read_probe_result(
        output.status.code(),
        &output.stdout,
        &output.stderr,
        expected.exit,
        expected.disposition,
    )
}

pub(super) fn require_worker_read_probe_result(
    code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
    expected: i32,
    disposition: &str,
) -> Result<()> {
    ensure!(
        code == Some(expected) && stdout.is_empty() && stderr.is_empty(),
        "managed MXC worker sandbox did not silently enforce the {disposition} read decision (exit={code:?}, stdout-bytes={}, stderr-bytes={})",
        stdout.len(),
        stderr.len()
    );
    Ok(())
}

pub(super) fn apply_base_environment(
    command: &mut Command,
    scratch: &OwnedScratch,
    tools: Option<(&Toolchain, &Path)>,
) -> Result<()> {
    let user_profile = std::env::var_os("USERPROFILE").context("USERPROFILE is not set")?;
    let codex_home = canonical_directory(&scratch.codex_home(), "private Codex home")?;
    ensure!(
        codex_home == scratch.codex_home(),
        "private Codex home is redirected"
    );
    require_no_ambient_codex_inputs(&codex_home)?;
    command.env_clear();
    for name in [
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "PROCESSOR_ARCHITECTURE",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("USERPROFILE", &user_profile)
        .env("HOME", &user_profile)
        .env("CODEX_HOME", &codex_home)
        .env("CODEX_SQLITE_HOME", &scratch.codex_state)
        .env("TEMP", &scratch.temporary)
        .env("TMP", &scratch.temporary)
        .env("TMPDIR", &scratch.temporary)
        .env("PYTHONPYCACHEPREFIX", &scratch.pycache)
        .env("XDG_CACHE_HOME", scratch.temporary.join("cache"));
    if let Some((tools, git)) = tools {
        let system_root = std::env::var_os("SYSTEMROOT").context("SYSTEMROOT is not set")?;
        let mut search_directories: Vec<PathBuf> = [
            tools.node.path.parent(),
            tools.python.path.parent(),
            tools.codex.path.parent(),
            git.parent(),
        ]
        .into_iter()
        .flatten()
        .map(Path::to_path_buf)
        .collect();
        search_directories.push(PathBuf::from(system_root).join("System32"));
        let search = std::env::join_paths(&search_directories)
            .context("building the owned Codex Security PATH")?;
        command
            .env("PATH", search)
            .env("PYTHON", &tools.python.path)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("PYTHONNOUSERSITE", "1")
            .env("PYTHONSAFEPATH", "1")
            .env("PYTHONUTF8", "1")
            .env("CODEX_SECURITY_GIT", git)
            .env("CODEX_SECURITY_SCAN_ROOT", &scratch.scan_root)
            .env("CODEX_SECURITY_STATE_DIR", &scratch.state)
            .env(
                "CODEX_SECURITY_DEEP_SCAN_CONFIG_PATH",
                scratch
                    .plugin
                    .join("scripts")
                    .join("deep_scan_defaults.json"),
            );
    }
    Ok(())
}
