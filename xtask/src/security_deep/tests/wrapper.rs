use super::*;

#[test]
fn private_child_command_specs_pin_arguments_and_strip_wrapper_controls() {
    let executable = Path::new(r"C:\runtime\tool.exe");
    let codex_arguments = [
        OsString::from("exec"),
        OsString::from("--experimental-json"),
    ];
    let codex = private_codex_child_command(executable, &codex_arguments);
    assert_eq!(codex.get_program(), executable);
    assert_eq!(
        codex.get_args().collect::<Vec<_>>(),
        codex_arguments
            .iter()
            .map(OsString::as_os_str)
            .collect::<Vec<_>>()
    );
    let codex_environment: BTreeMap<OsString, Option<OsString>> = codex
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
        .collect();
    for name in [
        WRAPPER_MARKER_ENV,
        WRAPPER_REAL_ENV,
        WRAPPER_OVERRIDES_ENV,
        WRAPPER_REPOSITORY_ENV,
        WRAPPER_NODE_ENV,
        WRAPPER_LIVE_AUTH_ENV,
        WRAPPER_PROVIDER_PERMISSION_ENV,
        WRAPPER_WORKER_PERMISSION_ENV,
        WRAPPER_TRANSITIONS_ENV,
        "CODEX_CLI_PATH",
        PYTHON_WRAPPER_REAL_ENV,
        PYTHON_WRAPPER_PLUGIN_ENV,
        PYTHON_WRAPPER_PYCACHE_ENV,
        PYTHON_WRAPPER_RUNTIME_ENV,
        "PYTHON",
    ] {
        assert_eq!(
            codex_environment.get(OsStr::new(name)),
            Some(&None),
            "real Codex retained {name}"
        );
    }

    let pycache = Path::new(r"C:\owned\pycache");
    let script = Path::new(r"C:\owned\plugin\scripts\workbench_db.py");
    let scripts = script.parent().unwrap();
    let script_arguments = [
        OsString::from("resolve-scan-root"),
        OsString::from("--scan-root"),
        OsString::from(r"C:\owned\scans"),
    ];
    let python =
        private_python_child_command(executable, pycache, script, scripts, &script_arguments)
            .unwrap();
    let expected_arguments: Vec<OsString> = [
        OsString::from("-I"),
        OsString::from("-S"),
        OsString::from("-B"),
        OsString::from("-X"),
        OsString::from("utf8"),
        OsString::from("-X"),
        OsString::from(r"pycache_prefix=C:\owned\pycache"),
        OsString::from("-c"),
        OsString::from(PYTHON_RUNPY_BOOTSTRAP),
        script.as_os_str().to_owned(),
        scripts.as_os_str().to_owned(),
    ]
    .into_iter()
    .chain(script_arguments)
    .collect();
    assert_eq!(
        python.get_args().collect::<Vec<_>>(),
        expected_arguments
            .iter()
            .map(OsString::as_os_str)
            .collect::<Vec<_>>()
    );
    let python_environment: BTreeMap<OsString, Option<OsString>> = python
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
        .collect();
    for name in [
        WRAPPER_MARKER_ENV,
        WRAPPER_REAL_ENV,
        WRAPPER_OVERRIDES_ENV,
        WRAPPER_REPOSITORY_ENV,
        WRAPPER_NODE_ENV,
        WRAPPER_LIVE_AUTH_ENV,
        WRAPPER_PROVIDER_PERMISSION_ENV,
        WRAPPER_WORKER_PERMISSION_ENV,
        WRAPPER_TRANSITIONS_ENV,
        "CODEX_CLI_PATH",
        PYTHON_WRAPPER_REAL_ENV,
        PYTHON_WRAPPER_PLUGIN_ENV,
        PYTHON_WRAPPER_PYCACHE_ENV,
        PYTHON_WRAPPER_RUNTIME_ENV,
    ] {
        assert_eq!(
            python_environment.get(OsStr::new(name)),
            Some(&None),
            "real Python retained {name}"
        );
    }
    for name in [
        "PYTHONDONTWRITEBYTECODE",
        "PYTHONNOUSERSITE",
        "PYTHONSAFEPATH",
        "PYTHONUTF8",
    ] {
        assert_eq!(
            python_environment.get(OsStr::new(name)),
            Some(&Some(OsString::from("1")))
        );
    }
}

#[cfg(windows)]
#[test]
fn wrapper_environment_propagates_both_exact_permission_profiles() {
    let (outer, scratch) = test_owned_scratch();
    let inert = inert_mcp_executable(&scratch.root).unwrap();
    let overrides = safety_overrides(&ambient_config(), &inert).unwrap();
    let policy = test_worker_policy(
        Path::new(r"C:\repo"),
        Path::new(r"C:\Users\owner\.codex\auth.json"),
        Path::new(r"C:\private-home\auth.json"),
    );
    let provider = provider_named_permission_profile(&policy).unwrap();
    let effective = effective_worker_named_permission_profile(&policy).unwrap();
    let mut command = Command::new("node");
    apply_wrapper_environment(
        &mut command,
        &source(),
        &owned_tools(),
        &policy,
        Path::new(r"C:\repo"),
        &scratch,
        &overrides,
    )
    .unwrap();
    let environment: BTreeMap<OsString, Option<OsString>> = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
        .collect();
    assert_eq!(
        environment.get(OsStr::new(WRAPPER_PROVIDER_PERMISSION_ENV)),
        Some(&Some(OsString::from(provider)))
    );
    assert_eq!(
        environment.get(OsStr::new(WRAPPER_WORKER_PERMISSION_ENV)),
        Some(&Some(OsString::from(effective)))
    );
    assert_eq!(
        environment.get(OsStr::new(WRAPPER_TRANSITIONS_ENV)),
        Some(&Some(scratch.transitions.clone().into_os_string()))
    );

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[cfg(windows)]
#[test]
fn wrapper_injects_safety_last_and_rejects_unpinned_shapes() {
    assert_eq!(WINDOWS_SANDBOX_OVERRIDE, "windows.sandbox=\"mxc\"");
    let (outer, root) = private_test_root();
    let inert = inert_mcp_executable(&root).unwrap();
    let overrides = safety_overrides(&ambient_config(), &inert).unwrap();
    assert!(require_wrapper_override_policy(&overrides[..overrides.len() - 1]).is_err());
    let mut wrong_last = overrides.clone();
    *wrong_last.last_mut().unwrap() = "windows.sandbox=\"unelevated\"".to_string();
    assert!(require_wrapper_override_policy(&wrong_last).is_err());
    let mut after_mxc = overrides.clone();
    after_mxc.push("plugins.after-mxc.enabled=false".to_string());
    assert!(require_wrapper_override_policy(&after_mxc).is_err());
    let mut duplicate_mxc = overrides.clone();
    duplicate_mxc.insert(
        duplicate_mxc.len() - 1,
        WINDOWS_SANDBOX_OVERRIDE.to_string(),
    );
    assert!(require_wrapper_override_policy(&duplicate_mxc).is_err());

    let app_policy = test_worker_policy(
        Path::new(r"C:\repo"),
        Path::new(r"C:\Users\owner\.codex\auth.json"),
        Path::new(r"C:\private-home\auth.json"),
    );
    let app_provider_permission = provider_named_permission_profile(&app_policy).unwrap();
    let app_worker_permission = effective_worker_named_permission_profile(&app_policy).unwrap();
    let app = vec![
        OsString::from("--config"),
        OsString::from("default_permissions=\"codex_security_deep_scan_worker\""),
        OsString::from("--config"),
        OsString::from(&app_provider_permission),
        OsString::from("app-server"),
        OsString::from("--stdio"),
    ];
    let injected = inject_wrapper_overrides_for_scan_root(
        &app,
        &overrides,
        None,
        &app_provider_permission,
        &app_worker_permission,
    )
    .unwrap();
    assert_eq!(
        injected.transition,
        WorkerTransitionKind::AppServerPassThrough
    );
    let injected = injected.arguments;
    let app_server = injected
        .iter()
        .position(|value| value == "app-server")
        .unwrap();
    assert_eq!(
        &injected[..4],
        &[
            OsString::from("--config"),
            OsString::from("default_permissions=\"codex_security_deep_scan_worker\""),
            OsString::from("--config"),
            OsString::from(&app_provider_permission),
        ]
    );
    assert_eq!(
        injected
            .iter()
            .filter(|value| value.as_os_str() == OsStr::new(&app_provider_permission))
            .count(),
        1
    );
    assert!(
        !injected
            .iter()
            .any(|value| value.as_os_str() == OsStr::new(&app_worker_permission))
    );
    assert_eq!(app_server, 4 + overrides.len() * 2);
    assert_eq!(
        &injected[app_server - 2..app_server],
        &[
            OsString::from("--config"),
            OsString::from(WINDOWS_SANDBOX_OVERRIDE),
        ]
    );
    assert_eq!(&injected[app_server..], &app[4..]);

    let repository = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
    let scan_root = root.join("scans");
    let scan_directory = scan_root
        .join(repository.file_name().unwrap())
        .join(format!("{}_20261014T000000_abcd", source().commit));
    let artifacts = scan_directory.join("artifacts");
    let worker_output = artifacts
        .join("deep_discovery")
        .join("workers")
        .join("discovery-0001")
        .join("output");
    let plugin = root.join("plugin");
    let pycache = root.join("pycache");
    std::fs::create_dir_all(&worker_output).unwrap();
    std::fs::create_dir_all(plugin.join("mcp")).unwrap();
    std::fs::create_dir(&pycache).unwrap();
    let scan_root = std::fs::canonicalize(scan_root).unwrap();
    let worker_output = std::fs::canonicalize(worker_output).unwrap();
    let plugin = std::fs::canonicalize(plugin).unwrap();
    let pycache = std::fs::canonicalize(pycache).unwrap();
    let python_wrapper = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
    let node = root.join("node.exe");
    std::fs::write(&node, b"synthetic node").unwrap();
    let node = std::fs::canonicalize(node).unwrap();
    let (node_bytes, node_digest) = measure_file(&node, 1024, "synthetic Node").unwrap();
    let artifact_node = ToolIdentity {
        path: node.clone(),
        bytes: node_bytes,
        sha256: sha256::hex(&node_digest),
        version: "synthetic".to_string(),
    };
    let python_real = root.join("python.exe");
    std::fs::write(&python_real, b"synthetic python").unwrap();
    std::fs::write(root.join("python3.dll"), b"synthetic python3").unwrap();
    std::fs::write(root.join("python312.dll"), b"synthetic python312").unwrap();
    std::fs::write(root.join("vcruntime140.dll"), b"synthetic runtime").unwrap();
    std::fs::create_dir(root.join("DLLs")).unwrap();
    std::fs::create_dir(root.join("Lib")).unwrap();
    std::fs::write(root.join("DLLs").join("probe.dll"), b"probe").unwrap();
    std::fs::write(root.join("Lib").join("probe.py"), b"probe").unwrap();
    let python_real = std::fs::canonicalize(python_real).unwrap();
    let codex_runtime_root = root.join("codex-runtime");
    std::fs::create_dir(&codex_runtime_root).unwrap();
    let codex_runtime_root = std::fs::canonicalize(codex_runtime_root).unwrap();
    let live_auth = root.join("live").join("auth.json");
    let projected_auth = root.join("codex-home").join("auth.json");
    let context = PrivateExecutionContext {
        scan_root,
        plugin_mirror: plugin.clone(),
        repository: repository.clone(),
        python_wrapper: python_wrapper.clone(),
        python_runtime: PythonRuntimeIdentity {
            root: python_real.parent().unwrap().to_path_buf(),
            entries: 1,
            bytes: 1,
            sha256: "f".repeat(64),
        },
        python_real,
        pycache,
        artifact_node,
        codex_runtime: CodexRuntimeIdentity {
            root: codex_runtime_root,
            entries: 1,
            bytes: 1,
            sha256: "e".repeat(64),
        },
        live_auth,
        projected_auth,
        shell_environment: ShellEnvironmentPolicy {
            system_root: PathBuf::from(r"C:\WINDOWS"),
            command_processor: PathBuf::from(r"C:\WINDOWS\System32\cmd.exe"),
            search_path: OsString::from(r"C:\owned\bin;C:\WINDOWS\System32"),
            temporary: root.join("temp"),
        },
        marker: format!("{WRAPPER_MARKER_PREFIX}{}", source().commit),
    };
    let (provider_permission, worker_permission) =
        context_named_permission_profiles(&context).unwrap();
    let (python_reads, python_denies, _) =
        python_worker_policy_entries(&context.python_runtime).unwrap();
    let mut readable_entries = vec![
        context.repository.clone(),
        context.plugin_mirror.clone(),
        context.scan_root.clone(),
        context.artifact_node.path.clone(),
        context.codex_runtime.root.clone(),
        context.python_wrapper.clone(),
    ];
    readable_entries.extend(python_reads);
    let mut denied_entries = vec![context.repository.join(".git")];
    denied_entries.extend(python_denies);
    let equivalent_policy = ManagedWorkerPolicy {
        repository: context.repository.clone(),
        probe_directory: context.scan_root.clone(),
        readable_entries,
        denied_entries,
        readable_probes: Vec::new(),
        denied_probes: Vec::new(),
        denied_write_probes: Vec::new(),
        existing_write_probe: context.scan_root.join(WORKER_ALLOWED_CANARY_NAME),
        repository_metadata_probe: context.repository.join(".git").join("HEAD"),
        live_auth: context.live_auth.clone(),
        projected_auth: context.projected_auth.clone(),
        denied_canary: root.join(WORKER_DENIED_CANARY_NAME),
    };
    assert_eq!(
        provider_permission,
        provider_named_permission_profile(&equivalent_policy).unwrap()
    );
    assert_eq!(
        worker_permission,
        effective_worker_named_permission_profile(&equivalent_policy).unwrap()
    );
    let discovery_context = object([
        ("scanId", text_value(TEST_SCAN_ID)),
        ("pluginRoot", text_value(plugin.to_str().unwrap())),
        ("targetPath", text_value(repository.to_str().unwrap())),
        ("scope", text_value(".")),
        ("userContext", JsonValue::Null),
        ("workerLabel", text_value("discovery-0001")),
        ("subagents", JsonValue::Number("3".to_string())),
    ]);
    let prompt = format!(
        "{DISCOVERY_PROMPT_PREFIX}{}\n```\n{DISCOVERY_PROMPT_BODY_PREFIX}",
        render_json(&discovery_context).unwrap()
    );
    std::fs::write(worker_output.parent().unwrap().join("prompt.md"), prompt).unwrap();
    let provider_configs = vec![
        "default_permissions=\"codex_security_deep_scan_worker\"".to_string(),
        provider_permission.clone(),
        format!("model_reasoning_effort=\"{REASONING}\""),
        "mcp_servers.codex-security.command=\"node\"".to_string(),
        "mcp_servers.codex-security.enabled=false".to_string(),
        format!(
            "mcp_servers.cs_artifacts.command={}",
            quote_toml_string(&node.to_string_lossy())
        ),
        format!(
            "mcp_servers.cs_artifacts.args=[{}, \"--artifact-writer\", \"--stdio\"]",
            quote_toml_string(&plugin.join("mcp").join("server.mjs").to_string_lossy())
        ),
        "mcp_servers.cs_artifacts.required=true".to_string(),
        "mcp_servers.cs_artifacts.startup_timeout_sec=180".to_string(),
        "mcp_servers.cs_artifacts.tool_timeout_sec=86400".to_string(),
        "agents.max_threads=3".to_string(),
        "features.multi_agent_v2.enabled=false".to_string(),
        "features.multi_agent_v2.max_concurrent_threads_per_session=4".to_string(),
        "approval_policy=\"never\"".to_string(),
        format!(
            "mcp_servers.cs_artifacts.env.CODEX_SECURITY_ARTIFACT_ROOT={}",
            quote_toml_string(worker_output.to_str().unwrap())
        ),
        format!(
            "mcp_servers.cs_artifacts.env.CODEX_SECURITY_REPO_ROOT={}",
            quote_toml_string(repository.to_str().unwrap())
        ),
        "mcp_servers.cs_artifacts.env.CODEX_SECURITY_ARTIFACT_LAYOUT=\"worker\"".to_string(),
        format!("mcp_servers.cs_artifacts.env.CODEX_SECURITY_SCAN_ID=\"{TEST_SCAN_ID}\""),
        format!(
            "mcp_servers.cs_artifacts.env.CODEX_SECURITY_PLUGIN_ROOT={}",
            quote_toml_string(plugin.to_str().unwrap())
        ),
        "mcp_servers.cs_artifacts.env.CODEX_SECURITY_SCOPE=\".\"".to_string(),
        format!(
            "mcp_servers.cs_artifacts.env.CODEX_SECURITY_PYTHON_COMMAND={}",
            quote_toml_string(python_wrapper.to_str().unwrap())
        ),
    ];
    let make_exec = |resume: bool| {
        let mut exec = vec![
            OsString::from("exec"),
            OsString::from("--experimental-json"),
        ];
        for config in &provider_configs {
            exec.push(OsString::from("--config"));
            exec.push(OsString::from(config));
        }
        exec.extend([
            OsString::from("--model"),
            OsString::from(MODEL),
            OsString::from("--cd"),
            worker_output.as_os_str().to_owned(),
            OsString::from("--skip-git-repo-check"),
        ]);
        if resume {
            exec.extend([OsString::from("resume"), OsString::from("thread-1")]);
        } else {
            exec.extend([
                OsString::from("--thread-source"),
                OsString::from("security_scan"),
            ]);
        }
        exec
    };
    let exec = make_exec(true);
    let injected = inject_wrapper_overrides_for_scan_root(
        &exec,
        &overrides,
        Some(&context),
        &provider_permission,
        &worker_permission,
    )
    .unwrap();
    assert_eq!(
        injected.transition,
        WorkerTransitionKind::ExecResumedAttenuated
    );
    assert_eq!(injected.expected_thread.as_deref(), Some("thread-1"));
    assert_eq!(
        injected.worker.as_ref().map(|worker| worker.path.as_path()),
        Some(worker_output.as_path())
    );
    let injected = injected.arguments;
    let wrong_worker_permission =
        worker_permission.replace("\":minimal\"=\"read\"", "\":minimal\"=\"deny\"");
    assert_ne!(wrong_worker_permission, worker_permission);
    assert!(
        inject_wrapper_overrides_for_scan_root(
            &exec,
            &overrides,
            Some(&context),
            &provider_permission,
            &wrong_worker_permission,
        )
        .is_err()
    );
    let resume = injected.iter().position(|value| value == "resume").unwrap();
    assert_eq!(
        resume,
        exec.len() - 2
            + 4
            + 1
            + (overrides.len() + context.artifact_overrides().unwrap().len()) * 2
    );
    assert_eq!(&injected[resume..], &exec[exec.len() - 2..]);
    assert_eq!(
        &injected[resume - 2..resume],
        &[
            OsString::from("--config"),
            OsString::from(WINDOWS_SANDBOX_OVERRIDE),
        ]
    );
    assert_eq!(injected.first(), Some(&OsString::from("--no-daemon")));
    assert!(
        !injected
            .iter()
            .any(|value| value.as_os_str() == OsStr::new(&provider_permission))
    );
    assert_eq!(
        injected
            .iter()
            .filter(|value| value.as_os_str() == OsStr::new(&worker_permission))
            .count(),
        1
    );
    for isolation in [
        "--ephemeral",
        "--ignore-user-config",
        "--ignore-rules",
        "--strict-config",
    ] {
        assert_eq!(
            injected
                .iter()
                .filter(|value| value.as_os_str() == OsStr::new(isolation))
                .count(),
            1
        );
        let mut supplied = make_exec(false);
        supplied.push(OsString::from(isolation));
        assert!(
            inject_wrapper_overrides_for_scan_root(
                &supplied,
                &overrides,
                Some(&context),
                &provider_permission,
                &worker_permission,
            )
            .is_err()
        );
    }
    for private in context.artifact_overrides().unwrap() {
        assert!(injected.contains(&OsString::from(private)));
    }

    let initial = make_exec(false);
    let initial_injected = inject_wrapper_overrides_for_scan_root(
        &initial,
        &overrides,
        Some(&context),
        &provider_permission,
        &worker_permission,
    )
    .unwrap();
    assert_eq!(
        initial_injected.transition,
        WorkerTransitionKind::ExecInitialAttenuated
    );
    assert_eq!(initial_injected.expected_thread, None);
    assert_eq!(
        initial_injected
            .worker
            .as_ref()
            .map(|worker| worker.path.as_path()),
        Some(worker_output.as_path())
    );
    let initial_injected = initial_injected.arguments;
    assert_eq!(
        &initial_injected[initial_injected.len() - 2..],
        &[
            OsString::from("--config"),
            OsString::from(WINDOWS_SANDBOX_OVERRIDE),
        ]
    );
    assert!(
        !initial_injected
            .iter()
            .any(|value| value.as_os_str() == OsStr::new(&provider_permission))
    );
    assert_eq!(
        initial_injected
            .iter()
            .filter(|value| value.as_os_str() == OsStr::new(&worker_permission))
            .count(),
        1
    );

    for forbidden in [
        "--sandbox",
        "--add-dir",
        "--cyber-access-program",
        "--output-schema",
        "--image",
    ] {
        let mut rejected = initial.clone();
        rejected.extend([OsString::from(forbidden), OsString::from("value")]);
        assert!(
            inject_wrapper_overrides_for_scan_root(
                &rejected,
                &overrides,
                Some(&context),
                &provider_permission,
                &worker_permission,
            )
            .is_err(),
            "accepted forbidden {forbidden}"
        );
    }

    let mut wrong_model = initial.clone();
    let model = wrong_model.iter().position(|value| value == MODEL).unwrap();
    wrong_model[model] = OsString::from("other-model");
    let mut duplicate_model = initial.clone();
    duplicate_model.extend([OsString::from("--model"), OsString::from(MODEL)]);
    let outside = root.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let outside = std::fs::canonicalize(outside).unwrap();
    let mut wrong_cd = initial.clone();
    let cd = wrong_cd
        .iter()
        .position(|value| value == worker_output.as_os_str())
        .unwrap();
    wrong_cd[cd] = outside.as_os_str().to_owned();
    let mut source_and_resume = initial.clone();
    source_and_resume.extend([OsString::from("resume"), OsString::from("thread-1")]);
    let mut extra_config = initial.clone();
    extra_config.extend([
        OsString::from("--config"),
        OsString::from("model_provider=\"other\""),
    ]);
    let mut wrong_scan = initial.clone();
    let expected_scan_config =
        format!("mcp_servers.cs_artifacts.env.CODEX_SECURITY_SCAN_ID=\"{TEST_SCAN_ID}\"");
    let scan_config = wrong_scan
        .iter()
        .position(|value| value.as_os_str() == OsStr::new(&expected_scan_config))
        .unwrap();
    wrong_scan[scan_config] =
        OsString::from("mcp_servers.cs_artifacts.env.CODEX_SECURITY_SCAN_ID=\"scan-2\"");
    for (name, rejected) in [
        ("wrong model", wrong_model),
        ("duplicate model", duplicate_model),
        ("wrong directory", wrong_cd),
        ("source and resume", source_and_resume),
        ("extra config", extra_config),
        ("wrong scan identity", wrong_scan),
    ] {
        assert!(
            inject_wrapper_overrides_for_scan_root(
                &rejected,
                &overrides,
                Some(&context),
                &provider_permission,
                &worker_permission,
            )
            .is_err(),
            "accepted {name}"
        );
    }

    for rejected in [
        vec!["--version"],
        vec!["app-server", "--stdio", "extra"],
        vec!["exec", "--experimental-json", "prompt"],
        vec!["exec", "--experimental-json", "resume", "thread", "extra"],
    ] {
        let rejected = rejected.into_iter().map(OsString::from).collect::<Vec<_>>();
        assert!(
            inject_wrapper_overrides_for_scan_root(
                &rejected,
                &overrides,
                Some(&context),
                &provider_permission,
                &worker_permission,
            )
            .is_err()
        );
    }
    let metadata = std::fs::symlink_metadata(&root).unwrap();
    remove_entry_no_follow(&root, &metadata).unwrap();
    std::fs::remove_dir(&outer).unwrap();
}

#[test]
fn raw_config_segments_and_inert_routes_are_exact() {
    let (outer, root) = private_test_root();
    let inert = inert_mcp_executable(&root).unwrap();
    let ambient = ambient_config();
    let overrides = safety_overrides(&ambient, &inert).unwrap();
    require_wrapper_override_policy(&overrides).unwrap();
    assert!(overrides.contains(&"mcp_servers.node_repl.enabled=false".to_string()));
    assert!(overrides.contains(&"plugins.browser@openai-bundled.enabled=false".to_string()));
    assert!(
        !overrides
            .iter()
            .any(|value| value.contains(".\"node_repl\"."))
    );

    // The pinned CLI treats quotes in a dotted override as literal name
    // bytes, so accepting this would create a second, still-enabled route.
    let quoted = r#"mcp_servers."node_repl".enabled=false"#;
    let literal_name = quoted
        .strip_prefix("mcp_servers.")
        .unwrap()
        .split_once('.')
        .unwrap()
        .0;
    assert_eq!(literal_name, r#""node_repl""#);
    assert!(require_config_name(literal_name).is_err());

    let effective = safe_config(&inert);
    require_safe_effective_config(&effective, &ambient, &inert).unwrap();
    let mut routed = effective.clone();
    routed
        .mcp_servers
        .get_mut("node_repl")
        .unwrap()
        .fields
        .insert("url".to_string());
    assert!(require_safe_effective_config(&routed, &ambient, &inert).is_err());
    let mut enabled = effective;
    enabled.mcp_servers.get_mut("node_repl").unwrap().enabled = true;
    assert!(require_safe_effective_config(&enabled, &ambient, &inert).is_err());

    std::fs::write(&inert, b"not executable").unwrap();
    assert!(require_safe_effective_config(&safe_config(&inert), &ambient, &inert).is_err());
    std::fs::remove_file(&inert).unwrap();
    std::fs::remove_dir(&root).unwrap();
    std::fs::remove_dir(&outer).unwrap();
}

#[test]
fn python_argument_boundaries_are_exact_and_owned() {
    let root = unique_test_directory("wizard-security-deep-python-args-test");
    let repository = root.join("repository");
    let scan_root = root.join("scans");
    let state_root = root.join("state");
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(&scan_root).unwrap();
    std::fs::create_dir(&state_root).unwrap();
    let repository = std::fs::canonicalize(repository).unwrap();
    let scan_root = std::fs::canonicalize(scan_root).unwrap();
    let state_root = std::fs::canonicalize(state_root).unwrap();
    let output_parent = scan_root
        .join("repository")
        .join("scan")
        .join("artifacts")
        .join("02_discovery");
    std::fs::create_dir_all(&output_parent).unwrap();
    let output = output_parent.join("in_scope_files.txt");
    let inventory = [
        OsString::from("--repo"),
        repository.as_os_str().to_owned(),
        OsString::from("--scope"),
        OsString::from("."),
        OsString::from("--out"),
        output.as_os_str().to_owned(),
    ];
    validate_inventory_arguments(&inventory, &repository, &scan_root).unwrap();
    let mut wrong_scope = inventory.clone();
    wrong_scope[3] = OsString::from("src");
    assert!(validate_inventory_arguments(&wrong_scope, &repository, &scan_root).is_err());
    let mut outside = inventory.clone();
    outside[5] = root.join("outside.txt").into_os_string();
    assert!(validate_inventory_arguments(&outside, &repository, &scan_root).is_err());

    let resolve = [
        OsString::from("resolve-scan-root"),
        OsString::from("--scan-root"),
        scan_root.as_os_str().to_owned(),
    ];
    validate_workbench_arguments(&resolve, &repository, &scan_root, &state_root).unwrap();
    for rejected in [
        vec![OsString::from("unknown")],
        vec![
            OsString::from("resolve-scan-root"),
            OsString::from("--scan-root"),
            root.join("outside").into_os_string(),
        ],
        vec![
            OsString::from("resolve-scan-root"),
            OsString::from("--scan-root"),
            scan_root.as_os_str().to_owned(),
            OsString::from("--scan-root"),
            scan_root.as_os_str().to_owned(),
        ],
    ] {
        assert!(
            validate_workbench_arguments(&rejected, &repository, &scan_root, &state_root).is_err()
        );
    }

    assert_eq!(
        classify_private_wrapper(&[OsString::from("exec")]).unwrap(),
        PrivateWrapperKind::Codex
    );
    assert_eq!(
        classify_private_wrapper(&[repository.join("script.py").into_os_string()]).unwrap(),
        PrivateWrapperKind::Python
    );
    assert!(classify_private_wrapper(&[]).is_err());
    assert!(classify_private_wrapper(&[OsString::from("-c")]).is_err());

    let metadata = std::fs::symlink_metadata(&root).unwrap();
    remove_entry_no_follow(&root, &metadata).unwrap();
}

#[cfg(windows)]
#[test]
fn python_runtime_policy_and_resource_bounds_detect_drift() {
    let outer = unique_test_directory("wizard-security-deep-python-runtime-test");
    let runtime = outer.join("runtime");
    std::fs::create_dir(&runtime).unwrap();
    for name in [
        "python.exe",
        "python3.dll",
        "python312.dll",
        "vcruntime140.dll",
    ] {
        std::fs::write(runtime.join(name), name.as_bytes()).unwrap();
    }
    std::fs::create_dir(runtime.join("DLLs")).unwrap();
    std::fs::create_dir(runtime.join("Lib")).unwrap();
    std::fs::write(runtime.join("Lib").join("runpy.py"), b"original").unwrap();
    let runtime = std::fs::canonicalize(runtime).unwrap();
    let executable = runtime.join("python.exe");
    let identity = identify_python_runtime(&executable).unwrap();
    assert_eq!(
        parse_python_runtime_policy(&render_python_runtime_policy(&identity).unwrap()).unwrap(),
        identity
    );
    std::fs::write(runtime.join("Lib").join("runpy.py"), b"replaced").unwrap();
    assert_ne!(identify_python_runtime(&executable).unwrap(), identity);
    std::fs::write(runtime.join("pyvenv.cfg"), b"home=other").unwrap();
    assert!(identify_python_runtime(&executable).is_err());
    std::fs::remove_file(runtime.join("pyvenv.cfg")).unwrap();

    let bounded = outer.join("bounded");
    std::fs::create_dir(&bounded).unwrap();
    for name in ["c", "a", "b"] {
        std::fs::write(bounded.join(name), b"x").unwrap();
    }
    assert!(bounded_sorted_directory_entries(&bounded, 2).is_err());
    let names = bounded_sorted_directory_entries(&bounded, 3)
        .unwrap()
        .into_iter()
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            OsString::from("a"),
            OsString::from("b"),
            OsString::from("c")
        ]
    );

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn wrapper_dispatch_is_early_and_strips_control_environment() {
    let main = include_str!("../../main.rs");
    assert!(
        main.find("dispatch_private_codex_wrapper").unwrap() < main.find("cli::parse").unwrap()
    );
    let implementation = concat!(
        include_str!("../wrapper.rs"),
        include_str!("../transitions.rs"),
        include_str!("../wrapper/python.rs"),
        include_str!("../wrapper/workbench.rs"),
        include_str!("../wrapper/overrides.rs")
    );
    let dispatch = implementation
        .find("fn dispatch_private_wrapper_inner")
        .unwrap();
    let parser = implementation[dispatch..]
        .find("fn parse_wrapper_overrides")
        .unwrap()
        + dispatch;
    let body = &implementation[dispatch..parser];
    for variable in [
        "WRAPPER_MARKER_ENV",
        "WRAPPER_REAL_ENV",
        "WRAPPER_OVERRIDES_ENV",
        "WRAPPER_LIVE_AUTH_ENV",
        "WRAPPER_PROVIDER_PERMISSION_ENV",
        "WRAPPER_WORKER_PERMISSION_ENV",
        "WRAPPER_TRANSITIONS_ENV",
        "\"CODEX_CLI_PATH\"",
    ] {
        assert!(body.contains(&format!(".env_remove({variable})")));
    }
    let child = body.find("private_codex_child_command").unwrap();
    let receipt = body.find("write_worker_transition_receipt").unwrap();
    let credential = body.find("require_private_credential_projection").unwrap();
    assert!(credential < child && child < receipt);
    let app_server_relay = body.find("relay_private_child_stdout").unwrap();
    assert!(receipt < app_server_relay);
    assert!(body.contains("WorkerTransitionStage::Launched"));
    assert!(body.contains("WorkerTransitionStage::Started"));
    assert!(body.contains("WorkerTransitionStage::ExitedZero"));
    let exit_guard = body.find("if status.code() == Some(0)").unwrap();
    let exit_receipt = body[exit_guard..]
        .find("write_worker_transition_receipt")
        .unwrap()
        + exit_guard;
    assert!(body[exit_guard..exit_receipt].contains("Some(0)"));
    assert!(body[exit_receipt..].starts_with("write_worker_transition_receipt"));
    let relay_start = body.find("fn relay_private_exec_line").unwrap();
    let relay_end = body[relay_start..]
        .find("fn observe_private_exec_line")
        .unwrap()
        + relay_start;
    let relay = &body[relay_start..relay_end];
    let relay_receipt = relay.find("write_worker_transition_receipt").unwrap();
    let relay_output = relay.find(".write_all(line)").unwrap();
    assert!(relay_receipt < relay_output);
    assert_eq!(body.match_indices("identify_codex_runtime").count(), 2);
}
