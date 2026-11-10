use super::*;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum ProviderExecConfig {
    Approval,
    DefaultPermissions,
    PermissionProfile,
    Reasoning,
    CodexSecurityCommand,
    CodexSecurityEnabled,
    ArtifactCommand,
    ArtifactArgs,
    ArtifactRequired,
    ArtifactStartup,
    ArtifactTool,
    Agents,
    MultiAgentEnabled,
    MultiAgentMaximum,
    Optional(&'static str),
    ArtifactEnvironment(String, String),
}

pub(super) fn require_provider_exec_config(
    value: &str,
    plugin_mirror: &Path,
    artifact_node: &ToolIdentity,
    provider_permission: &str,
) -> Result<ProviderExecConfig> {
    ensure!(
        !value.is_empty() && !value.contains(['\r', '\n', '\0']),
        "private Codex exec config is malformed"
    );
    let (key, setting) = value
        .split_once('=')
        .context("private Codex exec config has no value")?;
    let exact = |expected: &str, kind| {
        ensure!(setting == expected, "private Codex exec {key} differs");
        Ok(kind)
    };
    match key {
        "approval_policy" => exact("\"never\"", ProviderExecConfig::Approval),
        "default_permissions" => exact(
            "\"codex_security_deep_scan_worker\"",
            ProviderExecConfig::DefaultPermissions,
        ),
        "permissions.codex_security_deep_scan_worker" => exact(
            provider_permission
                .strip_prefix(WORKER_NAMED_PERMISSION_PREFIX)
                .context("private Codex provider permission profile key differs")?,
            ProviderExecConfig::PermissionProfile,
        ),
        "model_reasoning_effort" => {
            exact(&quote_toml_string(REASONING), ProviderExecConfig::Reasoning)
        }
        "service_tier" => exact("\"default\"", ProviderExecConfig::Optional("service_tier")),
        "model_reasoning_summary" => {
            let summary = parse_toml_string(setting)?;
            ensure!(
                matches!(summary.as_str(), "auto" | "concise" | "detailed" | "none"),
                "private Codex exec reasoning summary differs"
            );
            Ok(ProviderExecConfig::Optional("model_reasoning_summary"))
        }
        "mcp_servers.codex-security.command" => {
            exact("\"node\"", ProviderExecConfig::CodexSecurityCommand)
        }
        "mcp_servers.codex-security.enabled" => {
            exact("false", ProviderExecConfig::CodexSecurityEnabled)
        }
        "mcp_servers.cs_artifacts.command" => {
            let command = PathBuf::from(parse_toml_string(setting)?);
            let canonical = std::fs::canonicalize(&command).with_context(|| {
                format!(
                    "canonicalizing private Codex artifact Node {}",
                    command.display()
                )
            })?;
            ensure!(
                canonical == command,
                "private Codex artifact Node is redirected"
            );
            let (bytes, digest) = measure_file(
                &canonical,
                MAX_EXECUTABLE_BYTES,
                "private Codex artifact Node",
            )?;
            ensure!(
                canonical == artifact_node.path
                    && bytes == artifact_node.bytes
                    && sha256::hex(&digest) == artifact_node.sha256,
                "private Codex artifact Node differs"
            );
            Ok(ProviderExecConfig::ArtifactCommand)
        }
        "mcp_servers.cs_artifacts.args" => {
            let server = plugin_mirror.join("mcp").join("server.mjs");
            let server = server
                .to_str()
                .context("private Codex artifact server path is not UTF-8")?;
            let expected = format!(
                "[{}, \"--artifact-writer\", \"--stdio\"]",
                quote_toml_string(server)
            );
            exact(&expected, ProviderExecConfig::ArtifactArgs)
        }
        "mcp_servers.cs_artifacts.required" => exact("true", ProviderExecConfig::ArtifactRequired),
        "mcp_servers.cs_artifacts.startup_timeout_sec" => {
            exact("180", ProviderExecConfig::ArtifactStartup)
        }
        "mcp_servers.cs_artifacts.tool_timeout_sec" => {
            exact("86400", ProviderExecConfig::ArtifactTool)
        }
        "agents.max_threads" => {
            let threads = setting
                .parse::<u64>()
                .context("private Codex exec agent count is not numeric")?;
            ensure!(
                threads == 3 && setting == threads.to_string(),
                "private Codex exec agent count differs"
            );
            Ok(ProviderExecConfig::Agents)
        }
        "features.multi_agent_v2.enabled" => exact("false", ProviderExecConfig::MultiAgentEnabled),
        "features.multi_agent_v2.max_concurrent_threads_per_session" => {
            exact("4", ProviderExecConfig::MultiAgentMaximum)
        }
        key if key.starts_with("mcp_servers.cs_artifacts.env.") => {
            let variable = key
                .strip_prefix("mcp_servers.cs_artifacts.env.")
                .context("private Codex artifact environment key differs")?;
            ensure!(
                matches!(
                    variable,
                    "CODEX_SECURITY_ARTIFACT_ROOT"
                        | "CODEX_SECURITY_REPO_ROOT"
                        | "CODEX_SECURITY_ARTIFACT_LAYOUT"
                        | "CODEX_SECURITY_SCAN_ID"
                        | "CODEX_SECURITY_PLUGIN_ROOT"
                        | "CODEX_SECURITY_SCOPE"
                        | "CODEX_SECURITY_PYTHON_COMMAND"
                        | "CODEX_SECURITY_REDUCER_CONTEXT_JSON"
                ),
                "private Codex artifact environment key differs"
            );
            let setting = parse_toml_string(setting)?;
            ensure!(
                !setting.is_empty() && setting.len() <= MAX_MCP_LINE_BYTES,
                "private Codex artifact environment value differs"
            );
            Ok(ProviderExecConfig::ArtifactEnvironment(
                variable.to_string(),
                setting,
            ))
        }
        _ => bail!("private Codex exec contains an unexpected config override"),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WorkerDirectoryKind {
    ScanArtifacts,
    WorkerOutput,
    ReducerOutput,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct WorkerDirectory {
    pub(super) path: PathBuf,
    scan_directory: PathBuf,
    kind: WorkerDirectoryKind,
}

pub(super) fn require_worker_directory(
    directory: &Path,
    scan_root: &Path,
) -> Result<WorkerDirectory> {
    ensure!(
        directory.is_absolute(),
        "private Codex worker directory is relative"
    );
    let canonical = canonical_directory(directory, "private Codex worker directory")?;
    ensure!(
        canonical == directory,
        "private Codex worker directory is redirected"
    );
    let relative = canonical
        .strip_prefix(scan_root)
        .context("private Codex worker directory escaped the owned scan root")?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()
        .context("private Codex worker directory is not UTF-8")?;
    let kind = if components.len() == 3 && components[2] == "artifacts" {
        WorkerDirectoryKind::ScanArtifacts
    } else if components.len() == 7
        && components[2] == "artifacts"
        && components[3] == "deep_discovery"
        && components[4] == "workers"
        && components[6] == "output"
    {
        WorkerDirectoryKind::WorkerOutput
    } else if components.len() == 7
        && components[2] == "artifacts"
        && components[3] == "deep_discovery"
        && components[4] == "dedup"
        && components[6] == "output"
    {
        WorkerDirectoryKind::ReducerOutput
    } else {
        bail!("private Codex worker directory shape differs");
    };
    ensure!(
        components.iter().all(|component| !component.is_empty()),
        "private Codex worker directory shape differs"
    );
    Ok(WorkerDirectory {
        path: canonical,
        scan_directory: scan_root.join(&components[0]).join(&components[1]),
        kind,
    })
}

pub(super) fn require_artifact_environment(
    environment: &BTreeMap<String, String>,
    working_directory: &WorkerDirectory,
    scan_root: &Path,
    repository: &Path,
    plugin_mirror: &Path,
    python_wrapper: &Path,
    marker: &str,
) -> Result<()> {
    let value = |name: &str| {
        environment
            .get(name)
            .with_context(|| format!("private Codex artifact environment lacks {name}"))
    };
    let layout = value("CODEX_SECURITY_ARTIFACT_LAYOUT")?;
    ensure!(
        matches!(layout.as_str(), "worker" | "reducer"),
        "private Codex artifact layout differs"
    );
    let mut expected: BTreeSet<String> = [
        "CODEX_SECURITY_ARTIFACT_ROOT",
        "CODEX_SECURITY_REPO_ROOT",
        "CODEX_SECURITY_ARTIFACT_LAYOUT",
        "CODEX_SECURITY_SCAN_ID",
        "CODEX_SECURITY_PLUGIN_ROOT",
        "CODEX_SECURITY_SCOPE",
        "CODEX_SECURITY_PYTHON_COMMAND",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    if layout == "reducer" {
        expected.insert("CODEX_SECURITY_REDUCER_CONTEXT_JSON".to_string());
    }
    ensure!(
        environment.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "private Codex artifact environment field set differs"
    );
    ensure!(
        Path::new(value("CODEX_SECURITY_REPO_ROOT")?) == repository,
        "private Codex artifact repository differs"
    );
    ensure!(
        Path::new(value("CODEX_SECURITY_PLUGIN_ROOT")?) == plugin_mirror,
        "private Codex artifact plugin root differs"
    );
    ensure!(
        Path::new(value("CODEX_SECURITY_PYTHON_COMMAND")?) == python_wrapper,
        "private Codex artifact Python wrapper differs"
    );
    ensure!(
        value("CODEX_SECURITY_SCOPE")? == ".",
        "private Codex artifact scope differs"
    );
    let scan_id = value("CODEX_SECURITY_SCAN_ID")?;
    require_safe_identity(scan_id, "private Codex artifact scan identity")?;
    let target_root = working_directory
        .scan_directory
        .parent()
        .context("private Codex artifact scan has no target root")?;
    ensure!(
        target_root.parent() == Some(scan_root)
            && target_root.file_name() == repository.file_name(),
        "private Codex artifact target root differs"
    );
    let source = scan_id_source_from_marker(marker)?;
    let expected_prefix = format!("{source}_");
    let scan_directory_name = working_directory
        .scan_directory
        .file_name()
        .and_then(OsStr::to_str)
        .context("private Codex artifact scan directory is not UTF-8")?;
    ensure!(
        scan_directory_name.starts_with(&expected_prefix)
            && scan_directory_name.len() > expected_prefix.len()
            && scan_directory_name[expected_prefix.len()..]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "private Codex artifact scan directory is not source-bound"
    );
    let artifact_root =
        require_worker_directory(Path::new(value("CODEX_SECURITY_ARTIFACT_ROOT")?), scan_root)?;
    ensure!(
        artifact_root.scan_directory == working_directory.scan_directory,
        "private Codex artifact and worker directories name different scans"
    );
    match layout.as_str() {
        "worker" => {
            ensure!(
                working_directory.kind == WorkerDirectoryKind::WorkerOutput
                    && artifact_root.kind == WorkerDirectoryKind::WorkerOutput
                    && working_directory.path == artifact_root.path,
                "private Codex discovery artifact layout differs"
            );
            require_discovery_prompt(
                &working_directory.path,
                scan_root,
                scan_id,
                repository,
                plugin_mirror,
            )?;
        }
        "reducer" => {
            ensure!(
                working_directory.kind == WorkerDirectoryKind::ScanArtifacts
                    && artifact_root.kind == WorkerDirectoryKind::ReducerOutput,
                "private Codex reducer artifact layout differs"
            );
            require_reducer_context(
                value("CODEX_SECURITY_REDUCER_CONTEXT_JSON")?,
                &working_directory.scan_directory,
                scan_root,
                scan_id,
            )?;
        }
        _ => unreachable!("artifact layouts other than worker and reducer were refused"),
    }
    Ok(())
}

fn require_discovery_prompt(
    worker_output: &Path,
    scan_root: &Path,
    scan_id: &str,
    repository: &Path,
    plugin_mirror: &Path,
) -> Result<()> {
    let worker_root = worker_output
        .parent()
        .context("private Codex discovery output has no worker root")?;
    let worker_label = worker_root
        .file_name()
        .and_then(OsStr::to_str)
        .context("private Codex discovery worker label is not UTF-8")?;
    require_safe_identity(worker_label, "private Codex discovery worker label")?;
    let prompt_path = worker_root.join("prompt.md");
    require_owned_path(&prompt_path, scan_root, "private Codex discovery prompt")?;
    let prompt = read(
        &prompt_path,
        MAX_WORKER_PROMPT_BYTES,
        "private Codex discovery prompt",
    )?;
    let prompt =
        std::str::from_utf8(&prompt).context("private Codex discovery prompt is not UTF-8")?;
    let context_and_body = prompt
        .strip_prefix(DISCOVERY_PROMPT_PREFIX)
        .context("private Codex discovery prompt prefix differs")?;
    let (context, body) = context_and_body
        .split_once("\n```\n")
        .context("private Codex discovery prompt has no context terminator")?;
    ensure!(
        body.starts_with(DISCOVERY_PROMPT_BODY_PREFIX),
        "private Codex discovery prompt body differs"
    );
    let parsed = parse_json(context.as_bytes(), "private Codex discovery context")?;
    let context = json_object(&parsed, "private Codex discovery context")?;
    require_json_fields(
        context,
        &[
            "scanId",
            "pluginRoot",
            "targetPath",
            "scope",
            "userContext",
            "workerLabel",
            "subagents",
        ],
        "private Codex discovery context",
    )?;
    ensure!(
        string(context, "scanId", "private Codex discovery context")? == scan_id
            && Path::new(string(
                context,
                "pluginRoot",
                "private Codex discovery context"
            )?) == plugin_mirror
            && Path::new(string(
                context,
                "targetPath",
                "private Codex discovery context"
            )?) == repository
            && string(context, "scope", "private Codex discovery context")? == "."
            && matches!(context.get("userContext"), Some(JsonValue::Null))
            && string(context, "workerLabel", "private Codex discovery context")? == worker_label
            && json_u64(
                json_field(context, "subagents", "private Codex discovery context")?,
                "private Codex discovery subagent count"
            )? == 3,
        "private Codex discovery context differs"
    );
    Ok(())
}

fn scan_id_source_from_marker(marker: &str) -> Result<&str> {
    let source = marker
        .strip_prefix(WRAPPER_MARKER_PREFIX)
        .context("private wrapper selector is malformed")?;
    ensure!(
        is_source_commit(source),
        "private wrapper selector source differs"
    );
    Ok(source)
}

fn require_reducer_context(
    value: &str,
    scan_directory: &Path,
    scan_root: &Path,
    scan_id: &str,
) -> Result<()> {
    let parsed = parse_json(value.as_bytes(), "private Codex reducer context")?;
    let context = json_object(&parsed, "private Codex reducer context")?;
    if context.contains_key("previousReducerResultPath") {
        require_json_fields(
            context,
            &["scanRoot", "claimedWorkers", "previousReducerResultPath"],
            "private Codex reducer context",
        )?;
    } else {
        require_json_fields(
            context,
            &["scanRoot", "claimedWorkers"],
            "private Codex reducer context",
        )?;
    }
    ensure!(
        Path::new(string(
            context,
            "scanRoot",
            "private Codex reducer context"
        )?) == scan_directory,
        "private Codex reducer scan root differs"
    );
    let workers = json_array(
        json_field(context, "claimedWorkers", "private Codex reducer context")?,
        "private Codex reducer workers",
    )?;
    ensure!(
        !workers.is_empty(),
        "private Codex reducer has no claimed workers"
    );
    let mut worker_ids = BTreeSet::new();
    for worker in workers {
        let worker = json_object(worker, "private Codex reducer worker")?;
        require_json_fields(
            worker,
            &["id", "resultPath"],
            "private Codex reducer worker",
        )?;
        let id = string(worker, "id", "private Codex reducer worker")?;
        require_safe_identity(id, "private Codex reducer worker identity")?;
        ensure!(
            worker_ids.insert(id),
            "private Codex reducer repeats a worker identity"
        );
        let result_path = Path::new(string(
            worker,
            "resultPath",
            "private Codex reducer worker",
        )?);
        require_deep_result_path(result_path, scan_directory, scan_root, "workers")?;
        require_result_scan_id(result_path, scan_id)?;
    }
    if let Some(previous) = context.get("previousReducerResultPath") {
        let result_path = Path::new(json_string(
            previous,
            "private Codex previous reducer result",
        )?);
        require_deep_result_path(result_path, scan_directory, scan_root, "dedup")?;
        require_result_scan_id(result_path, scan_id)?;
    }
    Ok(())
}

fn require_result_scan_id(path: &Path, scan_id: &str) -> Result<()> {
    let result = read(path, MAX_FINDINGS_BYTES, "private Codex reducer source")?;
    let parsed = parse_json(&result, "private Codex reducer source")?;
    let result = json_object(&parsed, "private Codex reducer source")?;
    ensure!(
        string(result, "scanId", "private Codex reducer source")? == scan_id,
        "private Codex reducer source belongs to another scan"
    );
    Ok(())
}

fn require_deep_result_path(
    path: &Path,
    scan_directory: &Path,
    scan_root: &Path,
    phase: &str,
) -> Result<()> {
    require_owned_path(path, scan_root, "private Codex reducer source")?;
    let relative = path
        .strip_prefix(scan_directory)
        .context("private Codex reducer source escaped its scan")?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .context("private Codex reducer source is not UTF-8")?;
    ensure!(
        components.len() == 6
            && components[0] == "artifacts"
            && components[1] == "deep_discovery"
            && components[2] == phase
            && !components[3].is_empty()
            && components[4] == "output"
            && components[5] == "result.json",
        "private Codex reducer source shape differs"
    );
    Ok(())
}

pub(super) fn require_nonempty_argument(value: &OsStr, label: &str) -> Result<()> {
    let value = value
        .to_str()
        .with_context(|| format!("{label} is not UTF-8"))?;
    ensure!(
        !value.is_empty() && !value.contains(['\r', '\n', '\0']),
        "{label} is malformed"
    );
    Ok(())
}
