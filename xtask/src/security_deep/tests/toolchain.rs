use super::*;

#[test]
fn chatgpt_login_probe_accepts_stderr_status_but_rejects_other_or_ambiguous_statuses() {
    require_chatgpt_login_output(b"", b"warning\nLogged in using ChatGPT\n").unwrap();
    assert!(require_chatgpt_login_output(b"", b"Logged in using an API key\n").is_err());
    assert!(
        require_chatgpt_login_output(
            b"Logged in using ChatGPT\n",
            b"Logged in using an API key\n"
        )
        .is_err()
    );
}

#[cfg(windows)]
#[test]
fn worker_shell_environment_uses_only_qualified_paths() {
    let root = unique_test_directory("wizard-security-deep-shell-environment");
    let temporary = root.join("temp");
    let mut tools = Vec::new();
    for name in ["node", "python", "codex", "git"] {
        let directory = root.join(name);
        std::fs::create_dir(&directory).unwrap();
        tools.push(directory.join(format!("{name}.exe")));
    }
    std::fs::create_dir(&temporary).unwrap();
    let policy = ShellEnvironmentPolicy::qualify_paths(
        &tools[0], &tools[1], &tools[2], &tools[3], &temporary,
    )
    .unwrap();
    let mut expected_path = tools
        .iter()
        .map(|tool| tool.parent().unwrap().to_path_buf())
        .collect::<Vec<_>>();
    expected_path.push(policy.system_root.join("System32"));
    assert_eq!(
        std::env::split_paths(&policy.search_path).collect::<Vec<_>>(),
        expected_path
    );
    assert_eq!(policy.temporary, temporary);
    assert_eq!(
        policy.command_processor,
        policy.system_root.join("System32").join("cmd.exe")
    );

    let mut command = Command::new("unused");
    policy.apply(&mut command);
    let environment = command
        .get_envs()
        .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
        .collect::<BTreeMap<_, _>>();
    for (name, expected) in [
        ("SYSTEMROOT", policy.system_root.as_os_str()),
        ("WINDIR", policy.system_root.as_os_str()),
        ("COMSPEC", policy.command_processor.as_os_str()),
        ("PATHEXT", OsStr::new(WINDOWS_PATH_EXTENSIONS)),
        ("PATH", policy.search_path.as_os_str()),
        ("TEMP", policy.temporary.as_os_str()),
        ("TMP", policy.temporary.as_os_str()),
    ] {
        assert_eq!(
            environment.get(OsStr::new(name)),
            Some(&Some(expected.to_owned()))
        );
    }
    assert!(
        policy
            .set_override()
            .unwrap()
            .contains(WINDOWS_PATH_EXTENSIONS)
    );

    let implementation = concat!(
        include_str!("../environment.rs"),
        include_str!("../../security_deep.rs"),
        include_str!("../probe.rs"),
        include_str!("../mcp.rs")
    );
    let owned_start = implementation.find("fn run_owned(").unwrap();
    let owned_end = implementation[owned_start..]
        .find("fn write_new_file(")
        .unwrap()
        + owned_start;
    let owned = &implementation[owned_start..owned_end];
    assert_eq!(
        owned
            .match_indices("ShellEnvironmentPolicy::qualify")
            .count(),
        1
    );
    assert_eq!(owned.match_indices("&shell_environment").count(), 3);
    for (start, end) in [
        (
            "fn require_worker_filesystem_policy(",
            "fn apply_base_environment(",
        ),
        ("fn run_mcp(", "fn exchange_mcp("),
    ] {
        let start = implementation.find(start).unwrap();
        let end = implementation[start..].find(end).unwrap() + start;
        assert!(implementation[start..end].contains("shell_environment.apply(&mut command)"));
    }
    let context_start = implementation.find("impl PrivateExecutionContext").unwrap();
    let context_end = implementation[context_start..]
        .find("impl ShellEnvironmentPolicy")
        .unwrap()
        + context_start;
    assert!(
        implementation[context_start..context_end]
            .contains("self.shell_environment.require_current()?")
    );

    let mut shell = Command::new(&policy.command_processor);
    shell.args(["/D", "/S", "/C", "exit /b 0"]).env_clear();
    policy.apply(&mut shell);
    assert!(shell.status().unwrap().success());

    let metadata = std::fs::symlink_metadata(&root).unwrap();
    remove_entry_no_follow(&root, &metadata).unwrap();
}

#[test]
fn codex_runtime_inventory_is_exact_and_complete() {
    assert_eq!(CODEX_RUNTIME_FILES.len(), 4);
    assert_eq!(
        CODEX_RUNTIME_FILES
            .iter()
            .map(|(_, bytes, _)| bytes)
            .sum::<u64>(),
        CODEX_RUNTIME_BYTES
    );
    assert_eq!(CODEX_RUNTIME_SHA256.len(), 64);
    let names = CODEX_RUNTIME_FILES
        .iter()
        .map(|(name, _, _)| *name)
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), CODEX_RUNTIME_FILES.len());
    assert!(names.contains("codex.exe"));
    let actual = names.iter().map(|name| (*name).to_string()).collect();
    require_codex_runtime_inventory(&actual, &names).unwrap();
    let mut added = actual.clone();
    added.insert("unqualified.exe".to_string());
    assert!(require_codex_runtime_inventory(&added, &names).is_err());
    let mut removed = actual;
    removed.remove("codex-command-runner.exe");
    assert!(require_codex_runtime_inventory(&removed, &names).is_err());

    for (name, bytes, digest) in CODEX_RUNTIME_FILES {
        require_sha256(digest, "qualified Codex runtime digest").unwrap();
        let mut decoded = [0u8; 32];
        for (index, pair) in digest.as_bytes().chunks_exact(2).enumerate() {
            decoded[index] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
        }
        require_codex_runtime_file_fact(name, bytes, &decoded, bytes, digest).unwrap();
        assert!(require_codex_runtime_file_fact(name, bytes + 1, &decoded, bytes, digest).is_err());
        decoded[0] ^= 1;
        assert!(require_codex_runtime_file_fact(name, bytes, &decoded, bytes, digest).is_err());
    }

    let exact = qualified_codex_runtime_identity();
    require_codex_runtime_identity(&exact).unwrap();
    let mut changed = exact.clone();
    changed.entries += 1;
    assert!(require_codex_runtime_identity(&changed).is_err());
    changed = exact.clone();
    changed.bytes += 1;
    assert!(require_codex_runtime_identity(&changed).is_err());
    changed = exact;
    changed.sha256 = "0".repeat(64);
    assert!(require_codex_runtime_identity(&changed).is_err());

    let implementation = include_str!("../toolchain.rs");
    let start = implementation.find("fn identify_codex_runtime").unwrap();
    let end = implementation[start..]
        .find("fn require_codex_runtime_inventory")
        .unwrap()
        + start;
    let body = &implementation[start..end];
    for folded_fact in [
        "folded.update(&(name.len() as u64).to_be_bytes())?",
        "folded.update(name.as_bytes())?",
        "folded.update(&length.to_be_bytes())?",
        "folded.update(&digest)?",
    ] {
        assert!(body.contains(folded_fact));
    }
}

#[test]
fn pinned_tool_inventory_rejects_added_removed_reordered_or_changed_tools() {
    let mut exact = pinned_mcp_tool_inventory_for_test();
    require_exact_mcp_tools(&exact).unwrap();
    exact.swap(0, 1);
    assert!(require_exact_mcp_tools(&exact).is_err());
    exact = pinned_mcp_tool_inventory_for_test();
    exact.pop();
    assert!(require_exact_mcp_tools(&exact).is_err());
    exact = pinned_mcp_tool_inventory_for_test();
    exact.push(object([("name", text_value("other"))]));
    assert!(require_exact_mcp_tools(&exact).is_err());
}
