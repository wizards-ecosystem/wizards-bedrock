use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct McpOutcome {
    pub(super) scan_id: String,
    pub(super) scan_directory: PathBuf,
    pub(super) thread_ids: BTreeSet<String>,
    pub(super) execution_threads: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum McpReply {
    Success(JsonValue),
    Error(JsonValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaybreakStatus {
    Granted,
    NotGranted,
    Unknown,
}

impl DaybreakStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::NotGranted => "not_granted",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DaybreakAdvisory {
    status: DaybreakStatus,
    programs: Vec<String>,
    enrollment_url: Option<String>,
    stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DaybreakPresentation {
    Standard(String),
    Warning(String),
}

pub(super) fn run_mcp(invocation: OwnedMcpInvocation<'_>) -> Result<McpOutcome> {
    let OwnedMcpInvocation {
        source,
        scratch,
        worker_policy,
        tools,
        git,
        overrides,
        shell_environment,
    } = invocation;
    let repository = canonical_directory(&worker_policy.repository, "Codex Security target")?;
    ensure!(
        worker_policy.projected_auth == scratch.codex_home().join("auth.json")
            && worker_policy.live_auth != worker_policy.projected_auth,
        "Codex Security worker credential deny targets differ"
    );
    let mut command = Command::new(&tools.node.path);
    command
        .arg(scratch.plugin.join("mcp").join("server.mjs"))
        .arg("--stdio")
        .current_dir(&scratch.plugin);
    apply_base_environment(&mut command, scratch, Some((tools, git)))?;
    shell_environment.apply(&mut command);
    apply_wrapper_environment(
        &mut command,
        source,
        tools,
        worker_policy,
        &repository,
        scratch,
        overrides,
    )?;

    let dialogue_worker_policy = worker_policy.clone();
    let dialogue_source = source.clone();
    let dialogue_scan_root = scratch.scan_root.clone();
    let observer_root = scratch.root.clone();
    let mut last_observation = None;
    require_private_credential_projection(&scratch.codex_home())?;
    let output = crate::bounded_process::output_dialogue(
        command,
        DEEP_TIMEOUT,
        "owned Codex Security MCP server",
        DialogueLimits {
            line_bytes: MAX_MCP_LINE_BYTES,
            stdout_bytes: MAX_MCP_STDOUT_BYTES,
            stderr_bytes: MAX_MCP_STDERR_BYTES,
        },
        move || {
            if last_observation.is_none_or(|last: Instant| last.elapsed() >= OBSERVE_INTERVAL) {
                observe_scratch(&observer_root)?;
                last_observation = Some(Instant::now());
            }
            Ok(())
        },
        move |dialogue| {
            exchange_mcp(
                dialogue,
                &dialogue_worker_policy,
                &dialogue_source,
                &dialogue_scan_root,
            )
        },
    )?;
    require_private_credential_projection(&scratch.codex_home())?;
    ensure!(
        output.status.success(),
        "owned Codex Security MCP server failed ({})",
        output.status
    );
    ensure!(
        std::str::from_utf8(&output.stderr).is_ok(),
        "owned Codex Security diagnostics are not UTF-8"
    );
    Ok(output.value)
}

pub(super) fn exchange_mcp(
    dialogue: &mut LineDialogue,
    worker_policy: &ManagedWorkerPolicy,
    source: &SourceIdentity,
    scan_root: &Path,
) -> Result<McpOutcome> {
    initialize_mcp(dialogue)?;
    require_mcp_tools(dialogue)?;
    call_daybreak_advisory(dialogue, worker_policy, source)?;
    let start = call_start(dialogue, worker_policy, source, scan_root)?;
    let outcome = call_complete(dialogue, worker_policy, source, scan_root, &start)?;
    dialogue.close_input();
    ensure!(
        dialogue.receive_line()?.is_none(),
        "Codex Security MCP server sent output after its final completion response"
    );
    Ok(outcome)
}

pub(super) fn apply_wrapper_environment(
    command: &mut Command,
    source: &SourceIdentity,
    tools: &Toolchain,
    worker_policy: &ManagedWorkerPolicy,
    repository: &Path,
    scratch: &OwnedScratch,
    overrides: &[String],
) -> Result<()> {
    require_wrapper_override_policy(overrides)?;
    let wrapper = std::env::current_exe().context("resolving the private Codex wrapper")?;
    let wrapper = std::fs::canonicalize(&wrapper)
        .with_context(|| format!("canonicalizing private Codex wrapper {}", wrapper.display()))?;
    ensure!(
        wrapper != tools.codex.path && wrapper != tools.python.path,
        "private Codex wrapper resolves to the real Codex executable"
    );
    let serialized = render_json(&JsonValue::Array(
        overrides.iter().cloned().map(JsonValue::String).collect(),
    ))?;
    let python_runtime = render_python_runtime_policy(&tools.python_runtime)?;
    let provider_permission = provider_named_permission_profile(worker_policy)?;
    let worker_permission = effective_worker_named_permission_profile(worker_policy)?;
    command
        .env("CODEX_CLI_PATH", &wrapper)
        .env(
            WRAPPER_MARKER_ENV,
            format!("{WRAPPER_MARKER_PREFIX}{}", source.commit),
        )
        .env(WRAPPER_REAL_ENV, &tools.codex.path)
        .env(WRAPPER_OVERRIDES_ENV, serialized)
        .env(WRAPPER_REPOSITORY_ENV, repository)
        .env(WRAPPER_NODE_ENV, &tools.node.path)
        .env(WRAPPER_LIVE_AUTH_ENV, &worker_policy.live_auth)
        .env(WRAPPER_PROVIDER_PERMISSION_ENV, provider_permission)
        .env(WRAPPER_WORKER_PERMISSION_ENV, worker_permission)
        .env(WRAPPER_TRANSITIONS_ENV, &scratch.transitions)
        .env("PYTHON", &wrapper)
        .env(PYTHON_WRAPPER_REAL_ENV, &tools.python.path)
        .env(PYTHON_WRAPPER_PLUGIN_ENV, &scratch.plugin)
        .env(PYTHON_WRAPPER_PYCACHE_ENV, &scratch.pycache)
        .env(PYTHON_WRAPPER_RUNTIME_ENV, python_runtime);
    Ok(())
}

fn initialize_mcp(dialogue: &mut LineDialogue) -> Result<()> {
    let params = object([
        (
            "capabilities",
            object(std::iter::empty::<(&str, JsonValue)>()),
        ),
        (
            "clientInfo",
            object([
                ("name", text_value("wizard-bedrock-xtask")),
                ("version", text_value("1")),
            ]),
        ),
        ("protocolVersion", text_value(MCP_PROTOCOL)),
    ]);
    send_request(dialogue, "initialize", "initialize", params)?;
    let result = receive_response(dialogue, "initialize")?;
    let initialized = json_object(&result, "MCP initialize result")?;
    let rendered = render_json(&result)?;
    ensure!(
        rendered.len() == MCP_INITIALIZE_BYTES
            && sha256::hex(&sha256::digest(rendered.as_bytes())) == MCP_INITIALIZE_SHA256,
        "Codex Security MCP initialize schema differs from the qualified provider"
    );
    require_json_fields(
        initialized,
        &["capabilities", "protocolVersion", "serverInfo"],
        "MCP initialize result",
    )?;
    let protocol = string(initialized, "protocolVersion", "MCP initialize result")?;
    ensure!(
        protocol == MCP_PROTOCOL,
        "Codex Security negotiated an unqualified MCP protocol"
    );
    let server = json_object(
        json_field(initialized, "serverInfo", "MCP initialize result")?,
        "MCP server identity",
    )?;
    ensure!(
        string(server, "name", "MCP server identity")? == MCP_SERVER_NAME
            && string(server, "version", "MCP server identity")? == MCP_SERVER_VERSION,
        "Codex Security MCP server identity differs from the qualified provider"
    );
    let notification = object([
        ("jsonrpc", text_value("2.0")),
        ("method", text_value("notifications/initialized")),
    ]);
    send_value(dialogue, &notification)
}

fn require_mcp_tools(dialogue: &mut LineDialogue) -> Result<()> {
    send_request(
        dialogue,
        "tools",
        "tools/list",
        object(std::iter::empty::<(&str, JsonValue)>()),
    )?;
    let result = receive_response(dialogue, "tools")?;
    let listed = json_object(&result, "MCP tools result")?;
    require_allowed_fields(
        listed,
        &["tools", "nextCursor", "_meta"],
        "MCP tools result",
    )?;
    if let Some(cursor) = listed.get("nextCursor") {
        ensure!(
            matches!(cursor, JsonValue::Null),
            "Codex Security MCP tool list is unexpectedly paginated"
        );
    }
    let tools = json_array(
        json_field(listed, "tools", "MCP tools result")?,
        "MCP tool list",
    )?;
    require_exact_mcp_tools(tools)
}

pub(super) fn require_exact_mcp_tools(tools: &[JsonValue]) -> Result<()> {
    let descriptors = tools.iter().map(render_json).collect::<Result<Vec<_>>>()?;
    let rendered = format!("[{}]", descriptors.join(","));
    ensure!(
        tools.len() == MCP_TOOLS_COUNT
            && rendered.len() == MCP_TOOLS_BYTES
            && sha256::hex(&sha256::digest(rendered.as_bytes())) == MCP_TOOLS_SHA256,
        "Codex Security MCP tool inventory differs from the qualified provider"
    );
    let mut start = 0usize;
    let mut complete = 0usize;
    let mut daybreak = 0usize;
    for (tool, rendered) in tools.iter().zip(&descriptors) {
        let tool = json_object(tool, "MCP tool")?;
        let name = string(tool, "name", "MCP tool")?;
        if name == START_TOOL {
            start += 1;
            require_tool_descriptor(
                tool,
                rendered,
                START_TOOL_BYTES,
                START_TOOL_SHA256,
                START_TOOL,
            )?;
        } else if name == COMPLETE_TOOL {
            complete += 1;
            require_tool_descriptor(
                tool,
                rendered,
                COMPLETE_TOOL_BYTES,
                COMPLETE_TOOL_SHA256,
                COMPLETE_TOOL,
            )?;
        } else if name == DAYBREAK_TOOL {
            daybreak += 1;
            require_tool_descriptor(
                tool,
                rendered,
                DAYBREAK_TOOL_BYTES,
                DAYBREAK_TOOL_SHA256,
                DAYBREAK_TOOL,
            )?;
        }
    }
    ensure!(
        start == 1 && complete == 1 && daybreak == 1,
        "Codex Security MCP tool authority differs from the qualified workflow"
    );
    Ok(())
}

fn require_tool_descriptor(
    tool: &BTreeMap<String, JsonValue>,
    rendered: &str,
    expected_bytes: usize,
    expected_sha256: &str,
    name: &str,
) -> Result<()> {
    require_json_fields(
        tool,
        &[
            "_meta",
            "annotations",
            "description",
            "execution",
            "inputSchema",
            "name",
            "title",
        ],
        "Codex Security MCP tool descriptor",
    )?;
    ensure!(
        rendered.len() == expected_bytes
            && sha256::hex(&sha256::digest(rendered.as_bytes())) == expected_sha256,
        "Codex Security MCP tool descriptor for {name} differs"
    );
    Ok(())
}

#[cfg(test)]
pub(super) fn pinned_mcp_initialize_for_test() -> JsonValue {
    parse_json(
        br#"{"capabilities":{"experimental":{"codex/sandbox-state-meta":{}},"extensions":{"com.openai":{}},"logging":{},"tools":{"listChanged":true}},"protocolVersion":"2025-06-18","serverInfo":{"name":"codex-security","version":"0.1.160"}}"#,
        "pinned MCP initialize fixture",
    )
    .expect("pinned MCP initialize fixture parses")
}

#[cfg(test)]
pub(super) fn pinned_mcp_tool_inventory_for_test() -> Vec<JsonValue> {
    let rendered = include_str!("mcp_tools_0_1_32.json").trim_end();
    assert_eq!(rendered.len(), MCP_TOOLS_BYTES);
    assert_eq!(
        sha256::hex(&sha256::digest(rendered.as_bytes())),
        MCP_TOOLS_SHA256
    );
    let value = parse_json(rendered.as_bytes(), "pinned MCP tool inventory fixture")
        .expect("pinned MCP tool inventory parses");
    let JsonValue::Array(tools) = value else {
        panic!("pinned MCP tool inventory is not an array")
    };
    tools
}

fn call_daybreak_advisory(
    dialogue: &mut LineDialogue,
    worker_policy: &ManagedWorkerPolicy,
    source: &SourceIdentity,
) -> Result<()> {
    let params = object([
        ("_meta", call_metadata(worker_policy, source)?),
        ("arguments", object(std::iter::empty::<(&str, JsonValue)>())),
        ("name", text_value(DAYBREAK_TOOL)),
    ]);
    send_request(dialogue, "daybreak", "tools/call", params)?;
    let McpReply::Success(result) = receive_mcp_reply(dialogue, "daybreak")? else {
        return Ok(());
    };
    let Ok(advisory) = parse_daybreak_advisory(&result) else {
        return Ok(());
    };
    present_daybreak_advisory(&advisory);
    Ok(())
}

pub(super) fn parse_daybreak_advisory(value: &JsonValue) -> Result<DaybreakAdvisory> {
    let result = require_tool_success(value, "Codex Security Daybreak advisory")?;
    let structured = json_object(
        json_field(
            result,
            "structuredContent",
            "Codex Security Daybreak advisory",
        )?,
        "Codex Security Daybreak advisory result",
    )?;
    require_allowed_fields(
        structured,
        &[
            "schemaVersion",
            "status",
            "programs",
            "enrollmentUrl",
            "checkedAt",
            "stale",
        ],
        "Codex Security Daybreak advisory result",
    )?;
    ensure!(
        json_u64(
            json_field(
                structured,
                "schemaVersion",
                "Codex Security Daybreak advisory result",
            )?,
            "Codex Security Daybreak advisory schema version",
        )? == 1,
        "Codex Security Daybreak advisory schema differs"
    );
    let status = match string(
        structured,
        "status",
        "Codex Security Daybreak advisory result",
    )? {
        "granted" => DaybreakStatus::Granted,
        "not_granted" => DaybreakStatus::NotGranted,
        "unknown" => DaybreakStatus::Unknown,
        _ => bail!("Codex Security Daybreak advisory status differs"),
    };
    let program_values = json_array(
        json_field(
            structured,
            "programs",
            "Codex Security Daybreak advisory result",
        )?,
        "Codex Security Daybreak programs",
    )?;
    ensure!(
        program_values.len() <= 2,
        "Codex Security Daybreak program list differs"
    );
    let mut unique = BTreeSet::new();
    let mut programs = Vec::with_capacity(program_values.len());
    for program in program_values {
        let program = json_string(program, "Codex Security Daybreak program")?;
        ensure!(
            matches!(program, "Daybreak Blue" | "Daybreak Red")
                && unique.insert(program.to_string()),
            "Codex Security Daybreak program list differs"
        );
        programs.push(program.to_string());
    }
    ensure!(
        (status == DaybreakStatus::Granted) == !programs.is_empty(),
        "Codex Security Daybreak status and programs differ"
    );
    let checked_at = nonempty_string(
        structured,
        "checkedAt",
        "Codex Security Daybreak advisory result",
    )?;
    ensure!(
        qualification::is_rfc3339_timestamp(checked_at),
        "Codex Security Daybreak check time is not RFC3339"
    );
    let stale = boolean(
        structured,
        "stale",
        "Codex Security Daybreak advisory result",
    )?;
    ensure!(
        !stale || status == DaybreakStatus::Unknown,
        "Codex Security Daybreak stale status differs"
    );
    let enrollment_url = match structured.get("enrollmentUrl") {
        Some(JsonValue::String(value)) => {
            ensure!(
                value.len() <= 2048 && !value.contains(['\r', '\n', '\0']),
                "Codex Security Daybreak enrollment URL is malformed"
            );
            Some(value.clone())
        }
        Some(_) => bail!("Codex Security Daybreak enrollment URL is not text"),
        None => None,
    };
    let programs_text = if programs.is_empty() {
        "none".to_string()
    } else {
        programs.join(", ")
    };
    let suffix = if status == DaybreakStatus::NotGranted {
        DAYBREAK_NOT_GRANTED_SUFFIX
    } else {
        ""
    };
    let expected = format!(
        "Codex Security Daybreak access: status={}; programs={programs_text}; checkedAt={checked_at}; stale={stale}.{suffix}",
        status.as_str(),
    );
    require_single_text_content(result, &expected, "Codex Security Daybreak advisory")?;
    Ok(DaybreakAdvisory {
        status,
        programs,
        enrollment_url,
        stale,
    })
}

pub(super) fn daybreak_presentation(advisory: &DaybreakAdvisory) -> Option<DaybreakPresentation> {
    if advisory.stale || advisory.status == DaybreakStatus::Unknown {
        None
    } else if advisory.status == DaybreakStatus::Granted {
        Some(DaybreakPresentation::Standard(format!(
            "[xtask] Codex Security Daybreak access: status=granted; programs={}",
            advisory.programs.join(", ")
        )))
    } else {
        let enrollment_url = advisory
            .enrollment_url
            .as_deref()
            .filter(|url| safe_https_url(url))
            .unwrap_or(DAYBREAK_ENROLLMENT_FALLBACK)
            .to_string();
        Some(DaybreakPresentation::Warning(format!(
            "[xtask] WARNING: Codex Security Daybreak access is not granted for this ChatGPT account; protected scan outputs may not be displayable. Apply at {enrollment_url}"
        )))
    }
}

fn safe_https_url(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 2048
        || !value.is_ascii()
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ' || byte == b'\\')
    {
        return false;
    }
    let Some(remainder) = value.strip_prefix("https://") else {
        return false;
    };
    let boundary = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let (authority, suffix) = remainder.split_at(boundary);
    safe_https_authority(authority) && safe_https_suffix(suffix)
}

fn safe_https_authority(authority: &str) -> bool {
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let mut parts = authority.split(':');
    let host = parts.next().unwrap_or_default();
    let port = parts.next();
    if parts.next().is_some()
        || port.is_some_and(|port| {
            port.is_empty()
                || !port.bytes().all(|byte| byte.is_ascii_digit())
                || port.parse::<u16>().is_err()
        })
        || host.len() > 253
    {
        return false;
    }
    host.split('.').all(|label| {
        (1..=63).contains(&label.len())
            && label
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn safe_https_suffix(suffix: &str) -> bool {
    let bytes = suffix.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
            continue;
        }
        if !(byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'.'
                    | b'_'
                    | b'~'
                    | b':'
                    | b'/'
                    | b'?'
                    | b'#'
                    | b'['
                    | b']'
                    | b'@'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'\''
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b';'
                    | b'='
            ))
        {
            return false;
        }
        index += 1;
    }
    true
}

pub(super) fn write_daybreak_presentation(
    presentation: Option<&DaybreakPresentation>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> std::io::Result<()> {
    match presentation {
        None => Ok(()),
        Some(DaybreakPresentation::Standard(line)) => writeln!(stdout, "{line}"),
        Some(DaybreakPresentation::Warning(line)) => writeln!(stderr, "{line}"),
    }
}

pub(super) fn present_daybreak_with_writers(
    presentation: Option<&DaybreakPresentation>,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) {
    // The advisory is operator information, not scan authority. A detached or
    // broken console must not stop the owned Deep dialogue.
    let _ignored_operator_stream_failure =
        write_daybreak_presentation(presentation, stdout, stderr);
}

fn present_daybreak_advisory(advisory: &DaybreakAdvisory) {
    let presentation = daybreak_presentation(advisory);
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    present_daybreak_with_writers(presentation.as_ref(), &mut stdout, &mut stderr);
}

pub(super) fn send_request(
    dialogue: &mut LineDialogue,
    id: &str,
    method: &str,
    params: JsonValue,
) -> Result<()> {
    send_value(
        dialogue,
        &object([
            ("id", text_value(id)),
            ("jsonrpc", text_value("2.0")),
            ("method", text_value(method)),
            ("params", params),
        ]),
    )
}

pub(super) fn send_value(dialogue: &mut LineDialogue, value: &JsonValue) -> Result<()> {
    let rendered = render_json(value)?;
    dialogue.send_line(rendered.as_bytes())
}

pub(super) fn receive_response(
    dialogue: &mut LineDialogue,
    expected_id: &str,
) -> Result<JsonValue> {
    match receive_mcp_reply(dialogue, expected_id)? {
        McpReply::Success(result) => Ok(result),
        McpReply::Error(error) => bail!(
            "Codex Security MCP request {expected_id} failed: {}",
            render_json(&error)?
        ),
    }
}

fn receive_mcp_reply(dialogue: &mut LineDialogue, expected_id: &str) -> Result<McpReply> {
    loop {
        let mut line = dialogue
            .receive_line()?
            .context("Codex Security MCP server closed before its response")?;
        ensure!(
            line.last() == Some(&b'\n')
                && line.iter().filter(|byte| **byte == b'\n').count() == 1
                && !line.contains(&b'\r'),
            "Codex Security MCP response is not strict LF framing"
        );
        line.pop();
        let value = parse_json(&line, "Codex Security MCP response")?;
        let JsonValue::Object(mut response) = value else {
            bail!("Codex Security MCP response is not an object");
        };
        ensure!(
            string(&response, "jsonrpc", "Codex Security MCP response")? == "2.0",
            "Codex Security MCP response has another protocol version"
        );
        if response.contains_key("method") {
            handle_server_message(dialogue, &response)?;
            continue;
        }
        require_allowed_fields(
            &response,
            &["jsonrpc", "id", "result", "error"],
            "Codex Security MCP response",
        )?;
        ensure!(
            string(&response, "id", "Codex Security MCP response")? == expected_id,
            "Codex Security MCP response has an unexpected ID"
        );
        ensure!(
            response.contains_key("result") ^ response.contains_key("error"),
            "Codex Security MCP response must contain exactly one result or error"
        );
        if let Some(error) = response.remove("error") {
            require_mcp_error(&error)?;
            return Ok(McpReply::Error(error));
        }
        return Ok(McpReply::Success(
            response
                .remove("result")
                .context("Codex Security MCP response has no result")?,
        ));
    }
}

fn require_mcp_error(value: &JsonValue) -> Result<()> {
    let error = json_object(value, "Codex Security MCP error")?;
    require_allowed_fields(
        error,
        &["code", "message", "data"],
        "Codex Security MCP error",
    )?;
    require_signed_json_integer(
        json_field(error, "code", "Codex Security MCP error")?,
        "Codex Security MCP error code",
    )?;
    let message = json_string(
        json_field(error, "message", "Codex Security MCP error")?,
        "Codex Security MCP error message",
    )?;
    ensure!(
        !message.is_empty() && message.chars().all(|character| !character.is_control()),
        "Codex Security MCP error message is malformed"
    );
    Ok(())
}

fn require_signed_json_integer(value: &JsonValue, label: &str) -> Result<i64> {
    let JsonValue::Number(value) = value else {
        bail!("{label} is not numeric");
    };
    let parsed = value
        .parse::<i64>()
        .with_context(|| format!("{label} is not a signed integer"))?;
    ensure!(
        parsed.to_string() == value.as_str(),
        "{label} is not canonical"
    );
    Ok(parsed)
}

fn handle_server_message(
    dialogue: &mut LineDialogue,
    message: &BTreeMap<String, JsonValue>,
) -> Result<()> {
    let method = string(message, "method", "Codex Security MCP server message")?;
    if let Some(id) = message.get("id") {
        require_json_fields(
            message,
            &["id", "jsonrpc", "method"],
            "Codex Security MCP ping",
        )?;
        ensure!(
            method == "ping",
            "Codex Security MCP server requested unsupported client action {method}"
        );
        match id {
            JsonValue::String(value) => ensure!(
                !value.is_empty() && !value.contains(['\r', '\n', '\0']),
                "Codex Security MCP ping ID is malformed"
            ),
            JsonValue::Number(_) => {
                json_u64(id, "Codex Security MCP ping ID")?;
            }
            _ => bail!("Codex Security MCP ping ID has an unsupported type"),
        }
        let id = render_json(id)?;
        let response = format!(r#"{{"id":{id},"jsonrpc":"2.0","result":{{}}}}"#);
        dialogue.send_line(response.as_bytes())
    } else {
        require_json_fields(
            message,
            &["jsonrpc", "method", "params"],
            "Codex Security MCP progress notification",
        )?;
        ensure!(
            method == "notifications/progress",
            "Codex Security MCP server sent an unsupported notification"
        );
        let params = json_object(
            json_field(
                message,
                "params",
                "Codex Security MCP progress notification",
            )?,
            "Codex Security MCP progress parameters",
        )?;
        require_allowed_fields(
            params,
            &["progressToken", "progress", "total", "message"],
            "Codex Security MCP progress parameters",
        )?;
        let token = json_field(
            params,
            "progressToken",
            "Codex Security MCP progress parameters",
        )?;
        match token {
            JsonValue::String(value) => ensure!(
                !value.is_empty() && !value.contains(['\r', '\n', '\0']),
                "Codex Security MCP progress token is malformed"
            ),
            JsonValue::Number(_) => {
                json_u64(token, "Codex Security MCP progress token")?;
            }
            _ => bail!("Codex Security MCP progress token has an unsupported type"),
        }
        require_nonnegative_json_number(
            json_field(params, "progress", "Codex Security MCP progress parameters")?,
            "Codex Security MCP progress value",
        )?;
        if let Some(total) = params.get("total") {
            require_nonnegative_json_number(total, "Codex Security MCP progress total")?;
        }
        if let Some(JsonValue::String(message)) = params.get("message") {
            ensure!(
                !message.contains(['\r', '\0']) && message.len() <= MAX_MCP_LINE_BYTES,
                "Codex Security MCP progress message is malformed"
            );
        } else if params.contains_key("message") {
            bail!("Codex Security MCP progress message is not text");
        }
        Ok(())
    }
}

fn require_nonnegative_json_number(value: &JsonValue, label: &str) -> Result<()> {
    let JsonValue::Number(value) = value else {
        bail!("{label} is not numeric");
    };
    ensure!(
        value
            .parse::<f64>()
            .is_ok_and(|number| number.is_finite() && number >= 0.0),
        "{label} is not a nonnegative finite number"
    );
    Ok(())
}

pub(super) fn require_tool_success<'a>(
    value: &'a JsonValue,
    label: &str,
) -> Result<&'a BTreeMap<String, JsonValue>> {
    let result = json_object(value, label)?;
    require_allowed_fields(
        result,
        &["content", "structuredContent", "isError", "_meta"],
        label,
    )?;
    if let Some(error) = result.get("isError") {
        ensure!(
            matches!(error, JsonValue::Bool(false)),
            "{label} returned a tool error"
        );
    }
    json_field(result, "content", label)?;
    json_field(result, "structuredContent", label)?;
    Ok(result)
}

pub(super) fn require_single_text_content(
    result: &BTreeMap<String, JsonValue>,
    expected: &str,
    label: &str,
) -> Result<()> {
    let content = json_array(json_field(result, "content", label)?, label)?;
    ensure!(content.len() == 1, "{label} content schema differs");
    let text = json_object(&content[0], label)?;
    require_json_fields(text, &["type", "text"], label)?;
    ensure!(
        string(text, "type", label)? == "text" && string(text, "text", label)? == expected,
        "{label} text differs from its structured result"
    );
    Ok(())
}

pub(super) fn require_allowed_fields(
    object: &BTreeMap<String, JsonValue>,
    allowed: &[&str],
    label: &str,
) -> Result<()> {
    ensure!(
        object.keys().all(|field| allowed.contains(&field.as_str())),
        "{label} has an unexpected field"
    );
    Ok(())
}

pub(super) fn object(entries: impl IntoIterator<Item = (&'static str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

pub(super) fn text_value(value: &str) -> JsonValue {
    JsonValue::String(value.to_string())
}

pub(super) fn path_value(value: &Path) -> JsonValue {
    text_value(&value.to_string_lossy())
}

pub(super) fn windows_file_url(path: &Path) -> Result<String> {
    let path = path
        .to_str()
        .context("Windows file URL path is not UTF-8")?;
    let normalized = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!("//{unc}")
    } else if let Some(disk) = path.strip_prefix(r"\\?\") {
        disk.to_string()
    } else {
        path.to_string()
    }
    .replace('\\', "/");
    let prefix = if normalized.starts_with("//") {
        "file:"
    } else {
        ensure!(
            normalized.as_bytes().get(1) == Some(&b':')
                && normalized.as_bytes().get(2) == Some(&b'/')
                && normalized.as_bytes()[0].is_ascii_alphabetic(),
            "Windows file URL path is not an absolute drive or UNC path"
        );
        "file:///"
    };
    let mut encoded = String::with_capacity(prefix.len() + normalized.len());
    encoded.push_str(prefix);
    for byte in normalized.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    Ok(encoded)
}

pub(super) fn boolean(
    object: &BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<bool> {
    match json_field(object, field, label)? {
        JsonValue::Bool(value) => Ok(*value),
        _ => bail!("{label} {field} is not a Boolean"),
    }
}
