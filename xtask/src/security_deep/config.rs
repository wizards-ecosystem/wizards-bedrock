use super::*;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct EffectiveConfig {
    pub(super) model: Option<String>,
    pub(super) model_provider: Option<String>,
    pub(super) openai_base_url: Option<String>,
    pub(super) chatgpt_base_url: Option<String>,
    pub(super) reasoning: Option<String>,
    pub(super) service_tier: Option<String>,
    pub(super) notify_is_empty: bool,
    pub(super) memories_generate: Option<bool>,
    pub(super) memories_use: Option<bool>,
    pub(super) feature_memories: Option<bool>,
    pub(super) windows_sandbox: Option<String>,
    pub(super) mcp_servers: BTreeMap<String, McpRoute>,
    pub(super) plugins: BTreeMap<String, bool>,
    pub(super) model_providers: BTreeSet<String>,
    pub(super) origins: BTreeSet<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct McpRoute {
    pub(super) enabled: bool,
    pub(super) command: Option<String>,
    pub(super) args: Option<Vec<String>>,
    pub(super) fields: BTreeSet<String>,
    pub(super) env_keys: BTreeSet<String>,
    pub(super) env_values_empty: bool,
    pub(super) env_vars_empty: bool,
    pub(super) environment_id: Option<String>,
    pub(super) startup_timeout_seconds: Option<u64>,
    pub(super) tool_timeout_seconds: Option<u64>,
}

pub(super) fn read_codex_config(
    codex: &ToolIdentity,
    repository: &Path,
    scratch: &OwnedScratch,
    overrides: &[String],
) -> Result<EffectiveConfig> {
    require_private_credential_projection(&scratch.codex_home())?;
    let result = read_codex_config_with_command(
        codex_config_command(codex, overrides),
        repository,
        scratch,
    )?;
    require_private_credential_projection(&scratch.codex_home())?;
    Ok(result)
}

pub(super) fn codex_config_command(codex: &ToolIdentity, overrides: &[String]) -> Command {
    let mut command = Command::new(&codex.path);
    for value in overrides {
        command.arg("--config").arg(value);
    }
    command.arg("app-server").arg("--stdio");
    command
}

fn read_codex_config_with_command(
    mut command: Command,
    repository: &Path,
    scratch: &OwnedScratch,
) -> Result<EffectiveConfig> {
    #[cfg(test)]
    prepare_config_test_home(scratch)?;
    let repository = canonical_directory(repository, "Codex config target")?;
    command.current_dir(&repository);
    apply_base_environment(&mut command, scratch, None)?;
    let config_repository = repository.clone();
    let config_home = scratch.codex_home();
    let output = crate::bounded_process::output_dialogue(
        command,
        CONFIG_TIMEOUT,
        "Codex effective-config reader",
        DialogueLimits {
            line_bytes: 4 * 1024 * 1024,
            stdout_bytes: 8 * 1024 * 1024,
            stderr_bytes: 1024 * 1024,
        },
        || Ok(()),
        move |dialogue| exchange_codex_config(dialogue, &config_repository, &config_home),
    )?;
    ensure!(
        output.status.success(),
        "Codex effective-config reader failed ({})",
        output.status
    );
    ensure!(
        std::str::from_utf8(&output.stderr).is_ok(),
        "Codex effective-config diagnostics are not UTF-8"
    );
    Ok(output.value)
}

#[cfg(test)]
pub(super) fn prepare_config_test_home(scratch: &OwnedScratch) -> Result<()> {
    let home = scratch.codex_home();
    if !home.exists() {
        crate::mutate::workspace::prepare_real_child(
            &scratch.root,
            &home,
            "test private Codex home",
        )?;
    }
    let config = home.join("config.toml");
    if !config.exists() {
        write_new_file(&config, b"")?;
    }
    Ok(())
}

pub(super) fn exchange_codex_config(
    dialogue: &mut LineDialogue,
    repository: &Path,
    codex_home: &Path,
) -> Result<EffectiveConfig> {
    send_app_request(
        dialogue,
        1,
        "initialize",
        object([
            (
                "capabilities",
                object([("experimentalApi", JsonValue::Bool(true))]),
            ),
            (
                "clientInfo",
                object([
                    ("name", text_value("codex_security_deep_scan")),
                    ("title", text_value("Codex Security Deep Scan")),
                    ("version", text_value(MCP_SERVER_VERSION)),
                ]),
            ),
        ]),
    )?;
    let initialized = receive_app_response(dialogue, 1)?;
    require_app_initialize(&initialized, codex_home)?;
    send_value(
        dialogue,
        &object([
            ("method", text_value("initialized")),
            ("params", object(std::iter::empty::<(&str, JsonValue)>())),
        ]),
    )?;
    send_app_request(
        dialogue,
        2,
        "config/read",
        object([
            ("cwd", path_value(repository)),
            ("includeLayers", JsonValue::Bool(false)),
        ]),
    )?;
    let result = receive_app_response(dialogue, 2)?;
    let config = parse_effective_config(&result)?;
    dialogue.close_input();
    ensure!(
        dialogue.receive_line()?.is_none(),
        "Codex app-server sent output after its final config/read response"
    );
    Ok(config)
}

fn require_app_initialize(result: &JsonValue, expected_codex_home: &Path) -> Result<()> {
    let result = json_object(result, "Codex app-server initialize result")?;
    require_json_fields(
        result,
        &["codexHome", "platformFamily", "platformOs", "userAgent"],
        "Codex app-server initialize result",
    )?;
    let codex_home = json_string(
        json_field(result, "codexHome", "Codex app-server initialize result")?,
        "Codex app-server Codex home",
    )?;
    ensure!(
        canonical_spelling_matches(expected_codex_home, Path::new(codex_home)),
        "Codex app-server did not use the private Codex home"
    );
    ensure!(
        json_string(
            json_field(
                result,
                "platformFamily",
                "Codex app-server initialize result"
            )?,
            "Codex app-server platform family"
        )? == "windows"
            && json_string(
                json_field(result, "platformOs", "Codex app-server initialize result")?,
                "Codex app-server platform OS"
            )? == "windows",
        "Codex app-server platform differs from the qualified Windows host"
    );
    let user_agent = json_string(
        json_field(result, "userAgent", "Codex app-server initialize result")?,
        "Codex app-server user agent",
    )?;
    let codex_version = CODEX_VERSION
        .strip_prefix("codex-cli ")
        .context("qualified Codex version has no CLI prefix")?;
    let prefix = format!("codex_security_deep_scan/{codex_version} (");
    let suffix = format!(") unknown (codex_security_deep_scan; {MCP_SERVER_VERSION})");
    let host = user_agent
        .strip_prefix(&prefix)
        .and_then(|value| value.strip_suffix(&suffix))
        .context("Codex app-server user agent differs from the qualified client")?;
    ensure!(
        !host.is_empty()
            && host.len() <= 256
            && host
                .bytes()
                .all(|byte| byte.is_ascii_graphic() || byte == b' '),
        "Codex app-server host identity is malformed"
    );
    Ok(())
}

fn send_app_request(
    dialogue: &mut LineDialogue,
    id: u64,
    method: &str,
    params: JsonValue,
) -> Result<()> {
    send_value(
        dialogue,
        &object([
            ("id", JsonValue::Number(id.to_string())),
            ("method", text_value(method)),
            ("params", params),
        ]),
    )
}

fn receive_app_response(dialogue: &mut LineDialogue, expected_id: u64) -> Result<JsonValue> {
    loop {
        let mut line = dialogue.receive_line()?.with_context(|| {
            format!("Codex app-server closed before its response {expected_id}")
        })?;
        ensure!(
            line.last() == Some(&b'\n')
                && line.iter().filter(|byte| **byte == b'\n').count() == 1
                && !line.contains(&b'\r'),
            "Codex app-server response is not strict LF framing"
        );
        line.pop();
        let value = parse_json(&line, "Codex app-server response")?;
        let JsonValue::Object(mut response) = value else {
            bail!("Codex app-server response is not an object");
        };
        if response.contains_key("method") {
            require_json_fields(
                &response,
                &["emittedAtMs", "method", "params"],
                "Codex app-server notification",
            )?;
            ensure!(
                json_string(
                    json_field(&response, "method", "Codex app-server notification")?,
                    "Codex app-server notification method"
                )? == "remoteControl/status/changed",
                "Codex app-server sent an unexpected notification"
            );
            json_u64(
                json_field(&response, "emittedAtMs", "Codex app-server notification")?,
                "Codex app-server notification time",
            )?;
            json_object(
                json_field(&response, "params", "Codex app-server notification")?,
                "Codex app-server notification parameters",
            )?;
            continue;
        }
        require_allowed_fields(
            &response,
            &["id", "result", "error"],
            "Codex app-server response",
        )?;
        ensure!(
            json_u64(
                json_field(&response, "id", "Codex app-server response")?,
                "Codex app-server response ID"
            )? == expected_id,
            "Codex app-server response has an unexpected ID"
        );
        ensure!(
            response.contains_key("result") ^ response.contains_key("error"),
            "Codex app-server response must contain exactly one result or error"
        );
        if let Some(error) = response.get("error") {
            bail!(
                "Codex app-server request {expected_id} failed: {}",
                render_json(error)?
            );
        }
        return response
            .remove("result")
            .context("Codex app-server response has no result");
    }
}

fn parse_effective_config(result: &JsonValue) -> Result<EffectiveConfig> {
    let result = json_object(result, "Codex config/read result")?;
    require_json_fields(result, &["config", "origins"], "Codex config/read result")?;
    let config = json_object(
        json_field(result, "config", "Codex config/read result")?,
        "Codex effective config",
    )?;
    require_codex_default_config(config)?;
    let origins = json_object(
        json_field(result, "origins", "Codex config/read result")?,
        "Codex config origins",
    )?
    .keys()
    .cloned()
    .collect();
    let mcp_servers = named_mcp_routes(config)?;
    let plugins = named_enablement(config, "plugins", "Codex plugins")?;
    let model_providers = optional_object(config, "model_providers", "Codex model providers")?
        .map(|providers| providers.keys().cloned().collect())
        .unwrap_or_default();
    let memories = optional_object(config, "memories", "Codex memories")?;
    let features = optional_object(config, "features", "Codex features")?;
    let windows = optional_object(config, "windows", "Codex Windows")?;
    let notify_is_empty = match config.get("notify") {
        None | Some(JsonValue::Null) => true,
        Some(value) => json_array(value, "Codex notify commands")?.is_empty(),
    };
    Ok(EffectiveConfig {
        model: optional_config_string(config, "model", "Codex model")?,
        model_provider: optional_config_string(config, "model_provider", "Codex model provider")?,
        openai_base_url: optional_config_string(
            config,
            "openai_base_url",
            "Codex OpenAI base URL",
        )?,
        chatgpt_base_url: optional_config_string(
            config,
            "chatgpt_base_url",
            "Codex ChatGPT base URL",
        )?,
        reasoning: optional_config_string(
            config,
            "model_reasoning_effort",
            "Codex reasoning effort",
        )?,
        service_tier: optional_config_string(config, "service_tier", "Codex service tier")?,
        notify_is_empty,
        memories_generate: optional_bool(memories, "generate_memories", "Codex memories")?,
        memories_use: optional_bool(memories, "use_memories", "Codex memories")?,
        feature_memories: optional_bool(features, "memories", "Codex features")?,
        windows_sandbox: match windows.and_then(|windows| windows.get("sandbox")) {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(json_string(value, "Codex Windows sandbox")?.to_string()),
        },
        mcp_servers,
        plugins,
        model_providers,
        origins,
    })
}

fn require_codex_default_config(config: &BTreeMap<String, JsonValue>) -> Result<()> {
    let mut defaults = config.clone();
    for field in [
        "model",
        "model_provider",
        "openai_base_url",
        "chatgpt_base_url",
        "model_reasoning_effort",
        "service_tier",
        "notify",
        "mcp_servers",
        "plugins",
        "model_providers",
    ] {
        defaults.remove(field);
    }
    remove_owned_config_fields(
        &mut defaults,
        "memories",
        &["generate_memories", "use_memories"],
    )?;
    remove_owned_config_fields(&mut defaults, "features", &["memories"])?;
    remove_owned_config_fields(&mut defaults, "windows", &["sandbox"])?;
    // Disabling memories makes this Codex runtime project the rest of the
    // memories schema, and its associated network feature, as explicit nulls.
    // Normalize only those exact inert values; any active or newly introduced
    // default remains part of the pinned full-schema digest.
    remove_null_config_fields(
        &mut defaults,
        "memories",
        &[
            "consolidation_model",
            "dedicated_tools",
            "disable_on_external_context",
            "dual_write",
            "extract_model",
            "max_raw_memories_for_consolidation",
            "max_rollout_age_days",
            "max_rollouts_per_startup",
            "max_unused_days",
            "min_rate_limit_remaining_percent",
            "min_rollout_idle_hours",
            "version",
        ],
    )?;
    remove_null_config_fields(&mut defaults, "features", &["network_proxy"])?;
    let digest = sha256::hex(&sha256::digest(
        render_json(&JsonValue::Object(defaults))?.as_bytes(),
    ));
    ensure!(
        codex_default_config_digest_is_allowed(&digest),
        "Codex originless default config differs: observed sha256 {digest}"
    );
    Ok(())
}

fn remove_owned_config_fields(
    config: &mut BTreeMap<String, JsonValue>,
    root: &str,
    fields: &[&str],
) -> Result<()> {
    match config.get_mut(root) {
        None | Some(JsonValue::Null) => {
            config.remove(root);
        }
        Some(JsonValue::Object(object)) => {
            for field in fields {
                object.remove(*field);
            }
            if object.is_empty() {
                config.remove(root);
            }
        }
        Some(_) => bail!("Codex {root} config is not an object"),
    }
    Ok(())
}

fn remove_null_config_fields(
    config: &mut BTreeMap<String, JsonValue>,
    root: &str,
    fields: &[&str],
) -> Result<()> {
    match config.get_mut(root) {
        None | Some(JsonValue::Null) => {
            config.remove(root);
        }
        Some(JsonValue::Object(object)) => {
            for field in fields {
                match object.remove(*field) {
                    None | Some(JsonValue::Null) => {}
                    Some(_) => bail!("Codex {root} {field} dependent default is not null"),
                }
            }
            if object.is_empty() {
                config.remove(root);
            }
        }
        Some(_) => bail!("Codex {root} config is not an object"),
    }
    Ok(())
}

fn codex_default_config_digest_is_allowed(digest: &str) -> bool {
    if digest == CODEX_DEFAULT_CONFIG_SHA256 {
        return true;
    }
    #[cfg(test)]
    {
        // Fake app-server fixtures intentionally project only policy-owned fields.
        digest == "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    }
    #[cfg(not(test))]
    {
        false
    }
}

fn named_mcp_routes(config: &BTreeMap<String, JsonValue>) -> Result<BTreeMap<String, McpRoute>> {
    let Some(entries) = optional_object(config, "mcp_servers", "Codex MCP servers")? else {
        return Ok(BTreeMap::new());
    };
    entries
        .iter()
        .map(|(name, value)| {
            ensure!(
                !name.is_empty() && !name.contains(['\r', '\n', '\0']),
                "Codex MCP servers contains an invalid name"
            );
            let entry = json_object(value, "Codex MCP server")?;
            let enabled = match entry.get("enabled") {
                Some(JsonValue::Bool(enabled)) => *enabled,
                Some(_) => bail!("Codex MCP server enabled state is not Boolean"),
                None => true,
            };
            let command = optional_config_string(entry, "command", "Codex MCP command")?;
            let args = match entry.get("args") {
                None | Some(JsonValue::Null) => None,
                Some(value) => Some(
                    json_string_array(value, "Codex MCP arguments")?
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                ),
            };
            let env = optional_object(entry, "env", "Codex MCP environment")?;
            let env_keys = env
                .into_iter()
                .flat_map(|environment| environment.keys().cloned())
                .collect();
            let env_values_empty = env.is_none_or(|environment| {
                environment
                    .values()
                    .all(|value| matches!(value, JsonValue::String(value) if value.is_empty()))
            });
            let env_vars_empty = null_or_empty_array(entry.get("env_vars"));
            Ok((
                name.clone(),
                McpRoute {
                    enabled,
                    command,
                    args,
                    fields: entry.keys().cloned().collect(),
                    env_keys,
                    env_values_empty,
                    env_vars_empty,
                    environment_id: optional_config_string(
                        entry,
                        "environment_id",
                        "Codex MCP environment identity",
                    )?,
                    startup_timeout_seconds: optional_u64(
                        entry,
                        "startup_timeout_sec",
                        "Codex MCP startup timeout",
                    )?,
                    tool_timeout_seconds: optional_u64(
                        entry,
                        "tool_timeout_sec",
                        "Codex MCP tool timeout",
                    )?,
                },
            ))
        })
        .collect()
}

fn null_or_empty_array(value: Option<&JsonValue>) -> bool {
    value.is_none_or(|value| match value {
        JsonValue::Null => true,
        JsonValue::Array(values) => values.is_empty(),
        _ => false,
    })
}

fn optional_u64(
    object: &BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<Option<u64>> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => json_u64(value, label).map(Some),
    }
}

fn named_enablement(
    config: &BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<BTreeMap<String, bool>> {
    let Some(entries) = optional_object(config, field, label)? else {
        return Ok(BTreeMap::new());
    };
    entries
        .iter()
        .map(|(name, value)| {
            ensure!(
                !name.is_empty() && !name.contains(['\r', '\n', '\0']),
                "{label} contains an invalid name"
            );
            let entry = json_object(value, label)?;
            let enabled = match entry.get("enabled") {
                Some(JsonValue::Bool(enabled)) => *enabled,
                Some(_) => bail!("{label} enabled state is not Boolean"),
                None => true,
            };
            Ok((name.clone(), enabled))
        })
        .collect()
}

fn optional_object<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<Option<&'a BTreeMap<String, JsonValue>>> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => json_object(value, label).map(Some),
    }
}

fn optional_config_string(
    object: &BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<Option<String>> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => Ok(Some(json_string(value, label)?.to_string())),
    }
}

fn optional_bool(
    object: Option<&BTreeMap<String, JsonValue>>,
    field: &str,
    label: &str,
) -> Result<Option<bool>> {
    match object.and_then(|object| object.get(field)) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => bail!("{label} {field} is not Boolean"),
    }
}

pub(super) fn fixed_safety_overrides() -> Vec<String> {
    vec![
        "model_provider=\"openai\"".to_string(),
        format!("openai_base_url=\"{OFFICIAL_CHATGPT_BASE_URL}\""),
        format!("chatgpt_base_url=\"{OFFICIAL_CHATGPT_BASE_URL}\""),
        format!("model=\"{MODEL}\""),
        format!("model_reasoning_effort=\"{REASONING}\""),
        "service_tier=\"default\"".to_string(),
        "notify=[]".to_string(),
        "memories.generate_memories=false".to_string(),
        "memories.use_memories=false".to_string(),
        "features.memories=false".to_string(),
    ]
}

pub(super) fn safety_overrides(ambient: &EffectiveConfig, inert_mcp: &Path) -> Result<Vec<String>> {
    ensure!(
        ambient.model_providers.is_empty(),
        "custom Codex model-provider definitions cannot be made hermetic"
    );
    require_inert_mcp_path(inert_mcp)?;
    let mut overrides = fixed_safety_overrides();
    let mut dynamic = Vec::new();
    for (name, route) in &ambient.mcp_servers {
        require_config_name(name)?;
        let allowed_fields: BTreeSet<String> = [
            "args",
            "command",
            "enabled",
            "env",
            "env_vars",
            "environment_id",
            "startup_timeout_sec",
            "tool_timeout_sec",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();
        ensure!(
            route.command.is_some() && route.fields.is_subset(&allowed_fields),
            "ambient Codex MCP route is not a supported local-command transport"
        );
        dynamic.push(format!("mcp_servers.{name}.args=[]"));
        dynamic.push(format!(
            "mcp_servers.{name}.command={}",
            quote_toml_string(
                inert_mcp
                    .to_str()
                    .context("inert Codex MCP path is not UTF-8")?
            )
        ));
        dynamic.push(format!("mcp_servers.{name}.enabled=false"));
        for variable in &route.env_keys {
            require_config_name(variable)?;
            dynamic.push(format!("mcp_servers.{name}.env.{variable}=\"\""));
        }
        dynamic.push(format!("mcp_servers.{name}.env_vars=[]"));
        dynamic.push(format!(
            "mcp_servers.{name}.environment_id=\"{INERT_MCP_ENVIRONMENT}\""
        ));
        dynamic.push(format!(
            "mcp_servers.{name}.startup_timeout_sec={INERT_MCP_TIMEOUT_SECONDS}"
        ));
        dynamic.push(format!(
            "mcp_servers.{name}.tool_timeout_sec={INERT_MCP_TIMEOUT_SECONDS}"
        ));
    }
    for name in ambient.plugins.keys() {
        require_config_name(name)?;
        dynamic.push(format!("plugins.{name}.enabled=false"));
    }
    dynamic.sort();
    overrides.extend(dynamic);
    overrides.push(WINDOWS_SANDBOX_OVERRIDE.to_string());
    require_wrapper_override_policy(&overrides)?;
    Ok(overrides)
}

pub(super) fn require_safe_ambient_config(ambient: &EffectiveConfig) -> Result<()> {
    ensure!(
        ambient.origins.is_empty(),
        "private empty Codex config has an ambient origin"
    );
    ensure!(
        ambient.notify_is_empty,
        "private empty Codex config has active notification commands"
    );
    Ok(())
}

pub(super) fn require_safe_effective_config(
    effective: &EffectiveConfig,
    ambient: &EffectiveConfig,
    inert_mcp: &Path,
) -> Result<()> {
    let mut authorized_origins: BTreeSet<String> = [
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
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    for (name, route) in &ambient.mcp_servers {
        for field in [
            "args",
            "command",
            "enabled",
            "env_vars",
            "environment_id",
            "startup_timeout_sec",
            "tool_timeout_sec",
        ] {
            authorized_origins.insert(format!("mcp_servers.{name}.{field}"));
        }
        for variable in &route.env_keys {
            authorized_origins.insert(format!("mcp_servers.{name}.env.{variable}"));
        }
    }
    for name in ambient.plugins.keys() {
        authorized_origins.insert(format!("plugins.{name}.enabled"));
    }
    ensure!(
        effective.origins.is_subset(&authorized_origins),
        "effective Codex config has an origin not authorized by the safety overrides"
    );
    ensure!(
        effective.model.as_deref() == Some(MODEL)
            && effective.model_provider.as_deref() == Some("openai")
            && effective.openai_base_url.as_deref() == Some(OFFICIAL_CHATGPT_BASE_URL)
            && effective.chatgpt_base_url.as_deref() == Some(OFFICIAL_CHATGPT_BASE_URL)
            && effective.reasoning.as_deref() == Some(REASONING)
            && effective.service_tier.as_deref() == Some("default"),
        "effective Codex model route differs from the hermetic ChatGPT policy"
    );
    ensure!(
        effective.notify_is_empty
            && effective.memories_generate == Some(false)
            && effective.memories_use == Some(false)
            && effective.feature_memories == Some(false),
        "effective Codex side-effect features are not disabled"
    );
    ensure!(
        effective.windows_sandbox.as_deref() == Some("mxc"),
        "effective Codex Windows sandbox is not the required MXC backend"
    );
    ensure!(
        effective.model_providers.is_empty(),
        "effective Codex config contains custom provider definitions"
    );
    ensure!(
        effective.mcp_servers.keys().eq(ambient.mcp_servers.keys())
            && effective.plugins.keys().eq(ambient.plugins.keys())
            && effective.plugins.values().all(|enabled| !enabled),
        "effective Codex config has an enabled or unexpected MCP/plugin route"
    );
    let expected_mcp_fields: BTreeSet<String> = [
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
    .collect();
    for (name, route) in &effective.mcp_servers {
        let ambient_route = ambient
            .mcp_servers
            .get(name)
            .context("effective Codex MCP route is unexpected")?;
        ensure!(
            !route.enabled
                && route.command.as_deref() == inert_mcp.to_str()
                && route.args.as_deref() == Some(&[])
                && route.fields == expected_mcp_fields
                && route.env_keys == ambient_route.env_keys
                && route.env_values_empty
                && route.env_vars_empty
                && route.environment_id.as_deref() == Some(INERT_MCP_ENVIRONMENT)
                && route.startup_timeout_seconds == Some(INERT_MCP_TIMEOUT_SECONDS)
                && route.tool_timeout_seconds == Some(INERT_MCP_TIMEOUT_SECONDS),
            "effective Codex MCP route is not the exact inert disabled transport"
        );
    }
    require_inert_mcp_path(inert_mcp)?;
    for field in [
        "model_provider",
        "openai_base_url",
        "chatgpt_base_url",
        "model",
        "model_reasoning_effort",
        "service_tier",
        "memories.generate_memories",
        "memories.use_memories",
        "features.memories",
        "windows.sandbox",
    ] {
        ensure!(
            effective.origins.contains(field),
            "effective Codex config has no origin for safety field {field}"
        );
    }
    for (name, route) in &ambient.mcp_servers {
        for field in [
            "command",
            "enabled",
            "environment_id",
            "startup_timeout_sec",
            "tool_timeout_sec",
        ] {
            ensure!(
                effective
                    .origins
                    .contains(&format!("mcp_servers.{name}.{field}")),
                "effective Codex MCP inert field has no origin"
            );
        }
        for variable in &route.env_keys {
            ensure!(
                effective
                    .origins
                    .contains(&format!("mcp_servers.{name}.env.{variable}")),
                "effective Codex MCP environment neutralizer has no origin"
            );
        }
    }
    for name in ambient.plugins.keys() {
        ensure!(
            effective
                .origins
                .contains(&format!("plugins.{name}.enabled")),
            "effective Codex plugin disable has no origin"
        );
    }
    Ok(())
}

pub(super) fn require_config_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'@') }),
        "Codex config name cannot be represented as a safe CLI key segment"
    );
    Ok(())
}

pub(super) fn quote_toml_string(value: &str) -> String {
    let mut quoted = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            character if character <= '\u{001f}' => {
                quoted.push_str(&format!("\\u{:04x}", u32::from(character)));
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

pub(super) fn parse_toml_string(value: &str) -> Result<String> {
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .context("TOML string is not quoted")?;
    let mut characters = inner.chars();
    let mut name = String::new();
    while let Some(character) = characters.next() {
        if character != '\\' {
            ensure!(
                character > '\u{001f}' && character != '"',
                "TOML string has an unescaped character"
            );
            name.push(character);
            continue;
        }
        match characters
            .next()
            .context("TOML string has a trailing escape")?
        {
            '"' => name.push('"'),
            '\\' => name.push('\\'),
            'u' => {
                let mut hexadecimal = String::new();
                for _ in 0..4 {
                    hexadecimal.push(
                        characters
                            .next()
                            .context("TOML string has a short Unicode escape")?,
                    );
                }
                ensure!(
                    hexadecimal.bytes().all(|byte| byte.is_ascii_hexdigit()),
                    "TOML string has an invalid Unicode escape"
                );
                let scalar = u32::from_str_radix(&hexadecimal, 16)
                    .context("parsing TOML string Unicode escape")?;
                name.push(
                    char::from_u32(scalar).context("TOML string Unicode escape is not a scalar")?,
                );
            }
            _ => bail!("TOML string has an unsupported escape"),
        }
    }
    Ok(name)
}

pub(super) fn inert_mcp_executable(scratch_root: &Path) -> Result<PathBuf> {
    let root = canonical_directory(scratch_root, "owned Codex Security scratch")?;
    let path = root.join(INERT_MCP_EXECUTABLE);
    require_inert_mcp_path(&path)?;
    Ok(path)
}

pub(super) fn require_inert_mcp_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && path.file_name().and_then(OsStr::to_str) == Some(INERT_MCP_EXECUTABLE),
        "inert Codex MCP executable path differs"
    );
    let parent = path
        .parent()
        .context("inert Codex MCP executable has no parent")?;
    ensure!(
        canonical_directory(parent, "inert Codex MCP parent")? == parent
            && parent.file_name().and_then(OsStr::to_str) == Some(OWNED_SCRATCH_NAME),
        "inert Codex MCP executable is outside the owned scratch"
    );
    require_absent(path, "inert Codex MCP executable")
}

pub(super) fn require_absent(path: &Path, label: &str) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => bail!("{label} unexpectedly exists at {}", path.display()),
        Err(error) => Err(error).with_context(|| format!("inspecting {label} {}", path.display())),
    }
}

pub(super) fn require_mirror_exact(root: &Path, package: &PluginPackage) -> Result<()> {
    let mut expected_files = BTreeSet::from([PathBuf::from("integrity.json")]);
    let mut expected_directories = BTreeSet::new();
    for file in &package.files {
        expected_files.insert(file.relative.clone());
        let mut parent = file.relative.parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            expected_directories.insert(directory.to_path_buf());
            parent = directory.parent();
        }
        let bytes = read(
            &root.join(&file.relative),
            MAX_PLUGIN_FILE_BYTES,
            "private Codex Security package file",
        )?;
        ensure!(
            sha256::hex(&sha256::digest(&bytes)) == file.sha256,
            "private Codex Security package file {} changed",
            file.slash_path
        );
    }
    let integrity = read(
        &root.join("integrity.json"),
        MAX_PLUGIN_MANIFEST_BYTES,
        "private Codex Security integrity manifest",
    )?;
    ensure!(
        integrity == package.integrity,
        "private Codex Security integrity manifest changed"
    );
    let (actual_files, actual_directories) = mirror_entries(root)?;
    ensure!(
        actual_files == expected_files && actual_directories == expected_directories,
        "private Codex Security mirror gained an unlisted entry"
    );
    Ok(())
}

fn mirror_entries(root: &Path) -> Result<(BTreeSet<PathBuf>, BTreeSet<PathBuf>)> {
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    collect_mirror_entries(root, root, 0, &mut files, &mut directories)?;
    Ok((files, directories))
}

fn collect_mirror_entries(
    root: &Path,
    directory: &Path,
    depth: usize,
    files: &mut BTreeSet<PathBuf>,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    ensure!(
        depth <= MAX_SCRATCH_DEPTH,
        "plugin mirror nesting is excessive"
    );
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("listing plugin mirror {}", directory.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", directory.display()))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("inspecting plugin mirror entry {}", path.display()))?;
        let relative = path
            .strip_prefix(root)
            .context("plugin mirror entry escaped its root")?
            .to_path_buf();
        if crate::mutate::workspace::metadata_is_real_directory(&metadata) {
            directories.insert(relative);
            collect_mirror_entries(root, &path, depth + 1, files, directories)?;
        } else {
            ensure!(
                metadata.file_type().is_file()
                    && !metadata.file_type().is_symlink()
                    && !metadata_is_reparse_point(&metadata),
                "plugin mirror contains a redirected or special entry"
            );
            files.insert(relative);
        }
    }
    Ok(())
}

pub(super) fn observe_scratch(root: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)
        .with_context(|| format!("inspecting Codex Security scratch {}", root.display()))?;
    ensure!(
        crate::mutate::workspace::metadata_is_real_directory(&metadata),
        "Codex Security scratch root was redirected"
    );
    let mut entries = 0u64;
    let mut bytes = 0u64;
    measure_scratch(root, 0, &mut entries, &mut bytes)?;
    ensure!(
        entries <= MAX_SCRATCH_ENTRIES,
        "Codex Security scratch exceeds its {MAX_SCRATCH_ENTRIES}-entry limit"
    );
    ensure!(
        bytes <= MAX_SCRATCH_BYTES,
        "Codex Security scratch exceeds its {MAX_SCRATCH_BYTES}-byte limit"
    );
    Ok(())
}

pub(super) fn measure_scratch(
    directory: &Path,
    depth: usize,
    entries: &mut u64,
    bytes: &mut u64,
) -> Result<()> {
    ensure!(
        depth <= MAX_SCRATCH_DEPTH,
        "Codex Security scratch exceeds its nesting limit"
    );
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("listing Codex Security scratch {}", directory.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", directory.display()))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).with_context(|| {
            format!("inspecting Codex Security scratch entry {}", path.display())
        })?;
        *entries = entries
            .checked_add(1)
            .context("Codex Security scratch entry count overflowed")?;
        ensure!(
            *entries <= MAX_SCRATCH_ENTRIES,
            "Codex Security scratch exceeds its entry limit"
        );
        if crate::mutate::workspace::metadata_is_real_directory(&metadata) {
            measure_scratch(&path, depth + 1, entries, bytes)?;
        } else {
            ensure!(
                metadata.file_type().is_file()
                    && !metadata.file_type().is_symlink()
                    && !metadata_is_reparse_point(&metadata),
                "Codex Security scratch contains a redirected or special entry"
            );
            *bytes = bytes
                .checked_add(metadata.len())
                .context("Codex Security scratch byte count overflowed")?;
            ensure!(
                *bytes <= MAX_SCRATCH_BYTES,
                "Codex Security scratch exceeds its byte limit"
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub(super) const fn metadata_is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

pub(super) fn remove_scratch(scratch: &OwnedScratch) -> Result<()> {
    require_scratch_cleanup_target(&scratch.root)?;
    let metadata = std::fs::symlink_metadata(&scratch.root).with_context(|| {
        format!(
            "inspecting Codex Security scratch {}",
            scratch.root.display()
        )
    })?;
    ensure!(
        crate::mutate::workspace::metadata_is_real_directory(&metadata),
        "refusing to remove redirected Codex Security scratch"
    );
    remove_entry_no_follow(&scratch.root, &metadata)
}

pub(super) fn require_scratch_cleanup_target(root: &Path) -> Result<()> {
    let temporary_parent =
        canonical_directory(&std::env::temp_dir(), "Codex Security temporary parent")?;
    ensure!(
        root == temporary_parent.join(OWNED_SCRATCH_NAME),
        "refusing to remove an unexpected Codex Security scratch path"
    );
    Ok(())
}

pub(super) fn remove_entry_no_follow(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    if crate::mutate::workspace::metadata_is_real_directory(metadata) {
        for entry in std::fs::read_dir(path)
            .with_context(|| format!("listing cleanup directory {}", path.display()))?
        {
            let entry =
                entry.with_context(|| format!("reading cleanup entry in {}", path.display()))?;
            let child = entry.path();
            let child_metadata = std::fs::symlink_metadata(&child)
                .with_context(|| format!("inspecting cleanup entry {}", child.display()))?;
            remove_entry_no_follow(&child, &child_metadata)?;
        }
        std::fs::remove_dir(path)
            .with_context(|| format!("removing Codex Security directory {}", path.display()))
    } else {
        ensure!(
            metadata.file_type().is_file()
                && !metadata.file_type().is_symlink()
                && !metadata_is_reparse_point(metadata),
            "refusing to remove redirected or special Codex Security scratch entry {}",
            path.display()
        );
        std::fs::remove_file(path)
            .with_context(|| format!("removing Codex Security file {}", path.display()))
    }
}
