use super::*;

#[test]
fn worker_probe_summary_requires_the_complete_bounded_matrix() {
    let complete = owned_probe_summary();
    complete.require_complete().unwrap();
    let mut incomplete = Vec::new();
    let mut summary = complete.clone();
    summary.allowed_reads = 0;
    incomplete.push(summary);
    let mut summary = complete.clone();
    summary.denied_reads = 3;
    incomplete.push(summary);
    let mut summary = complete.clone();
    summary.denied_write_creates = 1;
    incomplete.push(summary);
    let mut summary = complete.clone();
    summary.denied_write_mutations = 2;
    incomplete.push(summary);
    let mut summary = complete.clone();
    summary.denied_network_listeners = 0;
    incomplete.push(summary);
    let mut summary = complete.clone();
    summary.denied_network_connections = 0;
    incomplete.push(summary);
    let mut summary = complete.clone();
    summary.denied_descendants = 0;
    incomplete.push(summary);
    let mut summary = complete;
    summary.allowed_reads = MAX_WORKER_POLICY_PROBES + 1;
    incomplete.push(summary);
    assert!(
        incomplete
            .iter()
            .all(|summary| summary.require_complete().is_err())
    );
}

#[cfg(windows)]
#[test]
fn owned_metadata_names_the_exact_managed_read_only_parent() {
    let source = source();
    let repository = Path::new(r"C:\repo");
    let live_auth = Path::new(r"C:\Users\owner\.codex\auth.json");
    let projected_auth = Path::new(r"C:\private-home\auth.json");
    let policy = test_worker_policy(repository, live_auth, projected_auth);
    let metadata = call_metadata(&policy, &source).unwrap();
    let metadata = json_object(&metadata, "test metadata").unwrap();
    let state = json_object(
        json_field(metadata, "codex/sandbox-state-meta", "test metadata").unwrap(),
        "test sandbox state",
    )
    .unwrap();
    assert_eq!(
        string(state, "sandboxCwd", "test sandbox state").unwrap(),
        "file:///C:/repo"
    );
    assert_eq!(state.get("useMxc"), Some(&JsonValue::Bool(true)));
    let profile = json_object(
        json_field(state, "permissionProfile", "test sandbox state").unwrap(),
        "test permission profile",
    )
    .unwrap();
    assert_eq!(
        string(profile, "type", "test permission profile").unwrap(),
        "managed"
    );
    assert_eq!(
        string(profile, "network", "test permission profile").unwrap(),
        "restricted"
    );
    let filesystem = json_object(
        json_field(profile, "file_system", "test permission profile").unwrap(),
        "test filesystem profile",
    )
    .unwrap();
    assert_eq!(
        string(filesystem, "type", "test filesystem profile").unwrap(),
        "restricted"
    );
    let entries = json_array(
        json_field(filesystem, "entries", "test filesystem profile").unwrap(),
        "test filesystem entries",
    )
    .unwrap();
    assert_eq!(entries.len(), 4);
    assert!(
        render_json(&entries[0])
            .unwrap()
            .contains("\"access\":\"read\"")
    );
    assert!(
        render_json(&entries[0])
            .unwrap()
            .contains("\"kind\":\"root\"")
    );
    let entry_path = |entry: &JsonValue| {
        let entry = json_object(entry, "test filesystem entry").unwrap();
        let path = json_object(
            json_field(entry, "path", "test filesystem entry").unwrap(),
            "test filesystem path",
        )
        .unwrap();
        string(path, "path", "test filesystem path")
            .unwrap()
            .to_string()
    };
    assert_eq!(entry_path(&entries[1]), r"C:\repo\.git");
    assert_eq!(entry_path(&entries[2]), r"C:\Users\owner\.codex\auth.json");
    assert_eq!(entry_path(&entries[3]), r"C:\private-home\auth.json");
    for entry in &entries[1..] {
        assert!(render_json(entry).unwrap().contains("\"access\":\"deny\""));
    }
    let rendered = render_json(&JsonValue::Object(metadata.clone())).unwrap();
    assert!(!rendered.contains("worker-denied-read-canary"));
}

#[cfg(windows)]
#[test]
fn named_permission_profiles_separate_provider_and_effective_authority() {
    let policy = test_worker_policy(
        Path::new(r"C:\repo"),
        Path::new(r"C:\Users\owner\.codex\auth.json"),
        Path::new(r"C:\private-home\auth.json"),
    );
    let provider = provider_named_permission_profile(&policy).unwrap();
    let effective = effective_worker_named_permission_profile(&policy).unwrap();
    assert_eq!(
        provider,
        r#"permissions.codex_security_deep_scan_worker={extends=":read-only",filesystem={":root"="read","C:\\repo\\.git"="deny","C:\\Users\\owner\\.codex\\auth.json"="deny","C:\\private-home\\auth.json"="deny"},network={enabled=false}}"#
    );
    assert_eq!(
        effective,
        r#"permissions.codex_security_deep_scan_worker={extends=":read-only",filesystem={":root"="deny",":minimal"="read","C:\\repo"="read","C:\\repo\\.git"="deny","C:\\Users\\owner\\.codex\\auth.json"="deny","C:\\private-home\\auth.json"="deny"},network={enabled=false}}"#
    );
    let digests = worker_permission_digests(&policy).unwrap();
    assert_eq!(
        digests.provider_profile,
        sha256::hex(&sha256::digest(provider.as_bytes()))
    );
    assert_eq!(
        digests.effective_profile,
        sha256::hex(&sha256::digest(effective.as_bytes()))
    );
    let parent = render_json(&provider_parent_sandbox_state(&policy).unwrap()).unwrap();
    let effective_state = render_json(&managed_read_only_sandbox_state(&policy).unwrap()).unwrap();
    assert_eq!(
        digests.parent_state,
        sha256::hex(&sha256::digest(parent.as_bytes()))
    );
    assert_ne!(digests.parent_state, digests.provider_profile);
    assert_eq!(
        digests.effective_state,
        sha256::hex(&sha256::digest(effective_state.as_bytes()))
    );
    assert_ne!(digests.effective_state, digests.effective_profile);
}

#[test]
fn credential_probe_is_exact_content_open_only_and_fails_closed() {
    #[cfg(windows)]
    assert_eq!(FILE_READ_DATA_ACCESS, 1);
    assert_eq!(
        private_credential_probe_exit(Err(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ))),
        PRIVATE_CREDENTIAL_DENIED_EXIT
    );
    assert_eq!(
        private_credential_probe_exit(Ok(())),
        PRIVATE_CREDENTIAL_READABLE_EXIT
    );
    assert_eq!(
        private_credential_probe_exit(Err(std::io::Error::from(std::io::ErrorKind::NotFound,))),
        PRIVATE_CREDENTIAL_ERROR_EXIT
    );
    require_worker_read_probe_result(
        Some(PRIVATE_CREDENTIAL_DENIED_EXIT),
        b"",
        b"",
        PRIVATE_CREDENTIAL_DENIED_EXIT,
        "deny",
    )
    .unwrap();
    require_worker_read_probe_result(
        Some(PRIVATE_CREDENTIAL_READABLE_EXIT),
        b"",
        b"",
        PRIVATE_CREDENTIAL_READABLE_EXIT,
        "allow",
    )
    .unwrap();
    for (code, stdout, stderr) in [
        (Some(PRIVATE_CREDENTIAL_READABLE_EXIT), &b""[..], &b""[..]),
        (Some(PRIVATE_CREDENTIAL_ERROR_EXIT), &b""[..], &b""[..]),
        (None, &b""[..], &b""[..]),
        (
            Some(PRIVATE_CREDENTIAL_DENIED_EXIT),
            &b"noise"[..],
            &b""[..],
        ),
        (
            Some(PRIVATE_CREDENTIAL_DENIED_EXIT),
            &b""[..],
            &b"noise"[..],
        ),
    ] {
        let error = require_worker_read_probe_result(
            code,
            stdout,
            stderr,
            PRIVATE_CREDENTIAL_DENIED_EXIT,
            "deny",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains(&format!(
            "exit={code:?}, stdout-bytes={}, stderr-bytes={}",
            stdout.len(),
            stderr.len()
        )));
        assert!(!error.contains("noise"));
    }

    let tag = OsString::from(PRIVATE_CREDENTIAL_PROBE_ARGUMENT);
    assert!(dispatch_private_credential_probe(std::slice::from_ref(&tag)).is_err());
    assert!(
        dispatch_private_credential_probe(&[
            OsString::from("--not-the-private-credential-probe"),
            std::env::temp_dir().into_os_string(),
        ])
        .is_err()
    );
    assert!(
        dispatch_private_credential_probe(&[tag, OsString::from("relative-auth.json")]).is_err()
    );
    assert!(dispatch_private_write_probe(&[]).is_err());
    assert!(dispatch_private_existing_write_probe(&[]).is_err());
    assert!(
        dispatch_private_network_probe(&[
            OsString::from(PRIVATE_NETWORK_PROBE_ARGUMENT),
            OsString::from("unexpected"),
        ])
        .is_err()
    );
    assert!(
        dispatch_private_network_connect_probe(&[
            OsString::from(PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT),
            OsString::from("8.8.8.8:53"),
        ])
        .is_err()
    );
    assert!(
        dispatch_private_network_connect_probe(&[
            OsString::from(PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT),
            OsString::from("127.0.0.1:0"),
        ])
        .is_err()
    );
    assert!(
        dispatch_private_descendant_probe(&[OsString::from(PRIVATE_DESCENDANT_PROBE_ARGUMENT,)])
            .is_err()
    );

    let positive_root = unique_test_directory("wizard-worker-probe-positive");
    let write_target = positive_root.join("fresh-write-target");
    assert_eq!(
        dispatch_private_write_probe(&[
            OsString::from(PRIVATE_WRITE_PROBE_ARGUMENT),
            write_target.clone().into_os_string(),
        ])
        .unwrap(),
        PRIVATE_CREDENTIAL_READABLE_EXIT
    );
    assert!(write_target.is_file());
    let existing_target = positive_root.join("existing-write-target");
    std::fs::write(&existing_target, b"before").unwrap();
    assert_eq!(
        dispatch_private_existing_write_probe(&[
            OsString::from(PRIVATE_WRITE_APPEND_PROBE_ARGUMENT),
            existing_target.clone().into_os_string(),
        ])
        .unwrap(),
        PRIVATE_CREDENTIAL_READABLE_EXIT
    );
    assert_eq!(
        std::fs::read(&existing_target).unwrap(),
        b"beforewizard append probe"
    );
    std::fs::write(&existing_target, b"before").unwrap();
    assert_eq!(
        dispatch_private_existing_write_probe(&[
            OsString::from(PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT),
            existing_target.clone().into_os_string(),
        ])
        .unwrap(),
        PRIVATE_CREDENTIAL_READABLE_EXIT
    );
    assert!(std::fs::read(&existing_target).unwrap().is_empty());
    std::fs::write(&existing_target, b"before-before-before-before").unwrap();
    assert_eq!(
        dispatch_private_existing_write_probe(&[
            OsString::from(PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT),
            existing_target.clone().into_os_string(),
        ])
        .unwrap(),
        PRIVATE_CREDENTIAL_READABLE_EXIT
    );
    assert!(
        std::fs::read(&existing_target)
            .unwrap()
            .starts_with(b"wizard overwrite probe")
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    assert_eq!(
        dispatch_private_network_connect_probe(&[
            OsString::from(PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT),
            OsString::from(endpoint),
        ])
        .unwrap(),
        PRIVATE_CREDENTIAL_READABLE_EXIT
    );
    drop(listener);
    let metadata = std::fs::symlink_metadata(&positive_root).unwrap();
    remove_entry_no_follow(&positive_root, &metadata).unwrap();

    let implementation = concat!(include_str!("../wrapper.rs"), include_str!("../probe.rs"));
    let dispatch = implementation
        .find("pub(crate) fn dispatch_private_wrapper")
        .unwrap();
    let probe = implementation[dispatch..]
        .find("fn dispatch_private_credential_probe")
        .unwrap()
        + dispatch;
    let dispatch_body = &implementation[dispatch..probe];
    let marker = dispatch_body.find("WRAPPER_MARKER_ENV").unwrap();
    for probe in [
        "PRIVATE_CREDENTIAL_PROBE_ARGUMENT",
        "PRIVATE_WRITE_PROBE_ARGUMENT",
        "PRIVATE_WRITE_APPEND_PROBE_ARGUMENT",
        "PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT",
        "PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT",
        "PRIVATE_NETWORK_PROBE_ARGUMENT",
        "PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT",
        "PRIVATE_DESCENDANT_PROBE_ARGUMENT",
    ] {
        assert!(dispatch_body.find(probe).unwrap() < marker);
    }
    let start = implementation
        .find("fn open_private_credential_for_probe")
        .unwrap();
    let end = implementation[start..]
        .find("#[cfg(not(windows))]")
        .unwrap()
        + start;
    let body = &implementation[start..end];
    assert!(body.contains(".access_mode(FILE_READ_DATA_ACCESS)"));
    assert!(!body.contains(".read("));
    assert!(!body.contains("read_to_end"));
    assert!(!body.contains("std::fs::read"));
    let network_start = implementation
        .find("fn dispatch_private_network_probe")
        .unwrap();
    let descendant_start = implementation[network_start..]
        .find("fn dispatch_private_descendant_probe")
        .unwrap()
        + network_start;
    let credential_open = implementation[descendant_start..]
        .find("fn open_private_credential_for_probe")
        .unwrap()
        + descendant_start;
    assert!(
        implementation[network_start..descendant_start]
            .contains("std::net::TcpListener::bind(\"127.0.0.1:0\").map(drop)")
    );
    assert!(
        implementation[network_start..descendant_start]
            .contains("std::net::TcpStream::connect_timeout")
    );
    let existing_write_start = implementation
        .find("fn dispatch_private_existing_write_probe")
        .unwrap();
    let existing_write_end = implementation[existing_write_start..]
        .find("fn dispatch_private_network_probe")
        .unwrap()
        + existing_write_start;
    let existing_write = &implementation[existing_write_start..existing_write_end];
    for operation in [
        ".append(true)",
        ".truncate(true)",
        "file.write_all(b\"wizard overwrite probe\")",
    ] {
        assert!(existing_write.contains(operation));
    }
    let descendant = &implementation[descendant_start..credential_open];
    assert!(descendant.contains("Command::new(executable)"));
    assert!(descendant.contains(".arg(PRIVATE_CREDENTIAL_PROBE_ARGUMENT)"));
    assert!(descendant.contains(".arg(target)"));
    assert!(descendant.contains(".env_clear()"));
    assert!(descendant.contains(".status()"));

    let preflight = implementation
        .find("fn require_worker_filesystem_policy")
        .unwrap();
    let arguments = implementation[preflight..]
        .find("fn worker_read_probe_arguments")
        .unwrap()
        + preflight;
    let body = &implementation[preflight..arguments];
    assert!(body.contains("scratch.codex_home().join(\"auth.json\")"));
    assert!(body.contains("require_worker_probe_positive_controls(scratch)?"));
    assert!(body.contains("for (probe_target, decision) in worker_read_probe_plan(policy)"));
    let write_loop = "for probe_target in &policy.denied_write_probes";
    let write_loop = body.find(write_loop).unwrap() + write_loop.len();
    assert!(body[write_loop..].starts_with(" {"));
    for required in [
        "PRIVATE_WRITE_PROBE_ARGUMENT",
        "PRIVATE_WRITE_APPEND_PROBE_ARGUMENT",
        "PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT",
        "PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT",
        "PRIVATE_NETWORK_PROBE_ARGUMENT",
        "PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT",
        "PRIVATE_DESCENDANT_PROBE_ARGUMENT",
        "for probe_argument in [",
        "WorkerReadDecision::Deny => summary.denied_reads += 1",
        "WorkerReadDecision::Allow => summary.allowed_reads += 1",
        "summary.denied_write_creates += 1",
        "summary.denied_write_mutations += 1",
        "summary.denied_network_listeners += 1",
        "summary.denied_network_connections += 1",
        "summary.denied_descendants += 1",
        "summary.allowed_reads == policy.readable_probes.len()",
        "summary.denied_reads == policy.deny_probes().len() + policy.denied_probes.len()",
        "summary.denied_write_creates == policy.denied_write_probes.len()",
        "summary.denied_write_mutations == 3",
        "summary.denied_network_listeners == 1",
        "summary.denied_network_connections == 1",
        "summary.require_complete()?",
    ] {
        assert!(body.contains(required));
    }
    assert_eq!(
        body.match_indices("require_worker_probe_execution(")
            .count(),
        6
    );
    assert_eq!(
        body.match_indices("require_existing_write_probe_unchanged(")
            .count(),
        2
    );
    let executor_start = implementation
        .find("fn require_worker_probe_execution(")
        .unwrap();
    let executor_end = implementation[executor_start..]
        .find("fn require_worker_read_probe_result(")
        .unwrap()
        + executor_start;
    let executor = &implementation[executor_start..executor_end];
    let projection_checks = executor
        .match_indices("require_private_credential_projection")
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    assert_eq!(projection_checks.len(), 2);
    let executions = executor
        .match_indices("output_with_limit")
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    assert_eq!(executions.len(), 1);
    assert!(projection_checks[0] < executions[0] && executions[0] < projection_checks[1]);
    assert_eq!(body.match_indices("identify_codex_runtime").count(), 2);
    assert!(body.contains("runtime_before == tools.codex_runtime"));
}

#[test]
fn worker_probe_positive_controls_exercise_each_operation_and_clean_up() {
    let (outer, scratch) = test_owned_scratch();
    let controls = scratch.scan_root.join("worker-probe-positive-controls");
    require_worker_probe_positive_controls(&scratch).unwrap();
    assert!(!controls.exists());
    require_worker_canaries(&scratch).unwrap();
    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[cfg(windows)]
#[test]
fn credential_probe_command_uses_the_exact_owned_worker_sandbox() {
    let repository = std::env::temp_dir().join("credential-probe-repository");
    let fixture = repository.join("xtask.exe");
    let live_auth = repository.join("live-home").join("auth.json");
    let projected_auth = repository.join("private-home").join("auth.json");
    let shell_environment = "shell_environment_policy.set={}";
    let policy = test_worker_policy(&repository, &live_auth, &projected_auth);
    let worker_permission = effective_worker_named_permission_profile(&policy).unwrap();
    let mut expected = vec![
        OsString::from("--no-daemon"),
        OsString::from("--strict-config"),
        OsString::from("--cd"),
        policy.probe_directory.clone().into_os_string(),
        OsString::from("sandbox"),
        OsString::from("--config"),
        OsString::from("default_permissions=\"codex_security_deep_scan_worker\""),
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
        fixture.clone().into_os_string(),
        OsString::from(PRIVATE_CREDENTIAL_PROBE_ARGUMENT),
        live_auth.clone().into_os_string(),
    ];
    assert_eq!(
        worker_read_probe_arguments(&policy, &fixture, &live_auth, shell_environment).unwrap(),
        expected
    );
    assert!(!expected.iter().any(|argument| {
        argument == "--sandbox-state-json" || argument == "--sandbox-state-disable-network"
    }));
    *expected.last_mut().unwrap() = projected_auth.clone().into_os_string();
    assert_eq!(
        worker_read_probe_arguments(&policy, &fixture, &projected_auth, shell_environment).unwrap(),
        expected
    );
    let prefix = expected[..expected.len() - 2].to_vec();
    let mut write = prefix.clone();
    write.extend([
        OsString::from(PRIVATE_WRITE_PROBE_ARGUMENT),
        policy.denied_write_probes[0].clone().into_os_string(),
    ]);
    assert_eq!(
        worker_private_probe_arguments(
            &policy,
            &fixture,
            PRIVATE_WRITE_PROBE_ARGUMENT,
            Some(policy.denied_write_probes[0].as_os_str()),
            shell_environment,
        )
        .unwrap(),
        write
    );
    for probe_argument in [
        PRIVATE_WRITE_APPEND_PROBE_ARGUMENT,
        PRIVATE_WRITE_TRUNCATE_PROBE_ARGUMENT,
        PRIVATE_WRITE_OVERWRITE_PROBE_ARGUMENT,
    ] {
        let mut existing_write = prefix.clone();
        existing_write.extend([
            OsString::from(probe_argument),
            policy.existing_write_probe.clone().into_os_string(),
        ]);
        assert_eq!(
            worker_private_probe_arguments(
                &policy,
                &fixture,
                probe_argument,
                Some(policy.existing_write_probe.as_os_str()),
                shell_environment,
            )
            .unwrap(),
            existing_write
        );
    }
    let mut network = prefix.clone();
    network.push(OsString::from(PRIVATE_NETWORK_PROBE_ARGUMENT));
    assert_eq!(
        worker_private_probe_arguments(
            &policy,
            &fixture,
            PRIVATE_NETWORK_PROBE_ARGUMENT,
            None,
            shell_environment,
        )
        .unwrap(),
        network
    );
    let endpoint = OsStr::new("127.0.0.1:47321");
    let mut connect = prefix.clone();
    connect.extend([
        OsString::from(PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT),
        endpoint.to_owned(),
    ]);
    assert_eq!(
        worker_private_probe_arguments(
            &policy,
            &fixture,
            PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT,
            Some(endpoint),
            shell_environment,
        )
        .unwrap(),
        connect
    );
    for rejected in ["127.0.0.1:0", "8.8.8.8:53", "[::1]:47321"] {
        assert!(
            worker_private_probe_arguments(
                &policy,
                &fixture,
                PRIVATE_NETWORK_CONNECT_PROBE_ARGUMENT,
                Some(OsStr::new(rejected)),
                shell_environment,
            )
            .is_err()
        );
    }
    let mut descendant = prefix;
    descendant.extend([
        OsString::from(PRIVATE_DESCENDANT_PROBE_ARGUMENT),
        policy.denied_canary.clone().into_os_string(),
    ]);
    assert_eq!(
        worker_private_probe_arguments(
            &policy,
            &fixture,
            PRIVATE_DESCENDANT_PROBE_ARGUMENT,
            Some(policy.denied_canary.as_os_str()),
            shell_environment,
        )
        .unwrap(),
        descendant
    );
    assert!(
        worker_private_probe_arguments(
            &policy,
            &fixture,
            PRIVATE_NETWORK_PROBE_ARGUMENT,
            Some(live_auth.as_os_str()),
            shell_environment,
        )
        .is_err()
    );
}

#[test]
fn managed_worker_policy_is_clone_backed_narrow_and_single_source() {
    let implementation = concat!(
        include_str!("../../security_deep.rs"),
        include_str!("../wrapper/overrides.rs"),
        include_str!("../toolchain.rs"),
        include_str!("../probe.rs"),
        include_str!("../completion.rs")
    );
    let clone_start = implementation
        .find("fn prepare_owned_worker_repository(")
        .unwrap();
    let clone_end = implementation[clone_start..].find("fn run_owned(").unwrap() + clone_start;
    let clone = &implementation[clone_start..clone_end];
    for required in [
        "\"--no-checkout\"",
        "\"--no-hardlinks\"",
        "copy_clean_tracked_to(paths.root(), &destination)?",
        "vec![\"remote\", \"remove\", \"origin\"]",
        "qualification::source_identity(&git_destination)? == *source",
    ] {
        assert!(
            clone.contains(required),
            "missing owned-clone guard {required}"
        );
    }

    let policy_start = implementation.find("impl ManagedWorkerPolicy").unwrap();
    let policy_end = implementation[policy_start..]
        .find("#[derive(Debug)]\nstruct PrivateCodexCredential")
        .unwrap()
        + policy_start;
    let policy = &implementation[policy_start..policy_end];
    for required in [
        "repository.parent() == Some(scratch.repository_parent.as_path())",
        "canonical_directory(&scratch.plugin",
        "canonical_directory(&scratch.scan_root",
        "canonical_probe_path(&tools.node.path",
        "python_worker_policy_entries(&tools.python_runtime)",
        "canonical_directory(&tools.codex_runtime.root",
        "vec![repository.join(\".git\")]",
        "!readable_entries.contains(&scratch.root)",
    ] {
        assert!(
            policy.contains(required),
            "missing worker-policy guard {required}"
        );
    }
    assert!(!policy.contains("git_root"));
    assert!(!policy.contains("tools.node.path.parent()"));
    assert!(policy.contains("&self.denied_canary"));
    let python_probe_start = implementation.find("fn first_python_policy_file(").unwrap();
    let python_probe_end = implementation[python_probe_start..]
        .find("fn require_worker_canaries(")
        .unwrap()
        + python_probe_start;
    let python_probe = &implementation[python_probe_start..python_probe_end];
    assert!(
        python_probe.contains("&& let Some(probe) = first_python_policy_file(&path, depth + 1)?")
    );
    assert!(!python_probe.contains("if let Ok(probe)"));

    let state_start = implementation
        .find("fn managed_read_only_sandbox_state(")
        .unwrap();
    let state_end = implementation[state_start..]
        .find("fn call_metadata(")
        .unwrap()
        + state_start;
    let state = &implementation[state_start..state_end];
    let root_deny = state.find("(\"access\", text_value(\"deny\"))").unwrap();
    let root_kind = state
        .find("object([(\"kind\", text_value(\"root\"))])")
        .unwrap();
    let minimal_kind = state
        .find("object([(\"kind\", text_value(\"minimal\"))])")
        .unwrap();
    let dynamic_reads = state.find("policy.readable_entries.iter().map").unwrap();
    let dynamic_denies = state.find("policy.denied_entries.iter().map").unwrap();
    let credential_denies = state
        .find("[&policy.live_auth, &policy.projected_auth]")
        .unwrap();
    assert!(
        root_deny < root_kind
            && root_kind < minimal_kind
            && minimal_kind < dynamic_reads
            && dynamic_reads < dynamic_denies
            && dynamic_denies < credential_denies
    );
    assert!(!state.contains("path_value(&policy.denied_canary)"));
    assert_eq!(
        state
            .matches("(\"network\", text_value(\"restricted\")),")
            .count(),
        2
    );

    let wrapper_start = implementation
        .find("fn inject_wrapper_overrides_for_scan_root(")
        .unwrap();
    let wrapper_end = implementation[wrapper_start..]
        .find("fn validate_app_server_arguments(")
        .unwrap()
        + wrapper_start;
    let wrapper = &implementation[wrapper_start..wrapper_end];
    assert!(wrapper.contains("if context.is_some()"));
    assert!(wrapper.contains("argument == provider_permission"));
    assert!(wrapper.contains("OsString::from(worker_permission)"));
    assert!(wrapper.contains("rewritten_permissions == 1"));
    assert!(wrapper.contains("app-server permission profile was altered"));

    let run_start = implementation.find("fn run_owned(").unwrap();
    let run_end = implementation[run_start..]
        .find("fn finalize_owned_transaction")
        .unwrap()
        + run_start;
    let run = &implementation[run_start..run_end];
    let prepare = run.find("prepare_owned_worker_repository").unwrap();
    let qualify = run
        .find("ManagedWorkerPolicy::qualify(\n            &worker_repository,")
        .unwrap();
    let preflight = run.find("require_worker_filesystem_policy").unwrap();
    let mcp = run.find("run_mcp(OwnedMcpInvocation").unwrap();
    let post_identity = run
        .find("qualification::source_identity(&worker_git_repository)? == source")
        .unwrap();
    let post_canaries = run.find("require_worker_canaries(&scratch)?").unwrap();
    assert!(prepare < qualify && qualify < preflight && preflight < mcp);
    assert!(mcp < post_identity && post_identity < post_canaries);
    assert!(run.contains("worker_policy: &worker_policy"));
    assert!(run.contains("&worker_probes"));

    let preflight_start = implementation
        .find("fn require_worker_filesystem_policy(")
        .unwrap();
    let preflight_end = implementation[preflight_start..]
        .find("fn worker_read_probe_arguments(")
        .unwrap()
        + preflight_start;
    let preflight = &implementation[preflight_start..preflight_end];
    let canary_checks = preflight
        .match_indices("require_worker_canaries(scratch)?")
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    let probe_loop = preflight
        .find("for (probe_target, decision) in worker_read_probe_plan(policy)")
        .unwrap();
    assert_eq!(canary_checks.len(), 2);
    assert!(canary_checks[0] < probe_loop && probe_loop < canary_checks[1]);
}

#[cfg(windows)]
#[test]
fn managed_worker_policy_is_an_executable_default_deny_allowlist() {
    let (outer, scratch) = test_owned_scratch();
    let repository = crate::mutate::workspace::prepare_real_child(
        &scratch.repository_parent,
        &scratch.repository_parent.join("wizards-bedrock"),
        "test owned worker repository",
    )
    .unwrap();
    std::fs::write(repository.join("README.md"), b"repository").unwrap();
    std::fs::create_dir(repository.join(".git")).unwrap();
    std::fs::write(
        repository.join(".git").join("HEAD"),
        b"ref: refs/heads/main\n",
    )
    .unwrap();
    std::fs::write(scratch.plugin.join("integrity.json"), b"{}").unwrap();

    let node_root = outer.join("node");
    let python_root = outer.join("python");
    let codex_root = outer.join("codex");
    let system_root = outer.join("windows");
    let live_root = outer.join("live");
    let wrong_projection_root = outer.join("wrong-projection");
    let fixture_directories = [
        node_root.clone(),
        python_root.clone(),
        codex_root.clone(),
        system_root.join("System32"),
        live_root.clone(),
        wrong_projection_root.clone(),
        python_root.join("DLLs"),
        python_root.join("Lib"),
        python_root.join("DLLs").join("__pycache__"),
        python_root.join("Lib").join("site-packages"),
    ];
    for directory in &fixture_directories {
        std::fs::create_dir_all(directory).unwrap();
    }
    let node = node_root.join("node.exe");
    let python = python_root.join("python.exe");
    let codex = codex_root.join("codex.exe");
    let command_processor = system_root.join("System32").join("cmd.exe");
    let live_auth = live_root.join("auth.json");
    let projected_auth = scratch.codex_home().join("auth.json");
    let wrong_projected_auth = wrong_projection_root.join("auth.json");
    let fixture_files = [
        node.clone(),
        python.clone(),
        python_root.join("python3.dll"),
        python_root.join("python312.dll"),
        python_root.join("vcruntime140.dll"),
        python_root.join("DLLs").join("fixture.pyd"),
        python_root.join("Lib").join("fixture.py"),
        codex.clone(),
        command_processor.clone(),
        live_auth.clone(),
        projected_auth.clone(),
        wrong_projected_auth.clone(),
    ];
    for path in &fixture_files {
        std::fs::write(path, b"fixture").unwrap();
    }
    std::fs::write(
        python_root
            .join("DLLs")
            .join("__pycache__")
            .join("ignored.pyc"),
        b"ignored",
    )
    .unwrap();
    std::fs::write(
        python_root
            .join("Lib")
            .join("site-packages")
            .join("ignored.py"),
        b"ignored",
    )
    .unwrap();

    let tools = Toolchain {
        node: ToolIdentity {
            path: node.clone(),
            bytes: 1,
            sha256: "a".repeat(64),
            version: "fixture".to_string(),
        },
        python: ToolIdentity {
            path: python.clone(),
            bytes: 1,
            sha256: "b".repeat(64),
            version: "fixture".to_string(),
        },
        python_runtime: PythonRuntimeIdentity {
            root: python_root.clone(),
            entries: 1,
            bytes: 1,
            sha256: "c".repeat(64),
        },
        codex: ToolIdentity {
            path: codex.clone(),
            bytes: 1,
            sha256: "d".repeat(64),
            version: "fixture".to_string(),
        },
        codex_runtime: CodexRuntimeIdentity {
            root: codex_root.clone(),
            entries: 1,
            bytes: 1,
            sha256: "e".repeat(64),
        },
    };
    let shell = ShellEnvironmentPolicy {
        system_root,
        command_processor: command_processor.clone(),
        search_path: OsString::new().to_owned(),
        temporary: scratch.temporary.clone(),
    };
    let policy = ManagedWorkerPolicy::qualify(
        &repository,
        &scratch,
        &live_auth,
        &projected_auth,
        &tools,
        &shell,
    )
    .unwrap();

    let expected_reads = vec![
        repository.clone(),
        scratch.plugin.clone(),
        scratch.scan_root.clone(),
        node.clone(),
        codex_root.clone(),
        python.clone(),
        python_root.join("python3.dll"),
        python_root.join("python312.dll"),
        python_root.join("vcruntime140.dll"),
        python_root.join("DLLs"),
        python_root.join("Lib"),
        std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap(),
    ];
    for expected in &expected_reads {
        assert!(
            policy.readable_entries.contains(expected),
            "missing exact worker read {}",
            expected.display()
        );
    }
    assert!(!policy.readable_entries.contains(&node_root));
    assert!(!policy.readable_entries.contains(&python_root));
    let expected_denied = [
        repository.join(".git"),
        python_root.join("DLLs").join("__pycache__"),
        python_root.join("Lib").join("site-packages"),
    ];
    for expected in &expected_denied {
        assert!(policy.denied_entries.contains(expected));
    }
    assert!(
        !policy
            .denied_entries
            .contains(&scratch.worker_denied_canary)
    );
    let plan = worker_read_probe_plan(&policy);
    let expected_deny_probes = [
        live_auth.clone(),
        projected_auth.clone(),
        scratch.worker_denied_canary.clone(),
        repository.join(".git").join("HEAD"),
        python_root
            .join("DLLs")
            .join("__pycache__")
            .join("ignored.pyc"),
        python_root
            .join("Lib")
            .join("site-packages")
            .join("ignored.py"),
    ];
    assert_eq!(
        plan.len(),
        expected_deny_probes.len() + policy.readable_probes.len()
    );
    for ((path, decision), expected) in plan.iter().zip(&expected_deny_probes) {
        assert_eq!(*path, expected);
        assert_eq!(*decision, WorkerReadDecision::Deny);
    }
    assert!(
        plan[expected_deny_probes.len()..]
            .iter()
            .all(|(_, decision)| *decision == WorkerReadDecision::Allow)
    );
    assert_eq!(
        policy.denied_write_probes,
        [
            scratch.scan_root.join(WORKER_SCAN_WRITE_CANARY_NAME),
            scratch.root.join(WORKER_OUTSIDE_WRITE_CANARY_NAME),
        ]
    );
    assert!(policy.denied_write_probes.iter().all(|path| !path.exists()));
    assert_eq!(policy.existing_write_probe, scratch.worker_allowed_canary);
    assert!(
        policy
            .readable_probes
            .contains(&policy.existing_write_probe)
    );
    assert!(policy.existing_write_probe.is_file());
    let state = render_json(&managed_read_only_sandbox_state(&policy).unwrap()).unwrap();
    assert!(state.contains("\"kind\":\"root\""));
    assert!(state.contains("\"kind\":\"minimal\""));
    assert!(!state.contains(WORKER_DENIED_CANARY_NAME));

    assert!(
        ManagedWorkerPolicy::qualify(
            &repository,
            &scratch,
            &live_auth,
            &wrong_projected_auth,
            &tools,
            &shell,
        )
        .is_err()
    );
    let foreign_repository = outer.join("foreign-repository");
    std::fs::create_dir(&foreign_repository).unwrap();
    std::fs::write(foreign_repository.join("README.md"), b"foreign").unwrap();
    std::fs::create_dir(foreign_repository.join(".git")).unwrap();
    std::fs::write(
        foreign_repository.join(".git").join("HEAD"),
        b"ref: refs/heads/main\n",
    )
    .unwrap();
    assert!(
        ManagedWorkerPolicy::qualify(
            &foreign_repository,
            &scratch,
            &live_auth,
            &projected_auth,
            &tools,
            &shell,
        )
        .is_err()
    );
    let overlapping_auth = repository.join("auth.json");
    std::fs::write(&overlapping_auth, b"overlap").unwrap();
    assert!(
        ManagedWorkerPolicy::qualify(
            &repository,
            &scratch,
            &overlapping_auth,
            &projected_auth,
            &tools,
            &shell,
        )
        .is_err()
    );

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn worker_canaries_are_owned_and_tamper_evident() {
    let (outer, scratch) = test_owned_scratch();
    require_worker_canaries(&scratch).unwrap();
    std::fs::write(&scratch.worker_denied_canary, b"tampered denied canary").unwrap();
    assert!(require_worker_canaries(&scratch).is_err());
    std::fs::write(&scratch.worker_denied_canary, WORKER_DENIED_CANARY).unwrap();
    std::fs::write(&scratch.worker_allowed_canary, b"tampered allowed canary").unwrap();
    assert!(require_worker_canaries(&scratch).is_err());

    let metadata = std::fs::symlink_metadata(&outer).unwrap();
    remove_entry_no_follow(&outer, &metadata).unwrap();
}

#[test]
fn python_probe_walk_propagates_deep_errors() {
    let root = unique_test_directory("wizard-security-deep-python-probe-test");
    let mut nested = root.join("a");
    for _ in 0..=MAX_PYTHON_RUNTIME_DEPTH {
        std::fs::create_dir(&nested).unwrap();
        nested = nested.join("a");
    }
    std::fs::write(root.join("z.py"), b"reachable only after swallowed error").unwrap();
    let error = first_python_policy_file(&root, 0).unwrap_err().to_string();
    assert!(error.contains("depth limit"), "{error}");

    let metadata = std::fs::symlink_metadata(&root).unwrap();
    remove_entry_no_follow(&root, &metadata).unwrap();
}

#[test]
fn managed_sandbox_cwd_is_an_exact_windows_file_url() {
    assert_eq!(
        windows_file_url(Path::new(r"C:\repo path\hash#percent%")).unwrap(),
        "file:///C:/repo%20path/hash%23percent%25"
    );
    assert_eq!(
        windows_file_url(Path::new(r"\\?\C:\repo")).unwrap(),
        "file:///C:/repo"
    );
    assert_eq!(
        windows_file_url(Path::new(r"\\server\share\repo")).unwrap(),
        "file://server/share/repo"
    );
    assert_eq!(
        windows_file_url(Path::new(r"\\?\UNC\server\share\repo")).unwrap(),
        "file://server/share/repo"
    );
    assert!(windows_file_url(Path::new("relative")).is_err());
}
