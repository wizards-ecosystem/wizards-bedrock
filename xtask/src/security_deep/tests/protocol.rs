use super::*;

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use super::fake_app_server::{expect_value, read_value, send_value, wait_until_ready};

const FAKE_MODE_ENV: &str = "WIZARD_SECURITY_DEEP_FAKE_MCP_MODE";
const FAKE_ROOT_ENV: &str = "WIZARD_SECURITY_DEEP_FAKE_MCP_ROOT";
const READY: &[u8] = b"WIZARD-SECURITY-DEEP-FAKE-MCP-READY\n";
const CLIENT: &str = "fake MCP client";
const SCAN_ID: &str = "11111111-1111-4111-8111-111111111111";
const OTHER_SCAN_ID: &str = "22222222-2222-4222-8222-222222222222";
const WORKSPACE_ID: &str = "33333333-3333-4333-8333-333333333333";
const OTHER_RUNNING_SCAN_ID: &str = "44444444-4444-4444-8444-444444444444";
const PARENT_SCAN_ID: &str = "55555555-5555-4555-8555-555555555555";

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct FakePaths {
    root: PathBuf,
    repository: PathBuf,
    other_repository: PathBuf,
    scan_root: PathBuf,
    scan_directory: PathBuf,
    changed_scan_directory: PathBuf,
    wrong_revision_directory: PathBuf,
    escaped_scan_directory: PathBuf,
    other_manifest: PathBuf,
    mcp_tools: PathBuf,
    fault_issued: PathBuf,
}

impl FakePaths {
    fn from_root(root: PathBuf) -> Self {
        let repository = root.join("repository");
        let scan_root = root.join("scans");
        let target_root = scan_root.join("repository");
        let commit = source().commit;
        Self {
            other_repository: root.join("other-repository"),
            scan_directory: target_root.join(format!("{commit}_{SCAN_ID}")),
            changed_scan_directory: target_root.join(format!("{commit}_{OTHER_SCAN_ID}")),
            wrong_revision_directory: target_root.join(format!("{}_{SCAN_ID}", "c".repeat(40))),
            escaped_scan_directory: root.join("escaped").join(format!("{commit}_{SCAN_ID}")),
            other_manifest: root.join("other-manifest.json"),
            mcp_tools: root.join("mcp-tools.json"),
            fault_issued: root.join("fault-issued"),
            root,
            repository,
            scan_root,
        }
    }

    fn start_directory(&self, mode: &str) -> &Path {
        match mode {
            "start-escaped-directory" => &self.escaped_scan_directory,
            "start-wrong-revision-directory" => &self.wrong_revision_directory,
            _ => &self.scan_directory,
        }
    }
}

struct Fixture {
    paths: FakePaths,
}

impl Fixture {
    fn new(mode: &str) -> Result<Self> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("fake MCP fixture clock predates the Unix epoch")?
            .as_nanos();
        let mut root = None;
        for _ in 0..64 {
            let candidate = std::env::temp_dir().join(format!(
                "wizard-security-deep-protocol-test-{}-{timestamp}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&candidate) {
                Ok(()) => {
                    root = Some(candidate);
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("creating fake MCP root {}", candidate.display())
                    });
                }
            }
        }
        let root = root.context("exhausted unique fake MCP fixture names")?;
        let root = match std::fs::canonicalize(&root) {
            Ok(root) => root,
            Err(error) => {
                let _ = std::fs::remove_dir(&root);
                return Err(error)
                    .with_context(|| format!("canonicalizing fake MCP root {}", root.display()));
            }
        };
        let fixture = Self {
            paths: FakePaths::from_root(root),
        };
        std::fs::create_dir(&fixture.paths.repository)?;
        std::fs::create_dir(&fixture.paths.other_repository)?;
        for directory in [
            &fixture.paths.scan_directory,
            &fixture.paths.changed_scan_directory,
            &fixture.paths.wrong_revision_directory,
            &fixture.paths.escaped_scan_directory,
        ] {
            std::fs::create_dir_all(directory)?;
            seed_artifacts(directory)?;
        }
        std::fs::write(&fixture.paths.other_manifest, b"other manifest")?;
        std::fs::write(
            &fixture.paths.mcp_tools,
            render_json(&JsonValue::Array(pinned_mcp_tool_inventory_for_test()))?,
        )?;
        if mode == "start-missing-manifest" {
            std::fs::remove_file(fixture.paths.scan_directory.join(MANIFEST))?;
        } else if mode == "artifact-missing" {
            std::fs::remove_file(fixture.paths.scan_directory.join(REPORT))?;
        }
        Ok(fixture)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(metadata) = std::fs::symlink_metadata(&self.paths.root) {
            let _ = remove_entry_no_follow(&self.paths.root, &metadata);
        }
    }
}

fn seed_artifacts(directory: &Path) -> Result<()> {
    for name in [MANIFEST, FINDINGS, COVERAGE, REPORT] {
        std::fs::write(directory.join(name), format!("fixture {name}\n"))?;
    }
    Ok(())
}

fn issue_fault(paths: &FakePaths, mode: &str) -> Result<()> {
    ensure!(mode != "happy", "refusing to mark the happy fake MCP path");
    let mut marker = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&paths.fault_issued)
        .with_context(|| {
            format!(
                "creating fake MCP fault marker {}",
                paths.fault_issued.display()
            )
        })?;
    marker.write_all(mode.as_bytes())?;
    marker.sync_all()?;
    Ok(())
}

fn read_fault_marker(paths: &FakePaths) -> Result<Option<String>> {
    match std::fs::read_to_string(&paths.fault_issued) {
        Ok(marker) => Ok(Some(marker)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| {
            format!(
                "reading fake MCP fault marker {}",
                paths.fault_issued.display()
            )
        }),
    }
}

fn is_initialize_fault(mode: &str) -> bool {
    matches!(
        mode,
        "wrong-id"
            | "error-and-result"
            | "malformed-json"
            | "crlf-response"
            | "oversized-line"
            | "premature-eof"
            | "initialize-response-extra"
            | "initialize-wrong-jsonrpc"
            | "initialize-envelope-extra"
            | "initialize-missing-capabilities"
            | "initialize-server-info-extra"
            | "initialize-wrong-protocol"
            | "initialize-wrong-name"
            | "initialize-wrong-version"
    )
}

fn is_notification_fault(mode: &str) -> bool {
    matches!(
        mode,
        "server-unknown-notification" | "server-notification-result" | "server-notification-error"
    )
}

fn is_ping_fault(mode: &str) -> bool {
    matches!(
        mode,
        "server-ping-bad-id" | "server-ping-extra" | "server-unsupported-request"
    )
}

fn is_completion_fault(mode: &str) -> bool {
    mode == "duplicate-id"
        || mode.starts_with("complete-")
        || mode.starts_with("workspace-")
        || mode.starts_with("artifact-")
}

fn source() -> SourceIdentity {
    SourceIdentity {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    }
}

fn start_scan_id(mode: &str) -> &str {
    if mode == "start-empty-scan-id" {
        ""
    } else {
        SCAN_ID
    }
}

fn clone_json(value: &JsonValue) -> JsonValue {
    let rendered = render_json(value).expect("fixture JSON renders");
    parse_json(rendered.as_bytes(), "cloned fake MCP JSON").expect("fixture JSON reparses")
}

fn path_text(path: &Path) -> JsonValue {
    JsonValue::String(path.to_string_lossy().into_owned())
}

fn live_auth_path(repository: &Path) -> PathBuf {
    repository
        .parent()
        .expect("fake repository has a fixture parent")
        .join("live-codex-home")
        .join("auth.json")
}

fn projected_auth_path(repository: &Path) -> PathBuf {
    repository
        .parent()
        .expect("fake repository has a fixture parent")
        .join("codex-home")
        .join("auth.json")
}

fn response(id: &str, result: JsonValue) -> JsonValue {
    object([
        ("id", text_value(id)),
        ("jsonrpc", text_value("2.0")),
        ("result", result),
    ])
}

fn initialize_result(mode: &str) -> JsonValue {
    let JsonValue::Object(mut result) = pinned_mcp_initialize_for_test() else {
        panic!("pinned MCP initialize fixture is not an object")
    };
    if mode == "initialize-wrong-protocol" {
        result.insert("protocolVersion".to_string(), text_value("2025-11-25"));
    } else if mode == "initialize-envelope-extra" {
        result.insert("unexpected".to_string(), JsonValue::Null);
    } else if mode == "initialize-missing-capabilities" {
        result.remove("capabilities");
    }
    let JsonValue::Object(server_info) = result
        .get_mut("serverInfo")
        .expect("pinned MCP initialize fixture omitted serverInfo")
    else {
        panic!("pinned MCP initialize serverInfo is not an object")
    };
    if mode == "initialize-wrong-name" {
        server_info.insert("name".to_string(), text_value("other-server"));
    } else if mode == "initialize-wrong-version" {
        server_info.insert("version".to_string(), text_value("0.0.0"));
    } else if mode == "initialize-server-info-extra" {
        server_info.insert("unexpected".to_string(), JsonValue::Null);
    }
    JsonValue::Object(result)
}

fn expected_metadata(repository: &Path, source: &SourceIdentity) -> JsonValue {
    let live_auth = live_auth_path(repository);
    let projected_auth = projected_auth_path(repository);
    let mut entries = vec![object([
        ("access", text_value("read")),
        (
            "path",
            object([
                ("type", text_value("special")),
                ("value", object([("kind", text_value("root"))])),
            ]),
        ),
    ])];
    entries.extend(
        [repository.join(".git"), live_auth, projected_auth]
            .into_iter()
            .map(|path| {
                object([
                    ("access", text_value("deny")),
                    (
                        "path",
                        object([("path", path_value(&path)), ("type", text_value("path"))]),
                    ),
                ])
            }),
    );
    object([
        (
            "codex/sandbox-state-meta",
            object([
                (
                    "permissionProfile",
                    object([
                        (
                            "file_system",
                            object([
                                ("entries", JsonValue::Array(entries)),
                                ("type", text_value("restricted")),
                            ]),
                        ),
                        ("network", text_value("restricted")),
                        ("type", text_value("managed")),
                    ]),
                ),
                (
                    "sandboxCwd",
                    text_value(&windows_file_url(repository).unwrap()),
                ),
                ("useMxc", JsonValue::Bool(true)),
            ]),
        ),
        (
            "openai/threadId",
            text_value(&format!("wizard-bedrock-deep-{}", source.commit)),
        ),
        (
            "x-codex-turn-metadata",
            object([
                ("model", text_value(MODEL)),
                ("reasoning_effort", text_value(REASONING)),
            ]),
        ),
    ])
}

fn expected_initialize_request() -> JsonValue {
    object([
        ("id", text_value("initialize")),
        ("jsonrpc", text_value("2.0")),
        ("method", text_value("initialize")),
        (
            "params",
            object([
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
            ]),
        ),
    ])
}

fn expected_initialized_notification() -> JsonValue {
    object([
        ("jsonrpc", text_value("2.0")),
        ("method", text_value("notifications/initialized")),
    ])
}

fn expected_tools_request() -> JsonValue {
    object([
        ("id", text_value("tools")),
        ("jsonrpc", text_value("2.0")),
        ("method", text_value("tools/list")),
        ("params", object(std::iter::empty::<(&str, JsonValue)>())),
    ])
}

fn expected_daybreak_request(repository: &Path, source: &SourceIdentity) -> JsonValue {
    object([
        ("id", text_value("daybreak")),
        ("jsonrpc", text_value("2.0")),
        ("method", text_value("tools/call")),
        (
            "params",
            object([
                ("_meta", expected_metadata(repository, source)),
                ("arguments", object(std::iter::empty::<(&str, JsonValue)>())),
                ("name", text_value(DAYBREAK_TOOL)),
            ]),
        ),
    ])
}

fn expected_start_request(repository: &Path, source: &SourceIdentity) -> JsonValue {
    object([
        ("id", text_value("start")),
        ("jsonrpc", text_value("2.0")),
        ("method", text_value("tools/call")),
        (
            "params",
            object([
                ("_meta", expected_metadata(repository, source)),
                (
                    "arguments",
                    object([
                        ("scope", text_value(".")),
                        ("targetPath", path_text(repository)),
                    ]),
                ),
                ("name", text_value(START_TOOL)),
            ]),
        ),
    ])
}

fn expected_complete_request(mode: &str, repository: &Path, source: &SourceIdentity) -> JsonValue {
    object([
        ("id", text_value("complete")),
        ("jsonrpc", text_value("2.0")),
        ("method", text_value("tools/call")),
        (
            "params",
            object([
                ("_meta", expected_metadata(repository, source)),
                (
                    "arguments",
                    object([("scanId", text_value(start_scan_id(mode)))]),
                ),
                ("name", text_value(COMPLETE_TOOL)),
            ]),
        ),
    ])
}

fn tools_result(mode: &str, paths: &FakePaths) -> JsonValue {
    let JsonValue::Array(mut tools) = parse_json(
        &std::fs::read(&paths.mcp_tools).expect("reading pinned MCP tool fixture"),
        "pinned MCP tool fixture",
    )
    .expect("parsing pinned MCP tool fixture") else {
        panic!("pinned MCP tool fixture is not an array")
    };
    let find = |tools: &[JsonValue], name: &str| {
        tools
            .iter()
            .position(|tool| {
                matches!(tool, JsonValue::Object(fields)
                    if fields.get("name") == Some(&text_value(name)))
            })
            .unwrap_or_else(|| panic!("pinned MCP fixture omitted {name}"))
    };
    let start = find(&tools, START_TOOL);
    let complete = find(&tools, COMPLETE_TOOL);
    let daybreak = find(&tools, DAYBREAK_TOOL);
    match mode {
        "tools-wrong-name" => {
            let JsonValue::Object(tool) = &mut tools[start] else {
                unreachable!()
            };
            tool.insert("name".to_string(), text_value("other-start"));
        }
        "tools-missing" => {
            tools.remove(start);
        }
        "tools-duplicate" => {
            let JsonValue::Object(tool) = &mut tools[complete] else {
                unreachable!()
            };
            tool.insert("name".to_string(), text_value(START_TOOL));
        }
        "tools-extra" => tools.push(object([("name", text_value("other-tool"))])),
        "tools-reordered" => tools.swap(start, complete),
        "tools-schema-drift" => {
            let JsonValue::Object(tool) = &mut tools[start] else {
                unreachable!()
            };
            tool.insert("unexpected".to_string(), JsonValue::Null);
        }
        "tools-daybreak-missing" => {
            tools.remove(daybreak);
        }
        "tools-daybreak-duplicate" => {
            let JsonValue::Object(tool) = &mut tools[complete] else {
                unreachable!()
            };
            tool.insert("name".to_string(), text_value(DAYBREAK_TOOL));
        }
        "tools-daybreak-schema-drift" => {
            let JsonValue::Object(tool) = &mut tools[daybreak] else {
                unreachable!()
            };
            tool.insert("unexpected".to_string(), JsonValue::Null);
        }
        _ => {}
    }
    let mut result = BTreeMap::from([
        (
            "nextCursor".to_string(),
            if mode == "tools-cursor" {
                text_value("next")
            } else {
                JsonValue::Null
            },
        ),
        (
            "tools".to_string(),
            if mode == "tools-not-array" {
                object(std::iter::empty::<(&str, JsonValue)>())
            } else {
                JsonValue::Array(tools)
            },
        ),
    ]);
    if mode == "tools-result-schema-drift" {
        result.insert("unexpected".to_string(), JsonValue::Null);
    }
    JsonValue::Object(result)
}

fn jsonrpc_error_response(id: &str, error: JsonValue) -> JsonValue {
    object([
        ("error", error),
        ("id", text_value(id)),
        ("jsonrpc", text_value("2.0")),
    ])
}

fn daybreak_structured(
    status: &str,
    programs: &[&str],
    checked_at: &str,
    stale: bool,
    enrollment_url: Option<JsonValue>,
) -> BTreeMap<String, JsonValue> {
    let mut structured = BTreeMap::from([
        ("checkedAt".to_string(), text_value(checked_at)),
        (
            "programs".to_string(),
            JsonValue::Array(programs.iter().map(|program| text_value(program)).collect()),
        ),
        (
            "schemaVersion".to_string(),
            JsonValue::Number("1".to_string()),
        ),
        ("stale".to_string(), JsonValue::Bool(stale)),
        ("status".to_string(), text_value(status)),
    ]);
    if let Some(enrollment_url) = enrollment_url {
        structured.insert("enrollmentUrl".to_string(), enrollment_url);
    }
    structured
}

fn daybreak_text(status: &str, programs: &[&str], checked_at: &str, stale: bool) -> String {
    let programs = if programs.is_empty() {
        "none".to_string()
    } else {
        programs.join(", ")
    };
    let suffix = if status == "not_granted" {
        DAYBREAK_NOT_GRANTED_SUFFIX
    } else {
        ""
    };
    format!(
        "Codex Security Daybreak access: status={status}; programs={programs}; checkedAt={checked_at}; stale={stale}.{suffix}"
    )
}

fn daybreak_result(mode: &str) -> JsonValue {
    let checked_at = "2026-10-15T00:00:00Z";
    let (status, programs, stale, enrollment_url): (&str, Vec<&str>, bool, Option<JsonValue>) =
        match mode {
            "daybreak-granted-blue" => ("granted", vec!["Daybreak Blue"], false, None),
            "daybreak-granted-red" => ("granted", vec!["Daybreak Red"], false, None),
            "daybreak-granted-both" => (
                "granted",
                vec!["Daybreak Blue", "Daybreak Red"],
                false,
                None,
            ),
            "daybreak-not-granted" => ("not_granted", Vec::new(), false, None),
            "daybreak-not-granted-url" => (
                "not_granted",
                Vec::new(),
                false,
                Some(text_value("https://chatgpt.com/cyber/apply")),
            ),
            "daybreak-not-granted-url-port" => (
                "not_granted",
                Vec::new(),
                false,
                Some(text_value(
                    "https://chatgpt.com:443/cyber/apply?from=deep#access",
                )),
            ),
            "daybreak-not-granted-unsafe-url" => (
                "not_granted",
                Vec::new(),
                false,
                Some(text_value("http://example.invalid/apply")),
            ),
            "daybreak-not-granted-invalid-authority" => (
                "not_granted",
                Vec::new(),
                false,
                Some(text_value("https://:")),
            ),
            "daybreak-not-granted-invalid-escape" => (
                "not_granted",
                Vec::new(),
                false,
                Some(text_value("https://chatgpt.com/cyber/%zz")),
            ),
            "daybreak-stale" => ("unknown", Vec::new(), true, None),
            "daybreak-payload-status" => ("other", Vec::new(), false, None),
            "daybreak-payload-program" => ("granted", vec!["Daybreak Green"], false, None),
            "daybreak-payload-duplicate-program" => (
                "granted",
                vec!["Daybreak Blue", "Daybreak Blue"],
                false,
                None,
            ),
            "daybreak-payload-status-programs" => ("unknown", vec!["Daybreak Blue"], false, None),
            "daybreak-payload-stale-status" => ("granted", vec!["Daybreak Blue"], true, None),
            "daybreak-payload-enrollment-type" => (
                "not_granted",
                Vec::new(),
                false,
                Some(JsonValue::Bool(true)),
            ),
            _ => ("unknown", Vec::new(), false, None),
        };
    let result_checked_at = if mode == "daybreak-payload-checked-at" {
        "not-a-timestamp"
    } else {
        checked_at
    };
    let mut structured =
        daybreak_structured(status, &programs, result_checked_at, stale, enrollment_url);
    if mode == "daybreak-payload-schema-version" {
        structured.insert(
            "schemaVersion".to_string(),
            JsonValue::Number("2".to_string()),
        );
    } else if mode == "daybreak-payload-extra" {
        structured.insert("unexpected".to_string(), JsonValue::Null);
    } else if mode == "daybreak-payload-missing-status" {
        structured.remove("status");
    }
    let content_text = if mode == "daybreak-payload-text-mismatch" {
        "different advisory text".to_string()
    } else {
        daybreak_text(status, &programs, result_checked_at, stale)
    };
    let mut result = BTreeMap::from([
        (
            "content".to_string(),
            JsonValue::Array(vec![object([
                ("text", text_value(&content_text)),
                ("type", text_value("text")),
            ])]),
        ),
        (
            "isError".to_string(),
            JsonValue::Bool(mode == "daybreak-tool-error"),
        ),
        (
            "structuredContent".to_string(),
            JsonValue::Object(structured),
        ),
    ]);
    if mode == "daybreak-payload-missing-content" {
        result.remove("content");
    } else if mode == "daybreak-payload-missing-structured" {
        result.remove("structuredContent");
    } else if mode == "daybreak-payload-result-extra" {
        result.insert("unexpected".to_string(), JsonValue::Null);
    }
    JsonValue::Object(result)
}

fn start_result(mode: &str, paths: &FakePaths) -> JsonValue {
    let directory = paths.start_directory(mode);
    let instructions = match mode {
        "start-no-completion-instruction" => "No completion action is available".to_string(),
        "start-empty-instructions" => String::new(),
        _ => format!("Call {COMPLETE_TOOL} for {SCAN_ID}"),
    };
    let manifest = if mode == "start-wrong-manifest-path" {
        paths.other_manifest.clone()
    } else {
        directory.join(MANIFEST)
    };
    let mut structured = BTreeMap::from([
        ("instructions".to_string(), text_value(&instructions)),
        (
            "manifestPath".to_string(),
            if mode == "start-empty-manifest-path" {
                text_value("")
            } else {
                path_text(&manifest)
            },
        ),
        (
            "scanDir".to_string(),
            if mode == "start-empty-scan-directory" {
                text_value("")
            } else {
                path_text(directory)
            },
        ),
        ("scanId".to_string(), text_value(start_scan_id(mode))),
    ]);
    if mode == "start-receipt-extra" {
        structured.insert("unexpected".to_string(), JsonValue::Null);
    }
    let content_text = if mode == "start-text-mismatch" {
        "different instructions"
    } else {
        &instructions
    };
    let mut content_item = BTreeMap::from([
        ("text".to_string(), text_value(content_text)),
        (
            "type".to_string(),
            text_value(if mode == "start-content-wrong-type" {
                "image"
            } else if mode == "start-content-empty-type" {
                ""
            } else {
                "text"
            }),
        ),
    ]);
    if mode == "start-content-missing-type" {
        content_item.remove("type");
    } else if mode == "start-content-missing-text" {
        content_item.remove("text");
    } else if mode == "start-content-empty-text" {
        content_item.insert("text".to_string(), text_value(""));
    }
    let content_item = JsonValue::Object(content_item);
    let second_content_item = clone_json(&content_item);
    let content = match mode {
        "start-content-not-array" => object(std::iter::empty::<(&str, JsonValue)>()),
        "start-content-item-not-object" => JsonValue::Array(vec![text_value("not an object")]),
        "start-content-count" => JsonValue::Array(vec![content_item, second_content_item]),
        _ => JsonValue::Array(vec![content_item]),
    };
    let mut result = BTreeMap::from([
        ("content".to_string(), content),
        (
            "isError".to_string(),
            JsonValue::Bool(mode == "start-is-error"),
        ),
        (
            "structuredContent".to_string(),
            JsonValue::Object(structured),
        ),
    ]);
    if mode == "start-result-extra" {
        result.insert("unexpected".to_string(), JsonValue::Null);
    } else if mode == "start-content-missing" {
        result.remove("content");
    }
    JsonValue::Object(result)
}

const COMPLETED_SCAN: &str = r#"{
    "artifacts": {},
    "canceledAt": null,
    "continuationThreadId": null,
    "contract": {
        "diffTarget": null,
        "scope": {"requestedPath": ".", "requiredExcludePaths": [], "requiredIncludePaths": ["."]},
        "target": {
            "allowedKinds": ["git_revision"],
            "displayName": "repository",
            "targetId": "target-1"
        }
    },
    "diffTarget": null,
    "executionThreadIds": ["thread-1"],
    "failureMessage": null,
    "findingCount": 0,
    "findings": [],
    "findingsTruncated": false,
    "handoffClaimToken": null,
    "handoffClaimedAt": null,
    "handoffStatus": null,
    "mode": "deep",
    "progress": {
        "candidates": {"reportable": 0},
        "coverage": {"closedRows": 1, "filesTotal": 1, "worklistRows": 1},
        "independentReviews": {"active": 0, "completed": 4, "consolidating": false, "maximum": 40},
        "phase": "reporting",
        "phaseProgress": {"completed": 1, "total": 1, "unit": "report_artifacts"},
        "preflightIssues": [],
        "preflightProgress": {"completed": 1, "total": 1},
        "reviewPass": 1,
        "status": "complete",
        "updatedAt": "2026-10-14T00:00:00Z"
    },
    "reportAvailable": true,
    "remediationAvailable": true,
    "remediationUnavailableReason": null,
    "resultsRecoveryNeeded": false,
    "scope": ".",
    "targetSummary": null,
    "threadIds": ["thread-1", "thread-2"],
    "severityCounts": {},
    "updatedAt": "2026-10-14T00:00:01Z",
    "userContext": null,
    "warnings": []
}"#;

const COMPLETION: &str = r#"{
    "isError": false,
    "structuredContent": {
        "otherRunningDeepScans": [],
        "targetWarnings": [],
        "workspace": {
            "diffTarget": null,
            "mode": "deep",
            "scope": ".",
            "setup": {"submitted": true},
            "setupValidation": {"error": null, "valid": true},
            "targetSummary": null,
            "targetTitle": "repository",
            "updatedAt": "2026-10-14T00:00:00Z",
            "userContext": null
        }
    }
}"#;

fn edit_json(value: &mut JsonValue, pointer: &str, replacement: Option<JsonValue>) {
    let (parent, field) = pointer
        .rsplit_once('/')
        .expect("fixture pointer names a field");
    let mut node = value;
    for segment in parent.split('/').skip(1) {
        node = match node {
            JsonValue::Object(fields) => fields.get_mut(segment),
            JsonValue::Array(items) => segment
                .parse::<usize>()
                .ok()
                .and_then(|index| items.get_mut(index)),
            _ => None,
        }
        .unwrap_or_else(|| panic!("fixture pointer {pointer} is absent"));
    }
    match (node, replacement) {
        (JsonValue::Object(fields), Some(replacement)) => {
            fields.insert(field.to_string(), replacement);
        }
        (JsonValue::Object(fields), None) => {
            fields.remove(field);
        }
        (JsonValue::Array(items), Some(replacement)) => {
            let index = field.parse::<usize>().expect("fixture array index");
            if index == items.len() {
                items.push(replacement);
            } else {
                items[index] = replacement;
            }
        }
        _ => panic!("fixture pointer {pointer} has no editable parent"),
    }
}

fn complete_result(mode: &str, paths: &FakePaths, source: &SourceIdentity) -> JsonValue {
    let directory = if mode == "complete-changed-directory" {
        &paths.changed_scan_directory
    } else {
        paths.start_directory(mode)
    };
    let mut scan = parse_json(COMPLETED_SCAN.as_bytes(), "completed scan fixture").unwrap();
    let other_manifest = || Some(path_text(&paths.other_manifest));
    for (fault, pointer, value) in [
        (
            "",
            "/artifacts/coverage",
            Some(path_text(&directory.join(COVERAGE))),
        ),
        (
            "",
            "/artifacts/findings",
            Some(path_text(&directory.join(FINDINGS))),
        ),
        (
            "",
            "/artifacts/manifest",
            Some(path_text(&directory.join(MANIFEST))),
        ),
        (
            "",
            "/artifacts/markdownReport",
            Some(path_text(&directory.join(REPORT))),
        ),
        ("", "/model", Some(text_value(MODEL))),
        ("", "/reasoningEffort", Some(text_value(REASONING))),
        ("", "/scanDir", Some(path_text(directory))),
        ("", "/scanId", Some(text_value(start_scan_id(mode)))),
        ("", "/targetPath", Some(path_text(&paths.repository))),
        ("", "/targetRevision", Some(text_value(&source.commit))),
        (
            "artifact-coverage-path",
            "/artifacts/coverage",
            other_manifest(),
        ),
        (
            "artifact-findings-path",
            "/artifacts/findings",
            other_manifest(),
        ),
        (
            "artifact-manifest-path",
            "/artifacts/manifest",
            other_manifest(),
        ),
        (
            "artifact-report-path",
            "/artifacts/markdownReport",
            other_manifest(),
        ),
        (
            "artifact-schema",
            "/artifacts/unexpected",
            Some(JsonValue::Null),
        ),
        (
            "complete-in-progress",
            "/progress/status",
            Some(text_value("running")),
        ),
        (
            "complete-progress-extra",
            "/progress/unexpected",
            Some(JsonValue::Null),
        ),
        (
            "complete-execution-thread-unreported",
            "/executionThreadIds",
            Some(JsonValue::Array(vec![text_value("thread-3")])),
        ),
        (
            "complete-finding-count",
            "/findingCount",
            Some(JsonValue::Number("1".to_string())),
        ),
        (
            "complete-finding-row",
            "/findings",
            Some(JsonValue::Array(vec![object([(
                "findingId",
                text_value("finding-1"),
            )])])),
        ),
        ("complete-wrong-mode", "/mode", Some(text_value("quick"))),
        (
            "complete-wrong-model",
            "/model",
            Some(text_value("other-model")),
        ),
        (
            "complete-wrong-reasoning",
            "/reasoningEffort",
            Some(text_value("low")),
        ),
        (
            "complete-no-report",
            "/reportAvailable",
            Some(JsonValue::Bool(false)),
        ),
        (
            "complete-needs-recovery",
            "/resultsRecoveryNeeded",
            Some(JsonValue::Bool(true)),
        ),
        (
            "complete-other-scan",
            "/scanId",
            Some(text_value(OTHER_SCAN_ID)),
        ),
        ("complete-wrong-scope", "/scope", Some(text_value("src"))),
        (
            "complete-other-target",
            "/targetPath",
            Some(path_text(&paths.other_repository)),
        ),
        (
            "complete-other-revision",
            "/targetRevision",
            Some(text_value(&"c".repeat(40))),
        ),
        (
            "complete-scan-warning",
            "/warnings",
            Some(JsonValue::Array(vec![text_value("fixture warning")])),
        ),
        ("complete-scan-extra", "/unexpected", Some(JsonValue::Null)),
    ] {
        if fault.is_empty() || fault == mode {
            edit_json(&mut scan, pointer, value);
        }
    }
    let content = object([
        ("text", text_value(COMPLETE_CONTENT)),
        ("type", text_value("text")),
    ]);
    let workspace = |field: &str| format!("/structuredContent/workspace/{field}");
    let mut result = parse_json(COMPLETION.as_bytes(), "completion fixture").unwrap();
    for (fault, pointer, value) in [
        (
            "",
            "/content".to_string(),
            Some(JsonValue::Array(vec![clone_json(&content)])),
        ),
        (
            "",
            "/structuredContent/scan".to_string(),
            Some(clone_json(&scan)),
        ),
        ("", workspace("id"), Some(text_value(WORKSPACE_ID))),
        (
            "",
            workspace("targetPath"),
            Some(path_text(&paths.repository)),
        ),
        ("", workspace("results"), Some(scan)),
        (
            "workspace-setup-unsubmitted",
            workspace("setup/submitted"),
            Some(JsonValue::Bool(false)),
        ),
        (
            "workspace-setup-schema",
            workspace("setup/unexpected"),
            Some(JsonValue::Null),
        ),
        (
            "workspace-validation-error",
            workspace("setupValidation/error"),
            Some(text_value("fixture setup failure")),
        ),
        (
            "workspace-validation-invalid",
            workspace("setupValidation/valid"),
            Some(JsonValue::Bool(false)),
        ),
        (
            "workspace-validation-schema",
            workspace("setupValidation/unexpected"),
            Some(JsonValue::Null),
        ),
        (
            "workspace-other-target",
            workspace("targetPath"),
            Some(path_text(&paths.other_repository)),
        ),
        ("workspace-id", workspace("id"), Some(text_value("short"))),
        (
            "workspace-diff-target",
            workspace("diffTarget"),
            Some(text_value("HEAD")),
        ),
        (
            "workspace-mode",
            workspace("mode"),
            Some(text_value("standard")),
        ),
        (
            "workspace-scope",
            workspace("scope"),
            Some(text_value("src")),
        ),
        (
            "workspace-target-summary",
            workspace("targetSummary"),
            Some(text_value("stale summary")),
        ),
        (
            "workspace-wrong-title",
            workspace("targetTitle"),
            Some(text_value("other")),
        ),
        (
            "workspace-updated-at",
            workspace("updatedAt"),
            Some(text_value("not-a-timestamp")),
        ),
        (
            "workspace-user-context",
            workspace("userContext"),
            Some(text_value("stale context")),
        ),
        (
            "workspace-results",
            workspace("results"),
            Some(JsonValue::Object(BTreeMap::new())),
        ),
        (
            "workspace-schema-extra",
            workspace("unexpected"),
            Some(JsonValue::Null),
        ),
        ("workspace-schema-missing", workspace("userContext"), None),
        (
            "complete-other-running",
            "/structuredContent/otherRunningDeepScans".to_string(),
            Some(JsonValue::Array(vec![text_value(OTHER_RUNNING_SCAN_ID)])),
        ),
        (
            "complete-target-warning",
            "/structuredContent/targetWarnings".to_string(),
            Some(JsonValue::Array(vec![text_value("fixture target warning")])),
        ),
        (
            "complete-context-extra",
            "/structuredContent/unexpected".to_string(),
            Some(JsonValue::Null),
        ),
        (
            "complete-parent-scan",
            "/structuredContent/parentScanId".to_string(),
            Some(text_value(PARENT_SCAN_ID)),
        ),
        (
            "complete-recipe",
            "/structuredContent/recipe".to_string(),
            Some(text_value("fixture-recipe")),
        ),
        (
            "complete-content-wrong-text",
            "/content/0/text".to_string(),
            Some(text_value("Completion was not indexed.")),
        ),
        (
            "complete-content-wrong-type",
            "/content/0/type".to_string(),
            Some(text_value("image")),
        ),
        (
            "complete-content-missing-type",
            "/content/0/type".to_string(),
            None,
        ),
        (
            "complete-content-missing-text",
            "/content/0/text".to_string(),
            None,
        ),
        (
            "complete-content-extra",
            "/content/0/unexpected".to_string(),
            Some(JsonValue::Null),
        ),
        (
            "complete-content-not-array",
            "/content".to_string(),
            Some(JsonValue::Object(BTreeMap::new())),
        ),
        (
            "complete-content-item-not-object",
            "/content".to_string(),
            Some(JsonValue::Array(vec![text_value("not an object")])),
        ),
        (
            "complete-content-count",
            "/content/1".to_string(),
            Some(content),
        ),
        (
            "complete-is-error",
            "/isError".to_string(),
            Some(JsonValue::Bool(true)),
        ),
        (
            "complete-result-extra",
            "/unexpected".to_string(),
            Some(JsonValue::Null),
        ),
    ] {
        if fault.is_empty() || fault == mode {
            edit_json(&mut result, &pointer, value);
        }
    }
    result
}

fn send_initialize_response(writer: &mut impl Write, mode: &str, paths: &FakePaths) -> Result<()> {
    if mode == "malformed-json" {
        issue_fault(paths, mode)?;
        writer.write_all(b"{not-json}\n")?;
        writer.flush()?;
        return Ok(());
    }
    let mut value = match mode {
        "wrong-id" => response("other", initialize_result(mode)),
        "error-and-result" => {
            let JsonValue::Object(mut response) = response("initialize", initialize_result(mode))
            else {
                unreachable!()
            };
            response.insert(
                "error".to_string(),
                object([
                    ("code", JsonValue::Number("-32000".to_string())),
                    ("message", text_value("fixture error")),
                ]),
            );
            JsonValue::Object(response)
        }
        _ => response("initialize", initialize_result(mode)),
    };
    if mode == "initialize-response-extra" {
        let JsonValue::Object(response) = &mut value else {
            unreachable!()
        };
        response.insert("unexpected".to_string(), JsonValue::Null);
    } else if mode == "initialize-wrong-jsonrpc" {
        let JsonValue::Object(response) = &mut value else {
            unreachable!()
        };
        response.insert("jsonrpc".to_string(), text_value("1.0"));
    }
    if mode == "oversized-line" {
        let JsonValue::Object(response) = &mut value else {
            unreachable!()
        };
        let JsonValue::Object(result) = response.get_mut("result").unwrap() else {
            unreachable!()
        };
        result.insert(
            "padding".to_string(),
            text_value(&"x".repeat(MAX_MCP_LINE_BYTES)),
        );
    }
    if is_initialize_fault(mode) {
        issue_fault(paths, mode)?;
    }
    if mode == "crlf-response" {
        writer.write_all(render_json(&value)?.as_bytes())?;
        writer.write_all(b"\r\n")?;
        writer.flush()?;
        return Ok(());
    }
    send_value(writer, &value)
}

fn send_daybreak_response(writer: &mut impl Write, mode: &str, paths: &FakePaths) -> Result<bool> {
    if mode == "daybreak-premature-eof" {
        issue_fault(paths, mode)?;
        return Ok(false);
    }
    if mode == "daybreak-malformed-json" {
        issue_fault(paths, mode)?;
        writer.write_all(b"{not-json}\n")?;
        writer.flush()?;
        return Ok(true);
    }
    let valid_error = object([
        ("code", JsonValue::Number("-32000".to_string())),
        ("data", object([("retryable", JsonValue::Bool(false))])),
        ("message", text_value("fixture advisory unavailable")),
    ]);
    let mut value = match mode {
        "daybreak-wrong-id" => response("other", daybreak_result(mode)),
        "daybreak-jsonrpc-error" => jsonrpc_error_response("daybreak", valid_error),
        "daybreak-error-null" => jsonrpc_error_response("daybreak", JsonValue::Null),
        "daybreak-error-missing-code" => jsonrpc_error_response(
            "daybreak",
            object([("message", text_value("fixture advisory unavailable"))]),
        ),
        "daybreak-error-float-code" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("-32000.0".to_string())),
                ("message", text_value("fixture advisory unavailable")),
            ]),
        ),
        "daybreak-error-code-overflow" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("9223372036854775808".to_string())),
                ("message", text_value("fixture advisory unavailable")),
            ]),
        ),
        "daybreak-error-code-noncanonical" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("-0".to_string())),
                ("message", text_value("fixture advisory unavailable")),
            ]),
        ),
        "daybreak-error-message-type" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("-32000".to_string())),
                ("message", JsonValue::Bool(true)),
            ]),
        ),
        "daybreak-error-message-empty" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("-32000".to_string())),
                ("message", text_value("")),
            ]),
        ),
        "daybreak-error-message-control" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("-32000".to_string())),
                ("message", text_value("fixture\tadvisory unavailable")),
            ]),
        ),
        "daybreak-error-message-unicode-control" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("-32000".to_string())),
                ("message", text_value("fixture\u{0085}advisory unavailable")),
            ]),
        ),
        "daybreak-error-extra" => jsonrpc_error_response(
            "daybreak",
            object([
                ("code", JsonValue::Number("-32000".to_string())),
                ("message", text_value("fixture advisory unavailable")),
                ("unexpected", JsonValue::Null),
            ]),
        ),
        "daybreak-error-and-result" => {
            let JsonValue::Object(mut response) = response("daybreak", daybreak_result(mode))
            else {
                unreachable!()
            };
            response.insert("error".to_string(), valid_error);
            JsonValue::Object(response)
        }
        _ => response("daybreak", daybreak_result(mode)),
    };
    if mode == "daybreak-response-extra" {
        let JsonValue::Object(response) = &mut value else {
            unreachable!()
        };
        response.insert("unexpected".to_string(), JsonValue::Null);
    } else if mode == "daybreak-wrong-jsonrpc" {
        let JsonValue::Object(response) = &mut value else {
            unreachable!()
        };
        response.insert("jsonrpc".to_string(), text_value("1.0"));
    } else if mode == "daybreak-oversized-line" {
        let JsonValue::Object(response) = &mut value else {
            unreachable!()
        };
        let JsonValue::Object(result) = response.get_mut("result").unwrap() else {
            unreachable!()
        };
        result.insert(
            "padding".to_string(),
            text_value(&"x".repeat(MAX_MCP_LINE_BYTES)),
        );
    }
    if mode.starts_with("daybreak-") {
        issue_fault(paths, mode)?;
    }
    if mode == "daybreak-crlf-response" {
        writer.write_all(render_json(&value)?.as_bytes())?;
        writer.write_all(b"\r\n")?;
        writer.flush()?;
        return Ok(true);
    }
    send_value(writer, &value)?;
    Ok(true)
}

fn exchange_server_messages(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    mode: &str,
    paths: &FakePaths,
) -> Result<()> {
    let mut notification = BTreeMap::from([
        ("jsonrpc".to_string(), text_value("2.0")),
        (
            "method".to_string(),
            text_value(if mode == "server-unknown-notification" {
                "notifications/unknown"
            } else {
                "notifications/progress"
            }),
        ),
        (
            "params".to_string(),
            object([
                ("message", text_value("fixture")),
                ("progress", JsonValue::Number("1".to_string())),
                ("progressToken", text_value("fixture-progress")),
                ("total", JsonValue::Number("2".to_string())),
            ]),
        ),
    ]);
    if mode == "server-notification-result" {
        notification.insert(
            "result".to_string(),
            object(std::iter::empty::<(&str, JsonValue)>()),
        );
    } else if mode == "server-notification-error" {
        notification.insert(
            "error".to_string(),
            object([
                ("code", JsonValue::Number("-32000".to_string())),
                ("message", text_value("smuggled error")),
            ]),
        );
    }
    if is_notification_fault(mode) {
        issue_fault(paths, mode)?;
    }
    send_value(writer, &JsonValue::Object(notification))?;

    let mut ping = BTreeMap::from([
        (
            "id".to_string(),
            if mode == "server-ping-bad-id" {
                JsonValue::Bool(true)
            } else {
                JsonValue::Number("17".to_string())
            },
        ),
        ("jsonrpc".to_string(), text_value("2.0")),
        (
            "method".to_string(),
            text_value(if mode == "server-unsupported-request" {
                "roots/list"
            } else {
                "ping"
            }),
        ),
    ]);
    if mode == "server-ping-extra" {
        ping.insert(
            "params".to_string(),
            object(std::iter::empty::<(&str, JsonValue)>()),
        );
    }
    if is_ping_fault(mode) {
        issue_fault(paths, mode)?;
    }
    send_value(writer, &JsonValue::Object(ping))?;
    if mode == "server-ping-bad-id" {
        read_value(reader, CLIENT, "bad ping response")?;
    } else {
        expect_value(
            reader,
            object([
                ("id", JsonValue::Number("17".to_string())),
                ("jsonrpc", text_value("2.0")),
                ("result", object(std::iter::empty::<(&str, JsonValue)>())),
            ]),
            CLIENT,
            "ping response",
        )?;
    }
    Ok(())
}

fn fake_mcp_main(mode: &str, paths: &FakePaths) -> Result<()> {
    let source = source();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();
    writer.write_all(READY)?;
    writer.flush()?;

    expect_value(
        &mut reader,
        expected_initialize_request(),
        CLIENT,
        "initialize request",
    )?;
    if mode == "premature-eof" {
        issue_fault(paths, mode)?;
        return Ok(());
    }
    send_initialize_response(&mut writer, mode, paths)?;

    expect_value(
        &mut reader,
        expected_initialized_notification(),
        CLIENT,
        "initialized notification",
    )?;
    expect_value(
        &mut reader,
        expected_tools_request(),
        CLIENT,
        "tools/list request",
    )?;
    exchange_server_messages(&mut reader, &mut writer, mode, paths)?;
    let tools = response("tools", tools_result(mode, paths));
    if mode.starts_with("tools-") {
        issue_fault(paths, mode)?;
    }
    send_value(&mut writer, &tools)?;

    expect_value(
        &mut reader,
        expected_daybreak_request(&paths.repository, &source),
        CLIENT,
        "Daybreak advisory request",
    )?;
    if !send_daybreak_response(&mut writer, mode, paths)? {
        return Ok(());
    }

    expect_value(
        &mut reader,
        expected_start_request(&paths.repository, &source),
        CLIENT,
        "start request",
    )?;
    let start = response("start", start_result(mode, paths));
    if mode.starts_with("start-") {
        issue_fault(paths, mode)?;
    }
    send_value(&mut writer, &start)?;

    expect_value(
        &mut reader,
        expected_complete_request(mode, &paths.repository, &source),
        CLIENT,
        "complete request",
    )?;
    if mode == "start-missing-manifest" {
        std::fs::write(paths.scan_directory.join(MANIFEST), b"late manifest\n")?;
    }
    let completion_id = if mode == "duplicate-id" {
        "start"
    } else {
        "complete"
    };
    let completion = response(completion_id, complete_result(mode, paths, &source));
    if is_completion_fault(mode) {
        issue_fault(paths, mode)?;
    }
    send_value(&mut writer, &completion)?;
    if mode == "trailing-output" {
        issue_fault(paths, mode)?;
        send_value(
            &mut writer,
            &object([
                ("jsonrpc", text_value("2.0")),
                ("method", text_value("notifications/trailing")),
            ]),
        )?;
    }

    let mut trailing_input = Vec::new();
    reader.read_to_end(&mut trailing_input)?;
    ensure!(
        trailing_input.is_empty(),
        "fake MCP client sent trailing input"
    );
    Ok(())
}

struct FakeRun {
    result: Result<(McpOutcome, PathBuf)>,
    fault_issued: Option<String>,
    _fixture: Fixture,
}

fn run_fake(mode: &str) -> Result<FakeRun> {
    let fixture = Fixture::new(mode)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--exact",
            "security_deep::tests::protocol::fake_mcp_child",
            "--ignored",
            "--nocapture",
            "--quiet",
        ])
        .env(FAKE_MODE_ENV, mode)
        .env(FAKE_ROOT_ENV, &fixture.paths.root);
    let repository = fixture.paths.repository.clone();
    let live_auth = live_auth_path(&repository);
    let projected_auth = projected_auth_path(&repository);
    let worker_policy = test_worker_policy(&repository, &live_auth, &projected_auth);
    let scan_root = fixture.paths.scan_root.clone();
    let expected_directory = fixture.paths.scan_directory.clone();
    let source = source();
    let result = (|| {
        let output = crate::bounded_process::output_dialogue(
            command,
            Duration::from_secs(5),
            "fake Codex Security MCP",
            DialogueLimits {
                line_bytes: MAX_MCP_LINE_BYTES,
                stdout_bytes: MAX_MCP_STDOUT_BYTES,
                stderr_bytes: MAX_MCP_STDERR_BYTES,
            },
            || Ok(()),
            move |dialogue| {
                wait_until_ready(dialogue, READY, "fake MCP test executable")?;
                exchange_mcp(dialogue, &worker_policy, &source, &scan_root)
            },
        )?;
        ensure!(output.status.success(), "fake MCP child failed");
        ensure!(
            std::str::from_utf8(&output.stderr).is_ok(),
            "fake MCP child diagnostics are not UTF-8"
        );
        Ok((output.value, expected_directory))
    })();
    let fault_issued = read_fault_marker(&fixture.paths)?;
    Ok(FakeRun {
        result,
        fault_issued,
        _fixture: fixture,
    })
}

fn assert_rejected(cases: &[(&str, &str)]) {
    for (mode, expected) in cases {
        let run = run_fake(mode).unwrap();
        let error = run.result.as_ref().unwrap_err();
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains(expected),
            "mode {mode} returned the wrong error; expected `{expected}` in {rendered}"
        );
        assert_eq!(
            run.fault_issued.as_deref(),
            Some(*mode),
            "mode {mode} rejected before the fake server issued its intended fault"
        );
    }
}

#[cfg(windows)]
fn assert_advisory_non_gating(modes: &[&str]) {
    for mode in modes {
        let run = run_fake(mode).unwrap();
        let (outcome, expected_directory) = run
            .result
            .unwrap_or_else(|error| panic!("mode {mode} unexpectedly gated the scan: {error:#}"));
        assert_eq!(outcome.scan_id, SCAN_ID);
        assert_eq!(outcome.scan_directory, expected_directory);
        assert_eq!(run.fault_issued.as_deref(), Some(*mode));
    }
}

#[cfg(windows)]
#[test]
fn owned_mcp_exchange_emits_the_exact_five_step_protocol_and_handles_server_messages() {
    let run = run_fake("happy").unwrap();
    assert_eq!(run.fault_issued, None);
    let (outcome, expected_directory) = run.result.unwrap();
    assert_eq!(outcome.scan_id, SCAN_ID);
    assert_eq!(outcome.scan_directory, expected_directory);
    assert_eq!(
        outcome.thread_ids,
        BTreeSet::from(["thread-1".to_string(), "thread-2".to_string()])
    );
    assert_eq!(
        outcome.execution_threads,
        BTreeSet::from(["thread-1".to_string()])
    );
}

#[test]
fn owned_mcp_initialization_and_tool_authority_are_exact() {
    assert_rejected(&[
        (
            "initialize-response-extra",
            "Codex Security MCP response has an unexpected field",
        ),
        ("initialize-wrong-jsonrpc", "another protocol version"),
        ("initialize-envelope-extra", "initialize schema differs"),
        (
            "initialize-missing-capabilities",
            "initialize schema differs",
        ),
        ("initialize-server-info-extra", "initialize schema differs"),
        ("initialize-wrong-protocol", "initialize schema differs"),
        ("initialize-wrong-name", "initialize schema differs"),
        ("initialize-wrong-version", "initialize schema differs"),
        ("tools-cursor", "unexpectedly paginated"),
        ("tools-not-array", "MCP tool list is not an array"),
        ("tools-wrong-name", "tool inventory differs"),
        ("tools-missing", "tool inventory differs"),
        ("tools-duplicate", "tool inventory differs"),
        ("tools-extra", "tool inventory differs"),
        ("tools-reordered", "tool inventory differs"),
        ("tools-schema-drift", "tool inventory differs"),
        ("tools-daybreak-missing", "tool inventory differs"),
        ("tools-daybreak-duplicate", "tool inventory differs"),
        ("tools-daybreak-schema-drift", "tool inventory differs"),
        (
            "tools-result-schema-drift",
            "MCP tools result has an unexpected field",
        ),
    ]);
}

#[cfg(windows)]
#[test]
fn owned_daybreak_advisory_is_once_immediately_before_start_and_non_gating() {
    assert_advisory_non_gating(&[
        "daybreak-unknown",
        "daybreak-stale",
        "daybreak-granted-blue",
        "daybreak-granted-red",
        "daybreak-granted-both",
        "daybreak-not-granted",
        "daybreak-not-granted-url",
        "daybreak-not-granted-url-port",
        "daybreak-not-granted-unsafe-url",
        "daybreak-not-granted-invalid-authority",
        "daybreak-not-granted-invalid-escape",
        "daybreak-jsonrpc-error",
        "daybreak-tool-error",
        "daybreak-payload-missing-content",
        "daybreak-payload-missing-structured",
        "daybreak-payload-result-extra",
        "daybreak-payload-schema-version",
        "daybreak-payload-extra",
        "daybreak-payload-missing-status",
        "daybreak-payload-status",
        "daybreak-payload-program",
        "daybreak-payload-duplicate-program",
        "daybreak-payload-status-programs",
        "daybreak-payload-checked-at",
        "daybreak-payload-stale-status",
        "daybreak-payload-enrollment-type",
        "daybreak-payload-text-mismatch",
    ]);
}

#[cfg(windows)]
#[test]
fn owned_daybreak_wire_and_error_envelopes_fail_closed() {
    assert_rejected(&[
        ("daybreak-wrong-id", "unexpected ID"),
        ("daybreak-error-and-result", "exactly one result or error"),
        (
            "daybreak-response-extra",
            "response has an unexpected field",
        ),
        ("daybreak-wrong-jsonrpc", "another protocol version"),
        ("daybreak-malformed-json", "Codex Security MCP response"),
        ("daybreak-crlf-response", "not strict LF framing"),
        ("daybreak-oversized-line", "line longer than 1048576 bytes"),
        ("daybreak-premature-eof", "closed before its response"),
        ("daybreak-error-null", "MCP error is not an object"),
        ("daybreak-error-missing-code", "has no `code` field"),
        ("daybreak-error-float-code", "not a signed integer"),
        ("daybreak-error-code-overflow", "not a signed integer"),
        ("daybreak-error-code-noncanonical", "not canonical"),
        (
            "daybreak-error-message-type",
            "error message is not a string",
        ),
        ("daybreak-error-message-empty", "error message is malformed"),
        (
            "daybreak-error-message-control",
            "error message is malformed",
        ),
        (
            "daybreak-error-message-unicode-control",
            "error message is malformed",
        ),
        ("daybreak-error-extra", "MCP error has an unexpected field"),
    ]);
}

#[test]
fn daybreak_payload_and_presentation_are_exact() {
    let parse = |mode| parse_daybreak_advisory(&daybreak_result(mode)).unwrap();
    let render = |mode| {
        let presentation = daybreak_presentation(&parse(mode));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        write_daybreak_presentation(presentation.as_ref(), &mut stdout, &mut stderr).unwrap();
        (
            presentation,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    };
    for mode in ["daybreak-unknown", "daybreak-stale"] {
        assert_eq!(render(mode), (None, String::new(), String::new()));
    }
    let granted = "[xtask] Codex Security Daybreak access: status=granted; programs=Daybreak Blue, Daybreak Red";
    assert_eq!(
        render("daybreak-granted-both"),
        (
            Some(DaybreakPresentation::Standard(granted.to_string())),
            format!("{granted}\n"),
            String::new()
        )
    );
    for (mode, url) in [
        (
            "daybreak-not-granted-url",
            "https://chatgpt.com/cyber/apply",
        ),
        (
            "daybreak-not-granted-url-port",
            "https://chatgpt.com:443/cyber/apply?from=deep#access",
        ),
    ] {
        let warning = format!(
            "[xtask] WARNING: Codex Security Daybreak access is not granted for this ChatGPT account; protected scan outputs may not be displayable. Apply at {url}"
        );
        assert_eq!(
            render(mode),
            (
                Some(DaybreakPresentation::Warning(warning.clone())),
                String::new(),
                format!("{warning}\n")
            )
        );
    }
    for mode in [
        "daybreak-not-granted",
        "daybreak-not-granted-unsafe-url",
        "daybreak-not-granted-invalid-authority",
        "daybreak-not-granted-invalid-escape",
    ] {
        let warning = format!(
            "[xtask] WARNING: Codex Security Daybreak access is not granted for this ChatGPT account; protected scan outputs may not be displayable. Apply at {DAYBREAK_ENROLLMENT_FALLBACK}"
        );
        assert_eq!(
            render(mode),
            (
                Some(DaybreakPresentation::Warning(warning.clone())),
                String::new(),
                format!("{warning}\n")
            )
        );
    }
    for mode in [
        "daybreak-tool-error",
        "daybreak-payload-missing-content",
        "daybreak-payload-missing-structured",
        "daybreak-payload-result-extra",
        "daybreak-payload-schema-version",
        "daybreak-payload-extra",
        "daybreak-payload-missing-status",
        "daybreak-payload-status",
        "daybreak-payload-program",
        "daybreak-payload-duplicate-program",
        "daybreak-payload-status-programs",
        "daybreak-payload-checked-at",
        "daybreak-payload-stale-status",
        "daybreak-payload-enrollment-type",
        "daybreak-payload-text-mismatch",
    ] {
        assert!(
            parse_daybreak_advisory(&daybreak_result(mode)).is_err(),
            "{mode}"
        );
    }
}

#[test]
fn daybreak_operator_stream_failures_are_non_gating() {
    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fixture operator stream is closed",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fixture operator stream is closed",
            ))
        }
    }

    for presentation in [
        DaybreakPresentation::Standard("fixture standard notice".to_string()),
        DaybreakPresentation::Warning("fixture warning notice".to_string()),
    ] {
        let mut stdout = BrokenWriter;
        let mut stderr = BrokenWriter;
        present_daybreak_with_writers(Some(&presentation), &mut stdout, &mut stderr);
    }
}

#[test]
fn owned_mcp_server_messages_have_strict_envelopes() {
    assert_rejected(&[
        ("server-unknown-notification", "unsupported notification"),
        ("server-notification-result", "notification schema differs"),
        ("server-notification-error", "notification schema differs"),
        ("server-ping-bad-id", "ping ID has an unsupported type"),
        ("server-ping-extra", "ping schema differs"),
        ("server-unsupported-request", "unsupported client action"),
    ]);
}

#[cfg(windows)]
#[test]
fn owned_mcp_response_correlation_and_transport_fail_closed() {
    assert_rejected(&[
        ("wrong-id", "unexpected ID"),
        ("duplicate-id", "unexpected ID"),
        ("error-and-result", "exactly one result or error"),
        ("start-is-error", "Deep start returned a tool error"),
        ("complete-is-error", "completion returned a tool error"),
        ("malformed-json", "Codex Security MCP response"),
        ("crlf-response", "not strict LF framing"),
        ("oversized-line", "line longer than 1048576 bytes"),
        ("premature-eof", "closed before its response"),
    ]);
}

#[cfg(windows)]
#[test]
fn owned_mcp_start_receipt_is_exact_source_bound_and_actionable() {
    assert_rejected(&[
        (
            "start-result-extra",
            "Codex Security Deep start has an unexpected field",
        ),
        (
            "start-receipt-extra",
            "Codex Security Deep start receipt schema differs",
        ),
        ("start-empty-scan-id", "`scanId` is empty"),
        ("start-empty-scan-directory", "`scanDir` is empty"),
        ("start-empty-manifest-path", "`manifestPath` is empty"),
        ("start-empty-instructions", "`instructions` is empty"),
        ("start-content-missing", "has no `content` field"),
        ("start-content-not-array", "Deep start is not an array"),
        (
            "start-content-count",
            "Codex Security Deep start content schema differs",
        ),
        (
            "start-content-item-not-object",
            "Deep start is not an object",
        ),
        (
            "start-content-wrong-type",
            "Codex Security Deep start text differs",
        ),
        (
            "start-content-missing-type",
            "Codex Security Deep start schema differs",
        ),
        (
            "start-content-missing-text",
            "Codex Security Deep start schema differs",
        ),
        (
            "start-content-empty-type",
            "Codex Security Deep start text differs",
        ),
        (
            "start-content-empty-text",
            "Codex Security Deep start text differs",
        ),
        (
            "start-text-mismatch",
            "Codex Security Deep start text differs",
        ),
        (
            "start-no-completion-instruction",
            "omitted the completion instruction",
        ),
        (
            "start-escaped-directory",
            "escaped its reserved two-level root",
        ),
        (
            "start-wrong-revision-directory",
            "not bound to the requested revision",
        ),
        ("start-wrong-manifest-path", "noncanonical manifest path"),
        ("start-missing-manifest", "scan-manifest.json"),
    ]);
}

#[cfg(windows)]
#[test]
fn owned_mcp_completion_is_exact_source_bound_and_clean() {
    assert_rejected(&[
        (
            "complete-result-extra",
            "Codex Security completion has an unexpected field",
        ),
        (
            "complete-context-extra",
            "Codex Security completion context has an unexpected field",
        ),
        ("complete-other-running", "another Codex Security Deep scan"),
        ("complete-parent-scan", "continuation or recipe-backed"),
        ("complete-recipe", "continuation or recipe-backed"),
        (
            "complete-scan-extra",
            "completed scan has an unexpected field",
        ),
        (
            "complete-progress-extra",
            "completion progress schema differs",
        ),
        (
            "complete-execution-thread-unreported",
            "execution threads differ from its thread set",
        ),
        ("complete-other-scan", "returned another scan"),
        ("complete-changed-directory", "directory changed"),
        ("complete-other-target", "belongs to another target"),
        (
            "complete-other-revision",
            "differs from the requested deep profile",
        ),
        (
            "complete-wrong-mode",
            "differs from the requested deep profile",
        ),
        (
            "complete-wrong-scope",
            "differs from the requested deep profile",
        ),
        (
            "complete-wrong-model",
            "differs from the requested deep profile",
        ),
        (
            "complete-wrong-reasoning",
            "differs from the requested deep profile",
        ),
        ("complete-in-progress", "is not terminal reporting"),
        ("complete-no-report", "has no final report"),
        ("complete-needs-recovery", "needs result recovery"),
        ("complete-target-warning", "has target warnings"),
        ("complete-scan-warning", "completion has warnings"),
        ("complete-finding-count", "reports findings"),
        ("complete-finding-row", "reports findings"),
        (
            "complete-content-not-array",
            "Codex Security completion is not an array",
        ),
        (
            "complete-content-item-not-object",
            "Codex Security completion is not an object",
        ),
        (
            "complete-content-count",
            "Codex Security completion content schema differs",
        ),
        (
            "complete-content-missing-type",
            "Codex Security completion schema differs",
        ),
        (
            "complete-content-missing-text",
            "Codex Security completion schema differs",
        ),
        (
            "complete-content-wrong-type",
            "Codex Security completion text differs",
        ),
        (
            "complete-content-wrong-text",
            "Codex Security completion text differs",
        ),
        (
            "complete-content-extra",
            "Codex Security completion schema differs",
        ),
        ("artifact-schema", "artifacts schema differs"),
        ("artifact-coverage-path", "noncanonical coverage artifact"),
        ("artifact-findings-path", "noncanonical findings artifact"),
        ("artifact-manifest-path", "noncanonical manifest artifact"),
        (
            "artifact-report-path",
            "noncanonical markdownReport artifact",
        ),
        ("artifact-missing", "report.md"),
        (
            "trailing-output",
            "sent output after its final completion response",
        ),
    ]);
}

#[cfg(windows)]
#[test]
fn owned_mcp_completion_workspace_is_an_exact_fresh_results_clone() {
    assert_rejected(&[
        ("workspace-schema-extra", "workspace schema differs"),
        ("workspace-schema-missing", "workspace schema differs"),
        ("workspace-id", "workspace identity is not a canonical UUID"),
        ("workspace-updated-at", "UTC timestamp has no Z suffix"),
        ("workspace-mode", "differs from a fresh owned scan"),
        ("workspace-scope", "differs from a fresh owned scan"),
        ("workspace-diff-target", "differs from a fresh owned scan"),
        (
            "workspace-target-summary",
            "differs from a fresh owned scan",
        ),
        ("workspace-user-context", "differs from a fresh owned scan"),
        ("workspace-results", "differs from a fresh owned scan"),
        ("workspace-other-target", "belongs to another target"),
        ("workspace-wrong-title", "belongs to another target"),
        ("workspace-setup-schema", "workspace setup schema differs"),
        (
            "workspace-setup-unsubmitted",
            "workspace setup was not submitted",
        ),
        (
            "workspace-validation-schema",
            "workspace validation schema differs",
        ),
        (
            "workspace-validation-error",
            "workspace validation did not succeed",
        ),
        (
            "workspace-validation-invalid",
            "workspace validation did not succeed",
        ),
    ]);
}

#[test]
#[ignore]
fn fake_mcp_child() {
    let Some(mode) = std::env::var_os(FAKE_MODE_ENV) else {
        return;
    };
    let Some(root) = std::env::var_os(FAKE_ROOT_ENV) else {
        return;
    };
    let mode = mode.to_string_lossy();
    let paths = FakePaths::from_root(PathBuf::from(root));
    let exit = match fake_mcp_main(&mode, &paths) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("fake MCP child failed: {error:#}");
            1
        }
    };
    std::process::exit(exit);
}
