#[cfg(windows)]
mod config;
mod fake_app_server;
mod owned;
mod protocol;
mod sandbox;
mod toolchain;
mod transitions;
mod verify;
mod wrapper;

use super::*;

const TEST_SCAN_ID: &str = "11111111-1111-4111-8111-111111111111";
static SCRATCH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn source() -> SourceIdentity {
    SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    }
}

fn owned_tools() -> Toolchain {
    Toolchain {
        node: ToolIdentity {
            path: PathBuf::from("node"),
            bytes: NODE_BYTES,
            sha256: NODE_SHA256.to_string(),
            version: NODE_VERSION.to_string(),
        },
        python: ToolIdentity {
            path: PathBuf::from("python"),
            bytes: PYTHON_BYTES,
            sha256: PYTHON_SHA256.to_string(),
            version: PYTHON_VERSION.to_string(),
        },
        python_runtime: PythonRuntimeIdentity {
            root: PathBuf::from("python-runtime"),
            entries: 1,
            bytes: 1,
            sha256: "0".repeat(64),
        },
        codex: ToolIdentity {
            path: PathBuf::from("codex"),
            bytes: CODEX_BYTES,
            sha256: CODEX_SHA256.to_string(),
            version: CODEX_VERSION.to_string(),
        },
        codex_runtime: qualified_codex_runtime_identity(),
    }
}

fn owned_git() -> GitIdentity {
    GitIdentity {
        path: PathBuf::from(r"C:\Program Files\Git\cmd\git.exe"),
        bytes: 1,
        sha256: "e".repeat(64),
        version: "git version 2.51.0.windows.1".to_string(),
    }
}

fn owned_package() -> PluginPackage {
    PluginPackage {
        root: PathBuf::from("plugin"),
        integrity: vec![b'x'; PLUGIN_INTEGRITY_BYTES as usize],
        files: Vec::new(),
    }
}

fn owned_permission_digests() -> WorkerPermissionDigests {
    WorkerPermissionDigests {
        parent_state: "1".repeat(64),
        provider_profile: "2".repeat(64),
        effective_profile: "3".repeat(64),
        effective_state: "4".repeat(64),
    }
}

fn owned_probe_summary() -> WorkerProbeSummary {
    WorkerProbeSummary {
        allowed_reads: 7,
        denied_reads: 4,
        denied_write_creates: 2,
        denied_write_mutations: 3,
        denied_network_listeners: 1,
        denied_network_connections: 1,
        denied_descendants: 1,
    }
}

fn mapping_probe_summary() -> WorkerProbeSummary {
    WorkerProbeSummary {
        allowed_reads: 11,
        denied_reads: 13,
        denied_write_creates: 17,
        denied_write_mutations: 19,
        denied_network_listeners: 23,
        denied_network_connections: 29,
        denied_descendants: 31,
    }
}

fn owned_transition_ledger() -> WorkerTransitionLedger {
    WorkerTransitionLedger {
        bytes: Vec::new(),
        app_server: 1,
        exec_initial: 2,
        exec_resumed: 3,
        exec_exited_zero: 4,
        thread_ids: BTreeSet::from(["thread-1".to_string()]),
        execution_threads: BTreeSet::from(["thread-1".to_string()]),
        probes: owned_probe_summary(),
    }
}

fn test_worker_transition_ledger(
    source: &SourceIdentity,
    permissions: &WorkerPermissionDigests,
) -> WorkerTransitionLedger {
    let receipt =
        |kind, stage, process_id, execution_thread: Option<&str>| WorkerTransitionReceipt {
            kind,
            stage,
            process_id,
            source_commit: source.commit.clone(),
            provider_profile: permissions.provider_profile.clone(),
            effective_profile: permissions.effective_profile.clone(),
            applied_profile: kind
                .applied_digest(
                    &permissions.provider_profile,
                    &permissions.effective_profile,
                )
                .to_string(),
            worker: "a".repeat(64),
            execution_thread: execution_thread.map(str::to_string),
            arguments: format!("{process_id:064x}"),
        };
    let thread_ids = BTreeSet::from(["thread-1".to_string()]);
    render_worker_transition_ledger(
        &[
            receipt(
                WorkerTransitionKind::AppServerPassThrough,
                WorkerTransitionStage::Launched,
                1,
                None,
            ),
            receipt(
                WorkerTransitionKind::AppServerPassThrough,
                WorkerTransitionStage::Launched,
                4,
                None,
            ),
            receipt(
                WorkerTransitionKind::ExecInitialAttenuated,
                WorkerTransitionStage::Started,
                2,
                Some("thread-1"),
            ),
            receipt(
                WorkerTransitionKind::ExecResumedAttenuated,
                WorkerTransitionStage::Started,
                3,
                Some("thread-1"),
            ),
        ],
        &thread_ids,
        &thread_ids,
        source,
        permissions,
        &owned_probe_summary(),
    )
    .unwrap()
}

fn unique_test_directory(prefix: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let parent = canonical_directory(
        &std::env::temp_dir(),
        "test Codex Security temporary parent",
    )
    .unwrap();
    for attempt in 0..128_u64 {
        let candidate = parent.join(format!(
            "{prefix}-{}-{nonce}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            attempt
        ));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return std::fs::canonicalize(candidate).unwrap(),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("creating unique test directory failed: {error}"),
        }
    }
    panic!("could not reserve a unique test directory")
}

fn private_test_root() -> (PathBuf, PathBuf) {
    let outer = unique_test_directory("wsdp");
    let root = outer.join(OWNED_SCRATCH_NAME);
    std::fs::create_dir(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    (outer, root)
}

fn test_owned_scratch() -> (PathBuf, OwnedScratch) {
    let (outer, root) = private_test_root();
    let child = |name: &str, label: &str| {
        crate::mutate::workspace::prepare_real_child(&root, &root.join(name), label).unwrap()
    };
    child("codex-home", "test private Codex home");
    let repository_parent = child("repository", "test worker repository parent");
    let plugin = child("plugin", "test worker plugin");
    let scan_root = child("scans", "test worker scan root");
    let worker_allowed_canary = scan_root.join(WORKER_ALLOWED_CANARY_NAME);
    let worker_denied_canary = root.join(WORKER_DENIED_CANARY_NAME);
    write_new_file(&worker_allowed_canary, WORKER_ALLOWED_CANARY).unwrap();
    write_new_file(&worker_denied_canary, WORKER_DENIED_CANARY).unwrap();
    let scratch = OwnedScratch {
        repository_parent,
        plugin,
        scan_root,
        state: child("state", "test worker state"),
        temporary: child("temp", "test worker temporary directory"),
        pycache: child("pycache", "test worker Python cache"),
        codex_state: child("codex-state", "test worker Codex state"),
        transitions: child("transitions", "test worker transition directory"),
        worker_allowed_canary,
        worker_denied_canary,
        root,
    };
    (outer, scratch)
}

fn fixture_git(root: &Path, arguments: &[&str]) -> std::process::Output {
    let git = crate::git::resolved().unwrap();
    let mut command = Command::new(git.path());
    crate::bounded_process::isolate_git_environment(&mut command);
    command.current_dir(root).args(arguments);
    crate::bounded_process::output_with_limit(
        command,
        Duration::from_secs(10),
        "Codex Security Git fixture",
        64 * 1024,
    )
    .unwrap()
}

fn require_fixture_git(root: &Path, arguments: &[&str]) -> std::process::Output {
    let output = fixture_git(root, arguments);
    assert!(
        output.status.success(),
        "git {arguments:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn ambient_config() -> EffectiveConfig {
    EffectiveConfig {
        model: None,
        model_provider: None,
        openai_base_url: None,
        chatgpt_base_url: None,
        reasoning: None,
        service_tier: None,
        notify_is_empty: true,
        memories_generate: None,
        memories_use: None,
        feature_memories: None,
        windows_sandbox: None,
        mcp_servers: BTreeMap::from([(
            "node_repl".to_string(),
            McpRoute {
                enabled: true,
                command: Some("ambient-command".to_string()),
                args: Some(Vec::new()),
                fields: ["args", "command", "enabled", "env"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                env_keys: ["AMBIENT_KEY".to_string()].into_iter().collect(),
                env_values_empty: false,
                env_vars_empty: false,
                environment_id: Some("ambient".to_string()),
                startup_timeout_seconds: Some(5),
                tool_timeout_seconds: None,
            },
        )]),
        plugins: BTreeMap::from([("browser@openai-bundled".to_string(), true)]),
        model_providers: BTreeSet::new(),
        origins: BTreeSet::new(),
    }
}

fn safe_config(inert_mcp: &Path) -> EffectiveConfig {
    let mut origins: BTreeSet<String> = [
        "model_provider",
        "openai_base_url",
        "chatgpt_base_url",
        "model",
        "model_reasoning_effort",
        "service_tier",
        "notify",
        "memories.generate_memories",
        "memories.use_memories",
        "features.memories",
        "windows.sandbox",
        "mcp_servers.node_repl.enabled",
        "mcp_servers.node_repl.env.AMBIENT_KEY",
        "mcp_servers.node_repl.environment_id",
        "mcp_servers.node_repl.startup_timeout_sec",
        "mcp_servers.node_repl.tool_timeout_sec",
        "plugins.browser@openai-bundled.enabled",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    origins.insert("mcp_servers.node_repl.command".to_string());
    EffectiveConfig {
        model: Some(MODEL.to_string()),
        model_provider: Some("openai".to_string()),
        openai_base_url: Some(OFFICIAL_CHATGPT_BASE_URL.to_string()),
        chatgpt_base_url: Some(OFFICIAL_CHATGPT_BASE_URL.to_string()),
        reasoning: Some(REASONING.to_string()),
        service_tier: Some("default".to_string()),
        notify_is_empty: true,
        memories_generate: Some(false),
        memories_use: Some(false),
        feature_memories: Some(false),
        windows_sandbox: Some("mxc".to_string()),
        mcp_servers: BTreeMap::from([(
            "node_repl".to_string(),
            McpRoute {
                enabled: false,
                command: Some(inert_mcp.to_string_lossy().into_owned()),
                args: Some(Vec::new()),
                fields: [
                    "args",
                    "command",
                    "enabled",
                    "env",
                    "environment_id",
                    "startup_timeout_sec",
                    "tool_timeout_sec",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
                env_keys: ["AMBIENT_KEY".to_string()].into_iter().collect(),
                env_values_empty: true,
                env_vars_empty: true,
                environment_id: Some(INERT_MCP_ENVIRONMENT.to_string()),
                startup_timeout_seconds: Some(INERT_MCP_TIMEOUT_SECONDS),
                tool_timeout_seconds: Some(INERT_MCP_TIMEOUT_SECONDS),
            },
        )]),
        plugins: BTreeMap::from([("browser@openai-bundled".to_string(), false)]),
        model_providers: BTreeSet::new(),
        origins,
    }
}

fn rebind_owned_request(text: &str) -> Vec<u8> {
    let offset = text.rfind("checksum.sha256=").unwrap();
    let body = &text[..offset];
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    format!("{body}checksum.sha256={checksum}\n").into_bytes()
}

fn rebind_worker_transition_ledger(text: &str) -> Vec<u8> {
    let offset = text.rfind("checksum.sha256=").unwrap();
    let body = &text[..offset];
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    format!("{body}checksum.sha256={checksum}\n").into_bytes()
}

fn artifacts(
    finding_level: Option<&str>,
    completeness: &str,
    revision: &str,
) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    let finding = finding_level.map_or(String::new(), |level| format!(r#"{{"findingId":"csf_000000000000000000000001","fingerprints":{{"algorithm":"codex-security/v1","primary":"codex-security/v1:sha256:{}"}},"severity":{{"level":"{level}"}}}}"#, "1".repeat(64)));
    let findings = format!(r#"{{"documentType":"codex-security.findings","schemaVersion":"1.0","scanId":"{TEST_SCAN_ID}","findings":[{finding}]}}"#).into_bytes();
    let coverage = format!(r#"{{"documentType":"codex-security.coverage","schemaVersion":"1.0","scanId":"{TEST_SCAN_ID}","mode":"deep_repository","completeness":"{completeness}","inventoryStrategy":"repository","includePaths":["."],"excludePaths":[],"surfaces":[],"explicitExclusions":[],"deferred":[],"openQuestions":[]}}"#).into_bytes();
    let manifest = format!(r#"{{"documentType":"codex-security.scan-manifest","schemaVersion":"1.0","scan":{{"id":"{TEST_SCAN_ID}","producer":{{"name":"codex-security-plugin","version":"0.1.32"}},"status":"completed","startedAt":"2026-09-28T00:00:00Z","completedAt":"2026-09-28T01:00:00Z","sealedAt":"2026-09-28T01:00:00Z","target":{{"kind":"git_revision","targetId":"target-1","displayName":"wizards-bedrock","revision":"{revision}"}},"scope":{{"includePaths":["."],"excludePaths":[],"summary":"Review of the complete `wizards-bedrock` tree.","artifactsReviewed":["Rust *sources*","qualification harness"],"runtimeStatus":"Static and host-side validation completed.","validationMode":"independent | host","context":"Paths [owned] <local>","limitations":["No runtime boot; source-only."]}},"threatModel":{{"summary":"Assets in `kernel` and *host*.","assets":["Kernel [state]"],"trustBoundaries":["Host > guest boundary"],"attackerCapabilities":["Can supply repository inputs."],"securityObjectives":["Keep authority local."],"assumptions":["Pinned tools are trusted."],"scope":{{"includePaths":["."],"excludePaths":[],"summary":"Repository source."}},"origin":"reconciled"}},"preservedSources":{{"workers/discovery/checkpoint.json":"{}"}},"coverageRef":"coverage.json","findingsRef":"findings.json","artifacts":[{{"path":"findings.json","sha256":"{}","mediaType":"application/json"}},{{"path":"coverage.json","sha256":"{}","mediaType":"application/json"}}]}}}}"#, "2".repeat(64), sha256::hex(&sha256::digest(&findings)), sha256::hex(&sha256::digest(&coverage))).into_bytes();
    let report = format!(
        "# Security Review: wizards-bedrock\n\n## Scope\n\nReview of the complete `wizards-bedrock` tree.\n\n- Scan mode: deep_repository\n- Target kind: git_revision\n- Target ID: target-1\n- Revision: {revision}\n- Inventory strategy: repository\n- Included paths: .\n- Excluded paths: none\n- Artifacts reviewed: Rust \\*sources\\*, qualification harness\n- Scan context: Paths \\[owned\\] \\<local\\>\n\nLimitations and exclusions:\n- No runtime boot; source-only.\n\n### Scan Summary\n\n| Field | Value |\n| --- | --- |\n| Scan outcome | completed |\n| Reportable DSS findings | 0 |\n| Report instances | 0 |\n| Report severity mix | none |\n| Report confidence mix | none |\n| Coverage | complete |\n| Validation mode | independent \\| host |\n\nCanonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.\n\n## Threat Model\n\nAssets in `kernel` and \\*host\\*.\n\n### Assets\n\n- Kernel \\[state\\]\n\n### Trust Boundaries\n\n- Host \\> guest boundary\n\n### Attacker Capabilities\n\n- Can supply repository inputs.\n\n### Security Objectives\n\n- Keep authority local.\n\n### Assumptions\n\n- Pinned tools are trusted.\n\n## Findings\n\n### No findings\n\nNo reportable findings survived the canonical discovery, validation, and reportability gates.\n"
    )
    .into_bytes();
    (manifest, findings, coverage, report)
}
