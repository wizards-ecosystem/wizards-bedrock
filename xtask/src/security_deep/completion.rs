use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StartReceipt {
    scan_id: String,
    scan_directory: PathBuf,
}

pub(super) fn call_start(
    dialogue: &mut LineDialogue,
    worker_policy: &ManagedWorkerPolicy,
    source: &SourceIdentity,
    scan_root: &Path,
) -> Result<StartReceipt> {
    let repository = worker_policy.repository.as_path();
    let arguments = object([
        ("scope", text_value(".")),
        ("targetPath", path_value(repository)),
    ]);
    let params = object([
        ("_meta", call_metadata(worker_policy, source)?),
        ("arguments", arguments),
        ("name", text_value(START_TOOL)),
    ]);
    send_request(dialogue, "start", "tools/call", params)?;
    let result = receive_response(dialogue, "start")?;
    let result = require_tool_success(&result, "Codex Security Deep start")?;
    let structured = json_object(
        json_field(result, "structuredContent", "Codex Security Deep start")?,
        "Codex Security Deep start receipt",
    )?;
    require_json_fields(
        structured,
        &["scanId", "scanDir", "manifestPath", "instructions"],
        "Codex Security Deep start receipt",
    )?;
    let scan_id = nonempty_string(structured, "scanId", "Codex Security Deep start receipt")?;
    require_uuid(scan_id, "Codex Security Deep scan identity")?;
    let scan_directory = canonical_directory(
        Path::new(nonempty_string(
            structured,
            "scanDir",
            "Codex Security Deep start receipt",
        )?),
        "Codex Security scan directory",
    )?;
    require_scan_directory_shape(scan_root, repository, source, &scan_directory)?;
    let manifest = PathBuf::from(nonempty_string(
        structured,
        "manifestPath",
        "Codex Security Deep start receipt",
    )?);
    ensure!(
        manifest == scan_directory.join(MANIFEST),
        "Codex Security Deep start returned a noncanonical manifest path"
    );
    crate::mutate::workspace::open_real_file(&manifest, "Codex Security start manifest")?;
    let instructions = nonempty_string(
        structured,
        "instructions",
        "Codex Security Deep start receipt",
    )?;
    ensure!(
        instructions.contains(COMPLETE_TOOL),
        "Codex Security Deep start omitted the completion instruction"
    );
    require_single_text_content(result, instructions, "Codex Security Deep start")?;
    Ok(StartReceipt {
        scan_id: scan_id.to_string(),
        scan_directory,
    })
}

pub(super) fn call_complete(
    dialogue: &mut LineDialogue,
    worker_policy: &ManagedWorkerPolicy,
    source: &SourceIdentity,
    scan_root: &Path,
    start: &StartReceipt,
) -> Result<McpOutcome> {
    let repository = worker_policy.repository.as_path();
    let arguments = object([("scanId", text_value(&start.scan_id))]);
    let params = object([
        ("_meta", call_metadata(worker_policy, source)?),
        ("arguments", arguments),
        ("name", text_value(COMPLETE_TOOL)),
    ]);
    send_request(dialogue, "complete", "tools/call", params)?;
    let result = receive_response(dialogue, "complete")?;
    let result = require_tool_success(&result, "Codex Security completion")?;
    require_single_text_content(result, COMPLETE_CONTENT, "Codex Security completion")?;
    let structured = json_object(
        json_field(result, "structuredContent", "Codex Security completion")?,
        "Codex Security completion context",
    )?;
    let other_running = json_array(
        json_field(
            structured,
            "otherRunningDeepScans",
            "Codex Security completion context",
        )?,
        "other running Codex Security Deep scans",
    )?;
    ensure!(
        other_running.is_empty(),
        "another Codex Security Deep scan is running"
    );
    ensure!(
        !structured.contains_key("parentScanId") && !structured.contains_key("recipe"),
        "Codex Security completion is a continuation or recipe-backed scan"
    );
    require_allowed_fields(
        structured,
        &[
            "otherRunningDeepScans",
            "parentScanId",
            "recipe",
            "scan",
            "targetWarnings",
            "workspace",
        ],
        "Codex Security completion context",
    )?;
    let target_warnings = json_array(
        json_field(
            structured,
            "targetWarnings",
            "Codex Security completion context",
        )?,
        "Codex Security target warnings",
    )?;
    ensure!(
        target_warnings.is_empty(),
        "Codex Security completion has target warnings"
    );
    let scan = json_object(
        json_field(structured, "scan", "Codex Security completion context")?,
        "Codex Security completed scan",
    )?;
    require_allowed_fields(
        scan,
        &[
            "artifacts",
            "canceledAt",
            "continuationThreadId",
            "contract",
            "cost",
            "diffTarget",
            "executionThreadIds",
            "failureMessage",
            "findingCount",
            "findings",
            "findingsTruncated",
            "handoffClaimToken",
            "handoffClaimedAt",
            "handoffStatus",
            "mode",
            "model",
            "progress",
            "reasoningEffort",
            "remediationAvailable",
            "remediationUnavailableReason",
            "reportAvailable",
            "resultsRecoveryNeeded",
            "scanDir",
            "scanId",
            "scope",
            "severityCounts",
            "targetPath",
            "targetRevision",
            "targetSummary",
            "threadIds",
            "threatModelAvailable",
            "threatModelPath",
            "threatModelProvenance",
            "updatedAt",
            "usage",
            "userContext",
            "warnings",
        ],
        "Codex Security completed scan",
    )?;
    require_fresh_workspace(structured, scan, repository)?;
    ensure!(
        string(scan, "scanId", "Codex Security completed scan")? == start.scan_id,
        "Codex Security completion returned another scan"
    );
    let scan_directory = canonical_directory(
        Path::new(nonempty_string(
            scan,
            "scanDir",
            "Codex Security completed scan",
        )?),
        "completed Codex Security scan directory",
    )?;
    ensure!(
        scan_directory == start.scan_directory,
        "Codex Security scan directory changed between start and completion"
    );
    require_scan_directory_shape(scan_root, repository, source, &scan_directory)?;
    ensure!(
        canonical_directory(
            Path::new(nonempty_string(
                scan,
                "targetPath",
                "Codex Security completed scan",
            )?),
            "Codex Security completed target",
        )? == repository,
        "Codex Security completion belongs to another target"
    );
    ensure!(
        string(scan, "targetRevision", "Codex Security completed scan")? == source.commit
            && string(scan, "mode", "Codex Security completed scan")? == "deep"
            && string(scan, "scope", "Codex Security completed scan")? == "."
            && string(scan, "model", "Codex Security completed scan")? == MODEL
            && string(scan, "reasoningEffort", "Codex Security completed scan")? == REASONING,
        "Codex Security completion differs from the requested deep profile"
    );
    require_completion_contract(scan, repository)?;
    require_null_fields(
        scan,
        &[
            "canceledAt",
            "continuationThreadId",
            "diffTarget",
            "failureMessage",
            "handoffClaimToken",
            "handoffClaimedAt",
            "handoffStatus",
            "remediationUnavailableReason",
            "targetSummary",
            "userContext",
        ],
        "Codex Security completed scan",
    )?;
    ensure!(
        !boolean(scan, "findingsTruncated", "Codex Security completed scan")?
            && boolean(
                scan,
                "remediationAvailable",
                "Codex Security completed scan"
            )?,
        "Codex Security completion is truncated, canceled, or not remediable"
    );
    ensure!(
        json_object(
            json_field(scan, "severityCounts", "Codex Security completed scan")?,
            "Codex Security completion severity counts",
        )?
        .is_empty(),
        "Codex Security completion has nonzero or unexpected severity counts"
    );
    let thread_ids = require_thread_id_list(scan, "threadIds")?;
    let execution_threads = require_thread_id_list(scan, "executionThreadIds")?;
    require_worker_thread_sets(&thread_ids, &execution_threads)?;
    let scan_updated = crate::mutants_grade::parse_utc_timestamp(
        nonempty_string(scan, "updatedAt", "Codex Security completed scan")?,
        "Codex Security completion update time",
    )?;
    let progress = json_object(
        json_field(scan, "progress", "Codex Security completed scan")?,
        "Codex Security completion progress",
    )?;
    require_json_fields(
        progress,
        &[
            "candidates",
            "coverage",
            "independentReviews",
            "phase",
            "phaseProgress",
            "preflightIssues",
            "preflightProgress",
            "reviewPass",
            "status",
            "updatedAt",
        ],
        "Codex Security completion progress",
    )?;
    require_completion_progress(progress, scan_updated)?;
    ensure!(
        boolean(scan, "reportAvailable", "Codex Security completed scan")?
            && !boolean(
                scan,
                "resultsRecoveryNeeded",
                "Codex Security completed scan"
            )?,
        "Codex Security completion has no final report or needs result recovery"
    );
    ensure!(
        json_array(
            json_field(scan, "warnings", "Codex Security completed scan")?,
            "Codex Security completion warnings"
        )?
        .is_empty(),
        "Codex Security completion has warnings"
    );
    ensure!(
        json_u64(
            json_field(scan, "findingCount", "Codex Security completed scan")?,
            "Codex Security finding count"
        )? == 0
            && json_array(
                json_field(scan, "findings", "Codex Security completed scan")?,
                "Codex Security completion findings"
            )?
            .is_empty(),
        "Codex Security completion reports findings"
    );
    require_completion_artifacts(scan, &scan_directory)?;
    Ok(McpOutcome {
        scan_id: start.scan_id.clone(),
        scan_directory,
        thread_ids,
        execution_threads,
    })
}

fn require_null_fields(
    object: &BTreeMap<String, JsonValue>,
    fields: &[&str],
    label: &str,
) -> Result<()> {
    for field in fields {
        ensure!(
            matches!(json_field(object, field, label)?, JsonValue::Null),
            "{label} {field} is not null"
        );
    }
    Ok(())
}

fn require_completion_contract(
    scan: &BTreeMap<String, JsonValue>,
    repository: &Path,
) -> Result<()> {
    let contract = json_object(
        json_field(scan, "contract", "Codex Security completed scan")?,
        "Codex Security Deep contract",
    )?;
    require_json_fields(
        contract,
        &["diffTarget", "scope", "target"],
        "Codex Security Deep contract",
    )?;
    ensure!(
        matches!(contract.get("diffTarget"), Some(JsonValue::Null)),
        "Codex Security Deep contract is diff-backed"
    );
    let scope = json_object(
        json_field(contract, "scope", "Codex Security Deep contract")?,
        "Codex Security Deep contract scope",
    )?;
    require_json_fields(
        scope,
        &[
            "requestedPath",
            "requiredExcludePaths",
            "requiredIncludePaths",
        ],
        "Codex Security Deep contract scope",
    )?;
    ensure!(
        string(scope, "requestedPath", "Codex Security Deep contract scope")? == "."
            && json_string_array(
                json_field(
                    scope,
                    "requiredIncludePaths",
                    "Codex Security Deep contract scope",
                )?,
                "Codex Security Deep contract includes",
            )? == ["."]
            && json_string_array(
                json_field(
                    scope,
                    "requiredExcludePaths",
                    "Codex Security Deep contract scope",
                )?,
                "Codex Security Deep contract excludes",
            )?
            .is_empty(),
        "Codex Security Deep contract does not cover the complete repository"
    );
    let target = json_object(
        json_field(contract, "target", "Codex Security Deep contract")?,
        "Codex Security Deep contract target",
    )?;
    require_json_fields(
        target,
        &["allowedKinds", "displayName", "targetId"],
        "Codex Security Deep contract target",
    )?;
    let repository_name = repository
        .file_name()
        .and_then(OsStr::to_str)
        .context("Codex Security repository name is not UTF-8")?;
    ensure!(
        json_string_array(
            json_field(
                target,
                "allowedKinds",
                "Codex Security Deep contract target"
            )?,
            "Codex Security Deep allowed target kinds",
        )? == ["git_revision"]
            && string(target, "displayName", "Codex Security Deep contract target")?
                == repository_name,
        "Codex Security Deep contract belongs to another target"
    );
    nonempty_string(target, "targetId", "Codex Security Deep contract target")?;
    Ok(())
}

fn require_thread_id_list(
    scan: &BTreeMap<String, JsonValue>,
    field: &str,
) -> Result<BTreeSet<String>> {
    let values = json_string_array(
        json_field(scan, field, "Codex Security completed scan")?,
        &format!("Codex Security completion {field}"),
    )?;
    ensure!(
        !values.is_empty(),
        "Codex Security completion {field} is empty"
    );
    let mut unique = BTreeSet::new();
    for value in values {
        require_safe_identity(value, &format!("Codex Security completion {field}"))?;
        ensure!(
            unique.insert(value.to_string()),
            "Codex Security completion {field} is duplicated"
        );
    }
    Ok(unique)
}

fn require_completion_progress(
    progress: &BTreeMap<String, JsonValue>,
    scan_updated: (u32, u32, u32, u32, u32, u32, u32),
) -> Result<()> {
    ensure!(
        string(progress, "status", "Codex Security completion progress")? == "complete"
            && string(progress, "phase", "Codex Security completion progress")? == "reporting",
        "Codex Security completion progress is not terminal reporting"
    );
    let updated = crate::mutants_grade::parse_utc_timestamp(
        nonempty_string(progress, "updatedAt", "Codex Security completion progress")?,
        "Codex Security progress update time",
    )?;
    ensure!(
        updated <= scan_updated,
        "Codex Security progress update is after the scan update"
    );
    let candidates = json_object(
        json_field(progress, "candidates", "Codex Security completion progress")?,
        "Codex Security candidate progress",
    )?;
    require_json_fields(
        candidates,
        &["reportable"],
        "Codex Security candidate progress",
    )?;
    ensure!(
        json_u64(
            json_field(
                candidates,
                "reportable",
                "Codex Security candidate progress"
            )?,
            "Codex Security reportable candidate count",
        )? == 0,
        "Codex Security progress retains reportable candidates"
    );
    let coverage = json_object(
        json_field(progress, "coverage", "Codex Security completion progress")?,
        "Codex Security coverage progress",
    )?;
    require_json_fields(
        coverage,
        &["closedRows", "filesTotal", "worklistRows"],
        "Codex Security coverage progress",
    )?;
    let closed = json_u64(
        json_field(coverage, "closedRows", "Codex Security coverage progress")?,
        "Codex Security closed review rows",
    )?;
    let worklist = json_u64(
        json_field(coverage, "worklistRows", "Codex Security coverage progress")?,
        "Codex Security worklist rows",
    )?;
    ensure!(
        closed == worklist
            && json_u64(
                json_field(coverage, "filesTotal", "Codex Security coverage progress")?,
                "Codex Security scope file count",
            )? > 0,
        "Codex Security coverage progress is incomplete"
    );
    require_completed_counter(
        progress,
        "phaseProgress",
        Some("report_artifacts"),
        "Codex Security phase progress",
    )?;
    require_completed_counter(
        progress,
        "preflightProgress",
        None,
        "Codex Security preflight progress",
    )?;
    ensure!(
        json_array(
            json_field(
                progress,
                "preflightIssues",
                "Codex Security completion progress"
            )?,
            "Codex Security preflight issues",
        )?
        .is_empty(),
        "Codex Security preflight has unresolved issues"
    );
    ensure!(
        json_u64(
            json_field(progress, "reviewPass", "Codex Security completion progress")?,
            "Codex Security Deep review pass",
        )? > 0,
        "Codex Security Deep performed no reducer review pass"
    );
    let independent = json_object(
        json_field(
            progress,
            "independentReviews",
            "Codex Security completion progress",
        )?,
        "Codex Security independent reviews",
    )?;
    require_json_fields(
        independent,
        &["active", "completed", "consolidating", "maximum"],
        "Codex Security independent reviews",
    )?;
    let completed = json_u64(
        json_field(
            independent,
            "completed",
            "Codex Security independent reviews",
        )?,
        "Codex Security completed independent reviews",
    )?;
    ensure!(
        json_u64(
            json_field(independent, "active", "Codex Security independent reviews")?,
            "Codex Security active independent reviews",
        )? == 0
            && (4..=40).contains(&completed)
            && json_u64(
                json_field(independent, "maximum", "Codex Security independent reviews")?,
                "Codex Security maximum independent reviews",
            )? == 40
            && !boolean(
                independent,
                "consolidating",
                "Codex Security independent reviews",
            )?,
        "Codex Security independent review series is incomplete or inconsistent"
    );
    Ok(())
}

fn require_completed_counter(
    progress: &BTreeMap<String, JsonValue>,
    field: &str,
    unit: Option<&str>,
    label: &str,
) -> Result<()> {
    let counter = json_object(json_field(progress, field, label)?, label)?;
    let fields: &[&str] = if unit.is_some() {
        &["completed", "total", "unit"]
    } else {
        &["completed", "total"]
    };
    require_json_fields(counter, fields, label)?;
    let completed = json_u64(json_field(counter, "completed", label)?, label)?;
    let total = json_u64(json_field(counter, "total", label)?, label)?;
    ensure!(completed == total, "{label} is incomplete");
    if let Some(unit) = unit {
        ensure!(
            completed > 0 && string(counter, "unit", label)? == unit,
            "{label} differs from the terminal report-artifact counter"
        );
    }
    Ok(())
}

fn require_fresh_workspace(
    completion: &BTreeMap<String, JsonValue>,
    scan: &BTreeMap<String, JsonValue>,
    repository: &Path,
) -> Result<()> {
    let repository_name = repository
        .file_name()
        .and_then(OsStr::to_str)
        .context("Codex Security repository name is not UTF-8")?;
    let workspace = json_object(
        json_field(completion, "workspace", "Codex Security completion context")?,
        "Codex Security completion workspace",
    )?;
    require_json_fields(
        workspace,
        &[
            "id",
            "diffTarget",
            "mode",
            "scope",
            "setup",
            "setupValidation",
            "targetPath",
            "targetSummary",
            "targetTitle",
            "updatedAt",
            "userContext",
            "results",
        ],
        "Codex Security completion workspace",
    )?;
    let workspace_id = nonempty_string(workspace, "id", "Codex Security completion workspace")?;
    require_uuid(workspace_id, "Codex Security completion workspace identity")?;
    crate::mutants_grade::parse_utc_timestamp(
        nonempty_string(
            workspace,
            "updatedAt",
            "Codex Security completion workspace",
        )?,
        "Codex Security completion workspace update time",
    )?;
    ensure!(
        string(workspace, "mode", "Codex Security completion workspace")? == "deep"
            && string(workspace, "scope", "Codex Security completion workspace")? == "."
            && matches!(workspace.get("diffTarget"), Some(JsonValue::Null))
            && matches!(workspace.get("targetSummary"), Some(JsonValue::Null))
            && matches!(workspace.get("userContext"), Some(JsonValue::Null))
            && json_object(
                json_field(workspace, "results", "Codex Security completion workspace")?,
                "Codex Security completion workspace results"
            )? == scan,
        "Codex Security completion workspace differs from a fresh owned scan"
    );
    ensure!(
        canonical_directory(
            Path::new(nonempty_string(
                workspace,
                "targetPath",
                "Codex Security completion workspace",
            )?),
            "Codex Security completion workspace target",
        )? == repository
            && string(
                workspace,
                "targetTitle",
                "Codex Security completion workspace"
            )? == repository_name,
        "Codex Security completion workspace belongs to another target"
    );
    let setup = json_object(
        json_field(workspace, "setup", "Codex Security completion workspace")?,
        "Codex Security workspace setup",
    )?;
    require_json_fields(setup, &["submitted"], "Codex Security workspace setup")?;
    ensure!(
        boolean(setup, "submitted", "Codex Security workspace setup")?,
        "Codex Security workspace setup was not submitted"
    );
    let validation = json_object(
        json_field(
            workspace,
            "setupValidation",
            "Codex Security completion workspace",
        )?,
        "Codex Security workspace validation",
    )?;
    require_json_fields(
        validation,
        &["error", "valid"],
        "Codex Security workspace validation",
    )?;
    ensure!(
        matches!(validation.get("error"), Some(JsonValue::Null))
            && boolean(validation, "valid", "Codex Security workspace validation")?,
        "Codex Security workspace validation did not succeed"
    );
    Ok(())
}

pub(super) fn require_uuid(value: &str, label: &str) -> Result<()> {
    let bytes = value.as_bytes();
    ensure!(
        bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => *byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            })
            && matches!(bytes[14].to_ascii_lowercase(), b'1'..=b'8')
            && matches!(bytes[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b'),
        "{label} is not a canonical UUID"
    );
    Ok(())
}

fn require_completion_artifacts(
    scan: &BTreeMap<String, JsonValue>,
    directory: &Path,
) -> Result<()> {
    let artifacts = json_object(
        json_field(scan, "artifacts", "Codex Security completed scan")?,
        "Codex Security completion artifacts",
    )?;
    require_json_fields(
        artifacts,
        &["coverage", "findings", "manifest", "markdownReport"],
        "Codex Security completion artifacts",
    )?;
    for (key, name) in [
        ("coverage", COVERAGE),
        ("findings", FINDINGS),
        ("manifest", MANIFEST),
        ("markdownReport", REPORT),
    ] {
        let actual = PathBuf::from(string(
            artifacts,
            key,
            "Codex Security completion artifacts",
        )?);
        ensure!(
            actual == directory.join(name),
            "Codex Security completion returned a noncanonical {key} artifact"
        );
        crate::mutate::workspace::open_real_file(&actual, "Codex Security completion artifact")?;
    }
    Ok(())
}

fn require_scan_directory_shape(
    scan_root: &Path,
    repository: &Path,
    source: &SourceIdentity,
    directory: &Path,
) -> Result<()> {
    let target_root = directory
        .parent()
        .context("Codex Security scan directory has no target parent")?;
    ensure!(
        target_root.parent() == Some(scan_root)
            && target_root.file_name() == repository.file_name(),
        "Codex Security scan directory escaped its reserved two-level root"
    );
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Codex Security scan directory name is not UTF-8")?;
    ensure!(
        name.starts_with(&format!("{}_", source.commit)),
        "Codex Security scan directory is not bound to the requested revision"
    );
    Ok(())
}

pub(super) fn managed_read_only_sandbox_state(policy: &ManagedWorkerPolicy) -> Result<JsonValue> {
    ensure!(
        policy.repository.is_absolute()
            && policy.live_auth.is_absolute()
            && policy.projected_auth.is_absolute()
            && policy.live_auth != policy.projected_auth
            && policy.live_auth.file_name() == Some(OsStr::new("auth.json"))
            && policy.projected_auth.file_name() == Some(OsStr::new("auth.json"))
            && !policy
                .readable_entries
                .iter()
                .any(|entry| policy.denied_canary.starts_with(entry)),
        "managed worker sandbox paths differ from the owned credential policy"
    );
    let mut entries = vec![
        object([
            ("access", text_value("deny")),
            (
                "path",
                object([
                    ("type", text_value("special")),
                    ("value", object([("kind", text_value("root"))])),
                ]),
            ),
        ]),
        object([
            ("access", text_value("read")),
            (
                "path",
                object([
                    ("type", text_value("special")),
                    ("value", object([("kind", text_value("minimal"))])),
                ]),
            ),
        ]),
    ];
    entries.extend(policy.readable_entries.iter().map(|path| {
        object([
            ("access", text_value("read")),
            (
                "path",
                object([("path", path_value(path)), ("type", text_value("path"))]),
            ),
        ])
    }));
    entries.extend(policy.denied_entries.iter().map(|path| {
        object([
            ("access", text_value("deny")),
            (
                "path",
                object([("path", path_value(path)), ("type", text_value("path"))]),
            ),
        ])
    }));
    entries.extend([&policy.live_auth, &policy.projected_auth].map(|path| {
        object([
            ("access", text_value("deny")),
            (
                "path",
                object([("path", path_value(path)), ("type", text_value("path"))]),
            ),
        ])
    }));
    Ok(object([
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
            text_value(&windows_file_url(&policy.repository)?),
        ),
        ("useMxc", JsonValue::Bool(true)),
    ]))
}

pub(super) fn provider_parent_sandbox_state(policy: &ManagedWorkerPolicy) -> Result<JsonValue> {
    ensure!(
        policy.repository.is_absolute()
            && policy.live_auth.is_absolute()
            && policy.projected_auth.is_absolute()
            && policy.live_auth != policy.projected_auth
            && policy.live_auth.file_name() == Some(OsStr::new("auth.json"))
            && policy.projected_auth.file_name() == Some(OsStr::new("auth.json")),
        "provider parent sandbox paths differ from the owned credential policy"
    );
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
    entries.extend(complete_worker_denies(policy).into_iter().map(|path| {
        object([
            ("access", text_value("deny")),
            (
                "path",
                object([("path", path_value(path)), ("type", text_value("path"))]),
            ),
        ])
    }));
    Ok(object([
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
            text_value(&windows_file_url(&policy.repository)?),
        ),
        ("useMxc", JsonValue::Bool(true)),
    ]))
}

pub(super) fn call_metadata(
    policy: &ManagedWorkerPolicy,
    source: &SourceIdentity,
) -> Result<JsonValue> {
    Ok(object([
        (
            "codex/sandbox-state-meta",
            provider_parent_sandbox_state(policy)?,
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
    ]))
}

#[cfg(test)]
pub(super) fn test_worker_policy(
    repository: &Path,
    live_auth: &Path,
    projected_auth: &Path,
) -> ManagedWorkerPolicy {
    let parent = repository.parent().unwrap_or(repository);
    ManagedWorkerPolicy {
        repository: repository.to_path_buf(),
        probe_directory: parent.to_path_buf(),
        readable_entries: vec![repository.to_path_buf()],
        denied_entries: vec![repository.join(".git")],
        readable_probes: Vec::new(),
        denied_probes: Vec::new(),
        denied_write_probes: vec![
            repository.join(WORKER_SCAN_WRITE_CANARY_NAME),
            parent.join(WORKER_OUTSIDE_WRITE_CANARY_NAME),
        ],
        existing_write_probe: repository.join(WORKER_ALLOWED_CANARY_NAME),
        repository_metadata_probe: repository.join(".git").join("HEAD"),
        live_auth: live_auth.to_path_buf(),
        projected_auth: projected_auth.to_path_buf(),
        denied_canary: parent.join("worker-denied-read-canary"),
    }
}
