use super::*;

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

const MODE_FILE: &str = ".fake-codex-config-mode";
const FAULT_FILE_PREFIX: &str = ".fault-issued-";
const CHILD_ERROR_FILE: &str = ".fake-child-error";
const READY: &[u8] = b"WIZARD-SECURITY-DEEP-FAKE-CONFIG-READY\n";

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct ReservedRoot {
    path: Option<PathBuf>,
}

impl ReservedRoot {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn transfer(mut self) -> PathBuf {
        self.path.take().expect("reserved fixture root is present")
    }
}

impl Drop for ReservedRoot {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            let _ = remove_entry_no_follow(&path, &metadata);
        }
    }
}

#[derive(Debug)]
struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    scratch: OwnedScratch,
    inert_mcp: PathBuf,
}

impl Fixture {
    fn new(mode: &str) -> Result<Self> {
        let parent = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("measuring fake Codex config fixture time")?
            .as_nanos();
        let mut selected = None;
        for attempt in 0..128_u64 {
            let candidate = parent.join(format!(
                "wizard-security-deep-config-test-{}-{nonce}-{}-{attempt}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    selected = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("creating fake Codex config root {}", candidate.display())
                    });
                }
            }
        }
        let root = std::fs::canonicalize(
            selected.context("reserving a collision-free fake Codex config root")?,
        )
        .context("canonicalizing fake Codex config root")?;
        let reserved = ReservedRoot::new(root.clone());
        let repository = root.join("repository");
        let scratch_root = root.join(OWNED_SCRATCH_NAME);
        std::fs::create_dir(&repository).context("creating fake config repository")?;
        std::fs::write(repository.join(MODE_FILE), mode.as_bytes())
            .context("writing fake config mode")?;
        for directory in [
            &scratch_root,
            &scratch_root.join("plugin"),
            &scratch_root.join("scans"),
            &scratch_root.join("state"),
            &scratch_root.join("temp"),
            &scratch_root.join("pycache"),
            &scratch_root.join("codex-state"),
        ] {
            std::fs::create_dir(directory)
                .with_context(|| format!("creating fixture directory {}", directory.display()))?;
        }
        let scratch = OwnedScratch {
            plugin: scratch_root.join("plugin"),
            scan_root: scratch_root.join("scans"),
            state: scratch_root.join("state"),
            temporary: scratch_root.join("temp"),
            pycache: scratch_root.join("pycache"),
            codex_state: scratch_root.join("codex-state"),
            root: scratch_root,
        };
        let inert_mcp = scratch.root.join(INERT_MCP_EXECUTABLE);
        Ok(Self {
            root: reserved.transfer(),
            repository,
            scratch,
            inert_mcp,
        })
    }

    fn marker(&self, mode: &str) -> PathBuf {
        self.repository.join(format!("{FAULT_FILE_PREFIX}{mode}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::symlink_metadata(&self.root) {
            let _ = remove_entry_no_follow(&self.root, &metadata);
        }
    }
}

fn fake_command() -> Result<Command> {
    let mut command = Command::new(std::env::current_exe().context("finding test executable")?);
    command.args([
        "--exact",
        "security_deep::config_tests::fake_codex_app_child",
        "--ignored",
        "--nocapture",
        "--quiet",
    ]);
    Ok(command)
}

fn fake_identity() -> Result<ToolIdentity> {
    Ok(ToolIdentity {
        path: std::env::current_exe().context("finding test executable")?,
        bytes: 1,
        sha256: "0".repeat(64),
        version: "fake Codex".to_string(),
    })
}

fn run_fake(mode: &str) -> Result<(EffectiveConfig, Fixture)> {
    let fixture = Fixture::new(mode)?;
    let mut command = fake_command()?;
    command.current_dir(&fixture.repository);
    prepare_config_test_home(&fixture.scratch)?;
    apply_base_environment(&mut command, &fixture.scratch, None)?;
    let repository = fixture.repository.clone();
    let codex_home = fixture.scratch.codex_home();
    let result = crate::bounded_process::output_dialogue(
        command,
        Duration::from_secs(30),
        "fake Codex effective-config reader",
        DialogueLimits {
            line_bytes: 4 * 1024 * 1024,
            stdout_bytes: 8 * 1024 * 1024,
            stderr_bytes: 1024 * 1024,
        },
        || Ok(()),
        move |dialogue| {
            wait_until_ready(dialogue)?;
            exchange_codex_config(dialogue, &repository, &codex_home)
        },
    )
    .and_then(|output| {
        ensure!(
            output.status.success(),
            "fake Codex effective-config reader failed ({})",
            output.status
        );
        ensure!(
            std::str::from_utf8(&output.stderr).is_ok(),
            "fake Codex effective-config diagnostics are not UTF-8"
        );
        Ok(output.value)
    })
    .map_err(|error| {
        let diagnostics = std::fs::read_to_string(fixture.repository.join(CHILD_ERROR_FILE))
            .unwrap_or_else(|_| "fake child emitted no diagnostics".to_string());
        error.context(diagnostics)
    });
    if mode != "ambient" && mode != "safe" && mode != "supported-notification" {
        ensure!(
            fixture.marker(mode).is_file(),
            "fake Codex config mode {mode} did not issue its fault marker"
        );
    }
    result.map(|config| (config, fixture))
}

fn wait_until_ready(dialogue: &mut LineDialogue) -> Result<()> {
    let mut observed = 0usize;
    for _ in 0..8 {
        let line = dialogue
            .receive_line()?
            .context("fake Codex app-server closed before its ready marker")?;
        observed = observed
            .checked_add(line.len())
            .context("fake Codex app-server ready prelude length overflowed")?;
        ensure!(
            observed <= 4096,
            "fake Codex app-server ready prelude is excessive"
        );
        if line.ends_with(READY) {
            return Ok(());
        }
    }
    bail!("fake Codex app-server omitted its ready marker")
}

fn assert_rejected(cases: &[(&str, &str)]) {
    for (mode, expected) in cases {
        let error = run_fake(mode).unwrap_err();
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains(expected),
            "mode {mode} returned the wrong error; expected `{expected}` in {rendered}"
        );
    }
}

fn read_child_mode(repository: &Path) -> Result<String> {
    let mode = std::fs::read_to_string(repository.join(MODE_FILE))
        .context("reading fake Codex config mode")?;
    ensure!(
        !mode.is_empty() && !mode.contains(['\r', '\n', '\0']),
        "fake Codex config mode is invalid"
    );
    Ok(mode)
}

fn issue_fault(repository: &Path, mode: &str) -> Result<()> {
    std::fs::write(
        repository.join(format!("{FAULT_FILE_PREFIX}{mode}")),
        mode.as_bytes(),
    )
    .with_context(|| format!("issuing fake Codex config fault marker for {mode}"))
}

fn read_value(reader: &mut impl BufRead, label: &str) -> Result<JsonValue> {
    let mut line = Vec::new();
    ensure!(
        reader.read_until(b'\n', &mut line)? != 0,
        "fake Codex config client closed before {label}"
    );
    ensure!(
        line.last() == Some(&b'\n')
            && line.iter().filter(|byte| **byte == b'\n').count() == 1
            && !line.contains(&b'\r'),
        "fake Codex config client used noncanonical framing for {label}"
    );
    line.pop();
    let value = parse_json(&line, label)?;
    ensure!(
        render_json(&value)?.as_bytes() == line,
        "fake Codex config client did not use canonical JSON for {label}"
    );
    Ok(value)
}

fn expect_value(reader: &mut impl BufRead, expected: JsonValue, label: &str) -> Result<()> {
    ensure!(
        read_value(reader, label)? == expected,
        "fake Codex config client {label} differs"
    );
    Ok(())
}

fn send_value(writer: &mut impl Write, value: &JsonValue) -> Result<()> {
    writer.write_all(render_json(value)?.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn expected_initialize_request() -> JsonValue {
    object([
        ("id", JsonValue::Number("1".to_string())),
        ("method", text_value("initialize")),
        (
            "params",
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
        ),
    ])
}

fn expected_initialized_notification() -> JsonValue {
    object([
        ("method", text_value("initialized")),
        ("params", object(std::iter::empty::<(&str, JsonValue)>())),
    ])
}

fn expected_config_request(repository: &Path) -> JsonValue {
    object([
        ("id", JsonValue::Number("2".to_string())),
        ("method", text_value("config/read")),
        (
            "params",
            object([
                ("cwd", path_value(repository)),
                ("includeLayers", JsonValue::Bool(false)),
            ]),
        ),
    ])
}

fn response(id: u64, result: JsonValue) -> JsonValue {
    object([
        ("id", JsonValue::Number(id.to_string())),
        ("result", result),
    ])
}

fn initialize_result(repository: &Path, mode: &str) -> Result<JsonValue> {
    let codex_home = repository
        .parent()
        .context("fake repository has no fixture root")?
        .join(OWNED_SCRATCH_NAME)
        .join("codex-home");
    let mut result = BTreeMap::from([
        (
            "codexHome".to_string(),
            path_value(if mode == "initialize-home-drift" {
                Path::new(r"C:\foreign-codex-home")
            } else {
                &codex_home
            }),
        ),
        (
            "platformFamily".to_string(),
            text_value(if mode == "initialize-platform-drift" {
                "unix"
            } else {
                "windows"
            }),
        ),
        (
            "platformOs".to_string(),
            text_value(if mode == "initialize-platform-drift" {
                "linux"
            } else {
                "windows"
            }),
        ),
        (
            "userAgent".to_string(),
            text_value(if mode == "initialize-agent-drift" {
                "foreign-client/1"
            } else {
                "codex_security_deep_scan/0.162.0-alpha.17.2 (Windows 10.0.26200; x86_64) unknown (codex_security_deep_scan; 0.1.160)"
            }),
        ),
    ]);
    if mode == "initialize-result-extra" {
        result.insert("unexpected".to_string(), JsonValue::Null);
    }
    Ok(JsonValue::Object(result))
}

fn ambient_config() -> JsonValue {
    object([
        (
            "mcp_servers",
            object([(
                "node_repl",
                object([
                    ("args", JsonValue::Array(vec![text_value("--ambient")])),
                    ("command", text_value("ambient-command")),
                    ("enabled", JsonValue::Bool(true)),
                    ("env", object([("AMBIENT_KEY", text_value("secret"))])),
                    (
                        "env_vars",
                        JsonValue::Array(vec![text_value("PASSTHROUGH")]),
                    ),
                    ("environment_id", text_value("ambient")),
                    ("startup_timeout_sec", JsonValue::Number("9".to_string())),
                    ("tool_timeout_sec", JsonValue::Number("10".to_string())),
                ]),
            )]),
        ),
        (
            "plugins",
            object([(
                "browser@openai-bundled",
                object([("enabled", JsonValue::Bool(true))]),
            )]),
        ),
    ])
}

fn safe_config(inert_mcp: &Path, mode: &str) -> JsonValue {
    let mut config = BTreeMap::from([
        (
            "chatgpt_base_url".to_string(),
            text_value(OFFICIAL_CHATGPT_BASE_URL),
        ),
        (
            "features".to_string(),
            object([
                ("memories", JsonValue::Bool(false)),
                (
                    "network_proxy",
                    if mode == "dependent-default-drift" {
                        JsonValue::Bool(true)
                    } else {
                        JsonValue::Null
                    },
                ),
            ]),
        ),
        (
            "mcp_servers".to_string(),
            object([(
                "node_repl",
                object([
                    ("args", JsonValue::Array(Vec::new())),
                    (
                        "command",
                        path_value(if mode == "drift-mcp" {
                            Path::new("wrong-command")
                        } else {
                            inert_mcp
                        }),
                    ),
                    ("enabled", JsonValue::Bool(false)),
                    ("env", object([("AMBIENT_KEY", text_value(""))])),
                    ("environment_id", text_value(INERT_MCP_ENVIRONMENT)),
                    (
                        "startup_timeout_sec",
                        JsonValue::Number(INERT_MCP_TIMEOUT_SECONDS.to_string()),
                    ),
                    (
                        "tool_timeout_sec",
                        JsonValue::Number(INERT_MCP_TIMEOUT_SECONDS.to_string()),
                    ),
                ]),
            )]),
        ),
        (
            "memories".to_string(),
            object([
                ("consolidation_model", JsonValue::Null),
                ("dedicated_tools", JsonValue::Null),
                ("disable_on_external_context", JsonValue::Null),
                ("dual_write", JsonValue::Null),
                ("extract_model", JsonValue::Null),
                ("generate_memories", JsonValue::Bool(false)),
                ("max_raw_memories_for_consolidation", JsonValue::Null),
                ("max_rollout_age_days", JsonValue::Null),
                ("max_rollouts_per_startup", JsonValue::Null),
                ("max_unused_days", JsonValue::Null),
                ("min_rate_limit_remaining_percent", JsonValue::Null),
                ("min_rollout_idle_hours", JsonValue::Null),
                ("use_memories", JsonValue::Bool(false)),
                ("version", JsonValue::Null),
            ]),
        ),
        (
            "model".to_string(),
            text_value(if mode == "drift-model" {
                "other-model"
            } else {
                MODEL
            }),
        ),
        ("model_provider".to_string(), text_value("openai")),
        ("model_reasoning_effort".to_string(), text_value(REASONING)),
        ("notify".to_string(), JsonValue::Array(Vec::new())),
        (
            "openai_base_url".to_string(),
            text_value(OFFICIAL_CHATGPT_BASE_URL),
        ),
        (
            "plugins".to_string(),
            object([(
                "browser@openai-bundled",
                object([("enabled", JsonValue::Bool(mode == "drift-plugin-enabled"))]),
            )]),
        ),
        ("service_tier".to_string(), text_value("default")),
    ]);
    if mode == "config-extra" {
        config.insert("unexpected".to_string(), JsonValue::Null);
    }
    if mode == "default-drift" {
        config.insert("approval_policy".to_string(), text_value("never"));
    }
    JsonValue::Object(config)
}

fn origins(mode: &str) -> JsonValue {
    let mut names = vec![
        "chatgpt_base_url",
        "features.memories",
        "mcp_servers.node_repl.command",
        "mcp_servers.node_repl.enabled",
        "mcp_servers.node_repl.env.AMBIENT_KEY",
        "mcp_servers.node_repl.environment_id",
        "mcp_servers.node_repl.startup_timeout_sec",
        "mcp_servers.node_repl.tool_timeout_sec",
        "memories.generate_memories",
        "memories.use_memories",
        "model",
        "model_provider",
        "model_reasoning_effort",
        "notify",
        "openai_base_url",
        "plugins.browser@openai-bundled.enabled",
        "service_tier",
    ];
    if mode == "drift-origin" {
        names.retain(|name| *name != "model");
    }
    if mode == "foreign-origin" {
        names.push("approval_policy");
    }
    JsonValue::Object(
        names
            .into_iter()
            .map(|name| (name.to_string(), JsonValue::Null))
            .collect(),
    )
}

fn config_result(repository: &Path, mode: &str) -> JsonValue {
    let inert_mcp = repository
        .parent()
        .expect("fixture repository has a parent")
        .join(OWNED_SCRATCH_NAME)
        .join(INERT_MCP_EXECUTABLE);
    let config = if mode == "ambient" {
        ambient_config()
    } else {
        safe_config(&inert_mcp, mode)
    };
    let mut result = BTreeMap::from([
        ("config".to_string(), config),
        (
            "origins".to_string(),
            if mode == "ambient" {
                object(std::iter::empty::<(&str, JsonValue)>())
            } else {
                origins(mode)
            },
        ),
    ]);
    if mode == "result-extra" {
        result.insert("unexpected".to_string(), JsonValue::Null);
    }
    JsonValue::Object(result)
}

fn send_notification(writer: &mut impl Write, repository: &Path, mode: &str) -> Result<()> {
    issue_fault(repository, mode)?;
    send_value(
        writer,
        &object([
            ("emittedAtMs", JsonValue::Number("1".to_string())),
            (
                "method",
                text_value(if mode == "unknown-notification" {
                    "unexpected/notification"
                } else {
                    "remoteControl/status/changed"
                }),
            ),
            ("params", object(std::iter::empty::<(&str, JsonValue)>())),
        ]),
    )
}

fn fake_app_server(repository: &Path, mode: &str) -> Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    writer.write_all(READY)?;
    writer.flush()?;

    expect_value(
        &mut reader,
        expected_initialize_request(),
        "initialize request",
    )?;
    if mode == "unsuccessful-child" {
        issue_fault(repository, mode)?;
        bail!("intentional fake app-server failure");
    }
    if mode == "premature-eof" {
        issue_fault(repository, mode)?;
        return Ok(());
    }
    if matches!(
        mode,
        "initialize-result-extra"
            | "initialize-home-drift"
            | "initialize-platform-drift"
            | "initialize-agent-drift"
    ) {
        issue_fault(repository, mode)?;
    }
    let initialize_result = initialize_result(repository, mode)?;
    send_value(&mut writer, &response(1, initialize_result))?;
    expect_value(
        &mut reader,
        expected_initialized_notification(),
        "initialized notification",
    )?;
    expect_value(
        &mut reader,
        expected_config_request(repository),
        "config/read request",
    )?;

    if mode == "supported-notification" {
        send_value(
            &mut writer,
            &object([
                ("emittedAtMs", JsonValue::Number("1".to_string())),
                ("method", text_value("remoteControl/status/changed")),
                ("params", object(std::iter::empty::<(&str, JsonValue)>())),
            ]),
        )?;
    } else if mode == "unknown-notification" {
        send_notification(&mut writer, repository, mode)?;
    }

    let result = config_result(repository, mode);
    match mode {
        "wrong-id" | "duplicate-id" => {
            issue_fault(repository, mode)?;
            send_value(&mut writer, &response(1, result))?;
        }
        "error-and-result" => {
            let mut value = BTreeMap::from([
                ("id".to_string(), JsonValue::Number("2".to_string())),
                ("result".to_string(), result),
                (
                    "error".to_string(),
                    object([("message", text_value("fixture failure"))]),
                ),
            ]);
            issue_fault(repository, mode)?;
            send_value(&mut writer, &JsonValue::Object(std::mem::take(&mut value)))?;
        }
        "error-only" => {
            issue_fault(repository, mode)?;
            send_value(
                &mut writer,
                &object([
                    ("id", JsonValue::Number("2".to_string())),
                    (
                        "error",
                        object([("message", text_value("fixture failure"))]),
                    ),
                ]),
            )?;
        }
        "envelope-extra" | "wrong-jsonrpc" => {
            let JsonValue::Object(mut value) = response(2, result) else {
                unreachable!()
            };
            value.insert(
                if mode == "wrong-jsonrpc" {
                    "jsonrpc".to_string()
                } else {
                    "unexpected".to_string()
                },
                if mode == "wrong-jsonrpc" {
                    text_value("1.0")
                } else {
                    JsonValue::Null
                },
            );
            issue_fault(repository, mode)?;
            send_value(&mut writer, &JsonValue::Object(value))?;
        }
        "malformed" => {
            issue_fault(repository, mode)?;
            writer.write_all(b"{not-json}\n")?;
            writer.flush()?;
        }
        "crlf" => {
            let value = response(2, result);
            issue_fault(repository, mode)?;
            writer.write_all(render_json(&value)?.as_bytes())?;
            writer.write_all(b"\r\n")?;
            writer.flush()?;
        }
        "oversized-line" => {
            let mut oversized = vec![b'x'; 4 * 1024 * 1024 + 1];
            oversized.push(b'\n');
            issue_fault(repository, mode)?;
            writer.write_all(&oversized)?;
            writer.flush()?;
        }
        _ => {
            if matches!(
                mode,
                "result-extra"
                    | "config-extra"
                    | "default-drift"
                    | "dependent-default-drift"
                    | "drift-model"
                    | "drift-plugin-enabled"
                    | "drift-mcp"
                    | "drift-origin"
                    | "foreign-origin"
            ) {
                issue_fault(repository, mode)?;
            }
            send_value(&mut writer, &response(2, result))?;
        }
    }
    if mode == "trailing-output" {
        issue_fault(repository, mode)?;
        send_value(
            &mut writer,
            &object([("method", text_value("unexpected/trailing"))]),
        )?;
    }
    let mut trailing = Vec::new();
    reader.read_to_end(&mut trailing)?;
    ensure!(trailing.is_empty(), "config reader sent trailing input");
    Ok(())
}

fn expected_overrides(inert_mcp: &Path) -> Vec<String> {
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
        "mcp_servers.node_repl.args=[]".to_string(),
        format!(
            "mcp_servers.node_repl.command={}",
            quote_toml_string(&inert_mcp.to_string_lossy())
        ),
        "mcp_servers.node_repl.enabled=false".to_string(),
        "mcp_servers.node_repl.env.AMBIENT_KEY=\"\"".to_string(),
        "mcp_servers.node_repl.env_vars=[]".to_string(),
        format!("mcp_servers.node_repl.environment_id=\"{INERT_MCP_ENVIRONMENT}\""),
        format!("mcp_servers.node_repl.startup_timeout_sec={INERT_MCP_TIMEOUT_SECONDS}"),
        format!("mcp_servers.node_repl.tool_timeout_sec={INERT_MCP_TIMEOUT_SECONDS}"),
        "plugins.browser@openai-bundled.enabled=false".to_string(),
    ]
}

#[test]
fn config_reader_emits_exact_requests_and_accepts_only_the_supported_notification() {
    let (ambient, _) = run_fake("ambient").unwrap();
    assert_eq!(ambient.mcp_servers.len(), 1);
    assert_eq!(ambient.plugins.len(), 1);
    let (notified, _) = run_fake("supported-notification").unwrap();
    assert_eq!(notified.model.as_deref(), Some(MODEL));
}

#[test]
fn config_command_preserves_exact_override_order_and_runtime_shape() {
    let (_, fixture) = run_fake("ambient").unwrap();
    let overrides = expected_overrides(&fixture.inert_mcp);
    let command = codex_config_command(&fake_identity().unwrap(), &overrides);
    assert_eq!(command.get_program(), fake_identity().unwrap().path);
    let expected: Vec<OsString> = overrides
        .iter()
        .flat_map(|value| [OsString::from("--config"), OsString::from(value)])
        .chain([OsString::from("app-server"), OsString::from("--stdio")])
        .collect();
    assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
}

#[test]
fn ambient_overrides_produce_the_exact_safe_effective_config() {
    let (ambient, fixture) = run_fake("ambient").unwrap();
    let overrides = safety_overrides(&ambient, &fixture.inert_mcp).unwrap();
    assert_eq!(overrides, expected_overrides(&fixture.inert_mcp));
    let (effective, safe_fixture) = run_fake("safe").unwrap();
    require_safe_effective_config(&effective, &ambient, &safe_fixture.inert_mcp).unwrap();
}

#[test]
fn config_response_correlation_and_transport_fail_closed() {
    assert_rejected(&[
        (
            "initialize-result-extra",
            "Codex app-server initialize result schema differs",
        ),
        (
            "initialize-home-drift",
            "did not use the private Codex home",
        ),
        (
            "initialize-platform-drift",
            "platform differs from the qualified Windows host",
        ),
        (
            "initialize-agent-drift",
            "user agent differs from the qualified client",
        ),
        ("wrong-id", "unexpected ID"),
        ("duplicate-id", "unexpected ID"),
        ("error-and-result", "exactly one result or error"),
        ("error-only", "request 2 failed"),
        ("envelope-extra", "unexpected field"),
        ("wrong-jsonrpc", "unexpected field"),
        ("malformed", "Codex app-server response"),
        ("crlf", "not strict LF framing"),
        ("oversized-line", "line longer than 4194304 bytes"),
        ("premature-eof", "closed before its response 1"),
        ("unknown-notification", "unexpected notification"),
        (
            "trailing-output",
            "sent output after its final config/read response",
        ),
        ("unsuccessful-child", "closed before its response 1"),
    ]);
}

#[test]
fn effective_config_schemas_and_safety_fields_fail_closed() {
    assert_rejected(&[
        ("result-extra", "Codex config/read result"),
        ("config-extra", "Codex originless default config differs"),
        ("default-drift", "Codex originless default config differs"),
        (
            "dependent-default-drift",
            "Codex features network_proxy dependent default is not null",
        ),
    ]);
    for (mode, expected) in [
        ("drift-model", "model route differs"),
        (
            "drift-plugin-enabled",
            "enabled or unexpected MCP/plugin route",
        ),
        ("drift-mcp", "not the exact inert disabled transport"),
        ("drift-origin", "has no origin for safety field model"),
        (
            "foreign-origin",
            "origin not authorized by the safety overrides",
        ),
    ] {
        let (effective, fixture) = run_fake(mode).unwrap();
        let (ambient, _) = run_fake("ambient").unwrap();
        let error =
            require_safe_effective_config(&effective, &ambient, &fixture.inert_mcp).unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "mode {mode} returned the wrong policy error: {error:#}"
        );
    }
}

#[test]
#[ignore]
fn fake_codex_app_child() {
    let repository = std::fs::canonicalize(
        std::env::current_dir().expect("reading fake app-server working directory"),
    )
    .expect("canonicalizing fake app-server working directory");
    let mode = read_child_mode(&repository).expect("reading fake app-server mode");
    let exit = match fake_app_server(&repository, &mode) {
        Ok(()) => 0,
        Err(error) => {
            let _ = std::fs::write(repository.join(CHILD_ERROR_FILE), format!("{error:#}"));
            eprintln!("fake Codex app-server failed: {error:#}");
            23
        }
    };
    std::process::exit(exit);
}
