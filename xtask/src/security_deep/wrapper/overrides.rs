use super::*;

pub(crate) fn require_owned_path(path: &Path, root: &Path, label: &str) -> Result<()> {
    ensure!(path.is_absolute(), "{label} is relative");
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{label} escaped its owned root"))?;
    ensure!(
        !relative.as_os_str().is_empty(),
        "{label} names its owned root"
    );
    ensure!(
        relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_))),
        "{label} is not lexically normalized"
    );
    let mut current = root.to_path_buf();
    let mut missing = false;
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else {
            unreachable!();
        };
        current.push(name);
        if missing {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) => {
                ensure!(
                    !metadata.file_type().is_symlink() && !metadata_is_reparse_point(&metadata),
                    "{label} contains a redirected component"
                );
                let is_final = index + 1 == components.len();
                ensure!(
                    crate::mutate::workspace::metadata_is_real_directory(&metadata)
                        || (is_final && metadata.file_type().is_file()),
                    "{label} contains a special or non-directory ancestor"
                );
                let canonical = std::fs::canonicalize(&current).with_context(|| {
                    format!("canonicalizing {label} component {}", current.display())
                })?;
                ensure!(
                    canonical == current,
                    "{label} contains a redirected component"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = current
                    .parent()
                    .context("owned path component has no existing parent")?;
                ensure!(
                    canonical_directory(parent, label)? == parent,
                    "{label} has a redirected existing parent"
                );
                missing = true;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspecting {label} {}", current.display()));
            }
        }
    }
    Ok(())
}

pub(crate) fn parse_wrapper_overrides(serialized: &str) -> Result<Vec<String>> {
    let value = parse_json(serialized.as_bytes(), "private Codex wrapper policy")?;
    let values = json_array(&value, "private Codex wrapper policy")?;
    let mut overrides = Vec::with_capacity(values.len());
    for value in values {
        let value = json_string(value, "private Codex wrapper override")?;
        ensure!(
            !value.is_empty() && !value.contains(['\r', '\n', '\0']),
            "private Codex wrapper override is malformed"
        );
        overrides.push(value.to_string());
    }
    require_wrapper_override_policy(&overrides)?;
    Ok(overrides)
}

pub(crate) fn require_wrapper_override_policy(overrides: &[String]) -> Result<()> {
    let fixed = fixed_safety_overrides();
    let (sandbox, policy) = overrides
        .split_last()
        .context("private Codex wrapper safety policy is empty")?;
    ensure!(
        sandbox == WINDOWS_SANDBOX_OVERRIDE
            && policy.len() >= fixed.len()
            && policy[..fixed.len()] == fixed,
        "private Codex wrapper safety policy differs"
    );
    let dynamic = &policy[fixed.len()..];
    ensure!(
        dynamic.windows(2).all(|pair| pair[0] < pair[1]),
        "private Codex wrapper dynamic policy is not unique and sorted"
    );
    let mut mcp_fields: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut mcp_commands = BTreeMap::new();
    let mut plugins = BTreeSet::new();
    for value in dynamic {
        if let Some(name) = value
            .strip_prefix("plugins.")
            .and_then(|value| value.strip_suffix(".enabled=false"))
        {
            require_config_name(name)?;
            ensure!(
                plugins.insert(name.to_string()),
                "private Codex wrapper plugin disable is duplicated"
            );
            continue;
        }
        let value = value
            .strip_prefix("mcp_servers.")
            .context("private Codex wrapper dynamic override is not a disable")?;
        let (name, field) = value
            .split_once('.')
            .context("private Codex wrapper MCP override has no field")?;
        require_config_name(name)?;
        let canonical_field = if field == "args=[]" {
            "args".to_string()
        } else if field == "enabled=false" {
            "enabled".to_string()
        } else if field == "env_vars=[]" {
            "env_vars".to_string()
        } else if field == format!("environment_id=\"{INERT_MCP_ENVIRONMENT}\"") {
            "environment_id".to_string()
        } else if field == format!("startup_timeout_sec={INERT_MCP_TIMEOUT_SECONDS}") {
            "startup_timeout_sec".to_string()
        } else if field == format!("tool_timeout_sec={INERT_MCP_TIMEOUT_SECONDS}") {
            "tool_timeout_sec".to_string()
        } else if let Some(variable) = field
            .strip_prefix("env.")
            .and_then(|value| value.strip_suffix("=\"\""))
        {
            require_config_name(variable)?;
            format!("env.{variable}")
        } else if let Some(command) = field.strip_prefix("command=") {
            let command = PathBuf::from(parse_toml_string(command)?);
            require_inert_mcp_path(&command)?;
            ensure!(
                mcp_commands.insert(name.to_string(), command).is_none(),
                "private Codex wrapper MCP command is duplicated"
            );
            "command".to_string()
        } else {
            bail!("private Codex wrapper MCP override differs from the inert policy");
        };
        ensure!(
            mcp_fields
                .entry(name.to_string())
                .or_default()
                .insert(canonical_field),
            "private Codex wrapper MCP field is duplicated"
        );
    }
    let expected_fields: BTreeSet<String> = [
        "args",
        "command",
        "enabled",
        "env_vars",
        "environment_id",
        "startup_timeout_sec",
        "tool_timeout_sec",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    for (name, fields) in &mcp_fields {
        ensure!(
            expected_fields.is_subset(fields)
                && fields
                    .iter()
                    .all(|field| expected_fields.contains(field) || field.starts_with("env."))
                && mcp_commands.contains_key(name),
            "private Codex wrapper MCP policy is incomplete"
        );
    }
    Ok(())
}

pub(crate) fn inject_wrapper_overrides(
    arguments: &[OsString],
    overrides: &[String],
) -> Result<InjectedCodexInvocation> {
    let provider_permission = std::env::var(WRAPPER_PROVIDER_PERMISSION_ENV)
        .context("private Codex wrapper has no provider permission profile")?;
    let worker_permission = std::env::var(WRAPPER_WORKER_PERMISSION_ENV)
        .context("private Codex wrapper has no effective worker permission profile")?;
    let context = if arguments.first().is_some_and(|arg| arg == "exec") {
        let supplied = std::env::var_os("CODEX_SECURITY_SCAN_ROOT")
            .context("private Codex exec has no owned scan root")?;
        let plugin = std::env::var_os(PYTHON_WRAPPER_PLUGIN_ENV)
            .context("private Codex exec has no plugin mirror")?;
        let repository = std::env::var_os(WRAPPER_REPOSITORY_ENV)
            .context("private Codex exec has no target repository")?;
        let python_real = std::env::var_os(PYTHON_WRAPPER_REAL_ENV)
            .context("private Codex exec has no real Python runtime")?;
        let pycache = std::env::var_os(PYTHON_WRAPPER_PYCACHE_ENV)
            .context("private Codex exec has no Python bytecode directory")?;
        let python_runtime = parse_python_runtime_policy(
            &std::env::var(PYTHON_WRAPPER_RUNTIME_ENV)
                .context("private Codex exec has no Python runtime-tree policy")?,
        )?;
        let node = std::env::var_os(WRAPPER_NODE_ENV)
            .context("private Codex exec has no qualified Node runtime")?;
        let codex = std::env::var_os(WRAPPER_REAL_ENV)
            .context("private Codex exec has no qualified Codex runtime")?;
        let live_auth = std::env::var_os(WRAPPER_LIVE_AUTH_ENV)
            .context("private Codex exec has no live credential deny path")?;
        let projected_auth = PathBuf::from(
            std::env::var_os("CODEX_HOME").context("private Codex exec has no private home")?,
        )
        .join("auth.json");
        let git = std::env::var_os("CODEX_SECURITY_GIT")
            .context("private Codex exec has no qualified Git runtime")?;
        let marker = std::env::var(WRAPPER_MARKER_ENV)
            .context("private Codex exec has no wrapper selector")?;
        let temporary = Path::new(&pycache)
            .parent()
            .context("private Python bytecode directory has no scratch parent")?
            .join("temp");
        let shell_environment = ShellEnvironmentPolicy::qualify_paths(
            Path::new(&node),
            Path::new(&python_real),
            Path::new(&codex),
            Path::new(&git),
            &temporary,
        )?;
        let context = PrivateExecutionContext {
            scan_root: canonical_directory(Path::new(&supplied), "private Codex exec scan root")?,
            plugin_mirror: canonical_directory(
                Path::new(&plugin),
                "private Codex exec plugin mirror",
            )?,
            repository: canonical_directory(
                Path::new(&repository),
                "private Codex exec target repository",
            )?,
            python_wrapper: std::fs::canonicalize(
                std::env::current_exe().context("resolving private Python wrapper")?,
            )
            .context("canonicalizing private Python wrapper")?,
            python_real: std::fs::canonicalize(Path::new(&python_real))
                .context("canonicalizing private Python runtime")?,
            pycache: canonical_directory(
                Path::new(&pycache),
                "private Codex exec Python bytecode directory",
            )?,
            python_runtime,
            artifact_node: ToolIdentity {
                path: std::fs::canonicalize(Path::new(&node))
                    .context("canonicalizing private artifact Node runtime")?,
                bytes: NODE_BYTES,
                sha256: NODE_SHA256.to_string(),
                version: NODE_VERSION.to_string(),
            },
            codex_runtime: identify_codex_runtime(Path::new(&codex))?,
            live_auth: PathBuf::from(live_auth),
            projected_auth,
            shell_environment,
            marker,
        };
        context.require_exact()?;
        Some(context)
    } else {
        None
    };
    inject_wrapper_overrides_for_scan_root(
        arguments,
        overrides,
        context.as_ref(),
        &provider_permission,
        &worker_permission,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InjectedCodexInvocation {
    pub(crate) arguments: Vec<OsString>,
    pub(crate) transition: WorkerTransitionKind,
    pub(crate) worker: Option<WorkerDirectory>,
    pub(crate) expected_thread: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedExecArguments {
    insertion: usize,
    worker: WorkerDirectory,
    resumed_thread: Option<String>,
}

pub(crate) fn inject_wrapper_overrides_for_scan_root(
    arguments: &[OsString],
    overrides: &[String],
    context: Option<&PrivateExecutionContext>,
    provider_permission: &str,
    worker_permission: &str,
) -> Result<InjectedCodexInvocation> {
    require_wrapper_override_policy(overrides)?;
    ensure!(
        !provider_permission.is_empty()
            && !worker_permission.is_empty()
            && provider_permission != worker_permission
            && !provider_permission.contains(['\r', '\n', '\0'])
            && !worker_permission.contains(['\r', '\n', '\0']),
        "private Codex named permission profiles differ"
    );
    let (insertion, transition, worker, expected_thread) =
        if arguments.first().is_some_and(|arg| arg == "exec") {
            ensure!(
                arguments
                    .get(1)
                    .is_some_and(|arg| arg == "--experimental-json"),
                "private Codex exec lacks its pinned protocol mode"
            );
            let context = context.context("private Codex exec has no owned runtime context")?;
            let (expected_provider, expected_worker) = context_named_permission_profiles(context)?;
            ensure!(
                provider_permission == expected_provider && worker_permission == expected_worker,
                "private Codex exec named permission profiles differ from the owned context"
            );
            let validated = validate_exec_arguments(arguments, context, provider_permission)?;
            let transition = if validated.resumed_thread.is_some() {
                WorkerTransitionKind::ExecResumedAttenuated
            } else {
                WorkerTransitionKind::ExecInitialAttenuated
            };
            (
                validated.insertion,
                transition,
                Some(validated.worker),
                validated.resumed_thread,
            )
        } else {
            ensure!(
                context.is_none(),
                "private Codex app-server received an exec context"
            );
            (
                validate_app_server_arguments(arguments, provider_permission)?,
                WorkerTransitionKind::AppServerPassThrough,
                None,
                None,
            )
        };
    let artifact_overrides = match context {
        Some(context) => context.artifact_overrides()?,
        None => Vec::new(),
    };
    let (sandbox_override, policy_overrides) = overrides
        .split_last()
        .context("private Codex wrapper safety policy is empty")?;
    ensure!(
        sandbox_override == WINDOWS_SANDBOX_OVERRIDE,
        "private Codex wrapper MXC selector is not safety-last"
    );
    let isolation_flags = if context.is_some() {
        &[
            OsString::from("--ephemeral"),
            OsString::from("--ignore-user-config"),
            OsString::from("--ignore-rules"),
            OsString::from("--strict-config"),
        ][..]
    } else {
        &[][..]
    };
    let mut result = Vec::with_capacity(
        arguments.len()
            + isolation_flags.len()
            + usize::from(context.is_some())
            + (overrides.len() + artifact_overrides.len()) * 2,
    );
    if context.is_some() {
        result.push(OsString::from("--no-daemon"));
        result.extend_from_slice(&arguments[..2]);
        result.extend_from_slice(isolation_flags);
        result.extend_from_slice(&arguments[2..insertion]);
    } else {
        result.extend_from_slice(&arguments[..insertion]);
    }
    for value in policy_overrides.iter().chain(&artifact_overrides) {
        result.push(OsString::from("--config"));
        result.push(OsString::from(value));
    }
    result.push(OsString::from("--config"));
    result.push(OsString::from(sandbox_override));
    result.extend_from_slice(&arguments[insertion..]);
    if context.is_some() {
        let mut rewritten_permissions = 0usize;
        for argument in &mut result {
            if argument == provider_permission {
                *argument = OsString::from(worker_permission);
                rewritten_permissions += 1;
            }
        }
        ensure!(
            rewritten_permissions == 1,
            "private Codex worker permission profile was not attenuated exactly once"
        );
    } else {
        ensure!(
            result
                .iter()
                .filter(|argument| argument.as_os_str() == OsStr::new(provider_permission))
                .count()
                == 1
                && !result
                    .iter()
                    .any(|argument| argument.as_os_str() == OsStr::new(worker_permission)),
            "private Codex app-server permission profile was altered"
        );
    }
    Ok(InjectedCodexInvocation {
        arguments: result,
        transition,
        worker,
        expected_thread,
    })
}

fn validate_app_server_arguments(
    arguments: &[OsString],
    provider_permission: &str,
) -> Result<usize> {
    let mut index = 0usize;
    let mut default_permissions = 0usize;
    let mut permission_profile = 0usize;
    while arguments
        .get(index)
        .is_some_and(|argument| argument == "--config")
    {
        let value = arguments
            .get(index + 1)
            .context("private Codex app-server has a config flag without a value")?;
        require_nonempty_argument(value, "private Codex app-server config")?;
        match value
            .to_str()
            .context("private Codex app-server config is not UTF-8")?
        {
            "default_permissions=\"codex_security_deep_scan_worker\"" => {
                default_permissions += 1;
            }
            value if value == provider_permission => {
                permission_profile += 1;
            }
            _ => bail!("private Codex app-server contains an unexpected config override"),
        }
        index += 2;
    }
    ensure!(
        arguments
            .get(index)
            .is_some_and(|argument| argument == "app-server")
            && arguments
                .get(index + 1)
                .is_some_and(|argument| argument == "--stdio")
            && index + 2 == arguments.len(),
        "private Codex app-server invocation shape differs"
    );
    ensure!(
        default_permissions == 1 && permission_profile == 1,
        "private Codex app-server permission policy differs"
    );
    Ok(index)
}

fn validate_exec_arguments(
    arguments: &[OsString],
    context: &PrivateExecutionContext,
    provider_permission: &str,
) -> Result<ValidatedExecArguments> {
    let scan_root = canonical_directory(&context.scan_root, "private Codex exec scan root")?;
    let plugin_mirror =
        canonical_directory(&context.plugin_mirror, "private Codex exec plugin mirror")?;
    let repository = canonical_directory(&context.repository, "private Codex exec repository")?;
    let mut index = 2usize;
    let mut model = 0usize;
    let mut thread_source = 0usize;
    let mut working_directory = 0usize;
    let mut skip_git = 0usize;
    let mut approval = 0usize;
    let mut default_permissions = 0usize;
    let mut permission_profile = 0usize;
    let mut reasoning = 0usize;
    let mut codex_security_command = 0usize;
    let mut codex_security_enabled = 0usize;
    let mut artifact_command = 0usize;
    let mut artifact_args = 0usize;
    let mut artifact_required = 0usize;
    let mut artifact_startup = 0usize;
    let mut artifact_tool = 0usize;
    let mut agents = 0usize;
    let mut multi_agent_enabled = 0usize;
    let mut multi_agent_maximum = 0usize;
    let mut optional_configs = BTreeSet::new();
    let mut artifact_environment = BTreeMap::new();
    let mut worker_directory = None;
    let mut resumed_thread = None;
    while index < arguments.len() {
        let argument = arguments[index]
            .to_str()
            .context("private Codex exec argument is not UTF-8")?;
        if argument == "resume" {
            let thread = arguments
                .get(index + 1)
                .context("private Codex resume has no thread identity")?;
            require_nonempty_argument(thread, "private Codex resume thread")?;
            let thread = thread
                .to_str()
                .context("private Codex resume thread is not UTF-8")?;
            require_safe_identity(thread, "private Codex resume thread")?;
            ensure!(
                index + 2 == arguments.len(),
                "private Codex resume is not the terminal positional form"
            );
            resumed_thread = Some(thread.to_string());
            break;
        }
        if argument == "--skip-git-repo-check" {
            skip_git += 1;
            index += 1;
            continue;
        }
        ensure!(
            matches!(
                argument,
                "--config" | "--model" | "--thread-source" | "--cd"
            ),
            "private Codex exec contains an unexpected argument"
        );
        let value = arguments
            .get(index + 1)
            .context("private Codex exec option has no value")?;
        require_nonempty_argument(value, "private Codex exec option")?;
        let value = value
            .to_str()
            .context("private Codex exec option is not UTF-8")?;
        match argument {
            "--model" => {
                ensure!(value == MODEL, "private Codex exec model differs");
                model += 1;
            }
            "--thread-source" => {
                ensure!(
                    value == "security_scan",
                    "private Codex exec thread source differs"
                );
                thread_source += 1;
            }
            "--cd" => {
                worker_directory = Some(require_worker_directory(Path::new(value), &scan_root)?);
                working_directory += 1;
            }
            "--config" => {
                match require_provider_exec_config(
                    value,
                    &plugin_mirror,
                    &context.artifact_node,
                    provider_permission,
                )? {
                    ProviderExecConfig::Approval => approval += 1,
                    ProviderExecConfig::DefaultPermissions => default_permissions += 1,
                    ProviderExecConfig::PermissionProfile => permission_profile += 1,
                    ProviderExecConfig::Reasoning => reasoning += 1,
                    ProviderExecConfig::CodexSecurityCommand => codex_security_command += 1,
                    ProviderExecConfig::CodexSecurityEnabled => codex_security_enabled += 1,
                    ProviderExecConfig::ArtifactCommand => artifact_command += 1,
                    ProviderExecConfig::ArtifactArgs => artifact_args += 1,
                    ProviderExecConfig::ArtifactRequired => artifact_required += 1,
                    ProviderExecConfig::ArtifactStartup => artifact_startup += 1,
                    ProviderExecConfig::ArtifactTool => artifact_tool += 1,
                    ProviderExecConfig::Agents => agents += 1,
                    ProviderExecConfig::MultiAgentEnabled => multi_agent_enabled += 1,
                    ProviderExecConfig::MultiAgentMaximum => multi_agent_maximum += 1,
                    ProviderExecConfig::Optional(name) => ensure!(
                        optional_configs.insert(name),
                        "private Codex exec optional configuration is duplicated"
                    ),
                    ProviderExecConfig::ArtifactEnvironment(name, value) => ensure!(
                        artifact_environment.insert(name, value).is_none(),
                        "private Codex artifact environment is duplicated"
                    ),
                }
            }
            _ => unreachable!(),
        }
        index += 2;
    }
    ensure!(
        model == 1 && working_directory == 1 && skip_git == 1,
        "private Codex exec lacks its exact model, directory, or Git policy"
    );
    ensure!(
        (resumed_thread.is_some() && thread_source == 0)
            || (resumed_thread.is_none() && thread_source == 1),
        "private Codex exec source and resume policy differs"
    );
    ensure!(
        approval == 1
            && default_permissions == 1
            && permission_profile == 1
            && reasoning == 1
            && codex_security_command == 1
            && codex_security_enabled == 1
            && artifact_command == 1
            && artifact_args == 1
            && artifact_required == 1
            && artifact_startup == 1
            && artifact_tool == 1
            && agents == 1
            && multi_agent_enabled == 1
            && multi_agent_maximum == 1,
        "private Codex exec provider configuration is incomplete or duplicated"
    );
    let worker = worker_directory.context("private Codex exec has no worker directory")?;
    require_artifact_environment(
        &artifact_environment,
        &worker,
        &scan_root,
        &repository,
        &plugin_mirror,
        &context.python_wrapper,
        &context.marker,
    )?;
    Ok(ValidatedExecArguments {
        insertion: if resumed_thread.is_some() {
            index
        } else {
            arguments.len()
        },
        worker,
        resumed_thread,
    })
}
