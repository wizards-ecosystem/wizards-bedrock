use super::*;

// Invalidates any legacy PASS record before reading unsigned provider output: its consistency
// helps the operator, but caller-selected bytes have no release authority.
pub(super) fn verify(paths: &Paths, scan_directory: &Path) -> Result<()> {
    let attempt = qualification::begin_security_deep(paths, true)?;
    let request = read(
        &attempt.directory().join(REQUEST),
        MAX_REQUEST_BYTES,
        "Codex Security request",
    )?;
    verify_request(&request, attempt.source())?;
    let scan_directory = canonical_directory(scan_directory, "Codex Security scan directory")?;
    let manifest = read(
        &scan_directory.join(MANIFEST),
        MAX_MANIFEST_BYTES,
        "Codex Security manifest",
    )?;
    let findings = read(
        &scan_directory.join(FINDINGS),
        MAX_FINDINGS_BYTES,
        "Codex Security findings",
    )?;
    let coverage = read(
        &scan_directory.join(COVERAGE),
        MAX_COVERAGE_BYTES,
        "Codex Security coverage",
    )?;
    let report = read(
        &scan_directory.join(REPORT),
        MAX_REPORT_BYTES,
        "Codex Security report",
    )?;
    let scan = verify_scan(&manifest, &findings, &coverage, &report, attempt.source())?;
    println!(
        "[xtask] unsigned {} {} scan {} is structurally consistent \
         (surfaces={}, findings={}/{}/{}/{}/{}); no qualification PASS was created",
        scan.producer_name,
        scan.producer_version,
        scan.id,
        scan.surfaces,
        scan.critical,
        scan.high,
        scan.medium,
        scan.low,
        scan.informational,
    );
    Ok(())
}

pub(super) fn verify_scan(
    manifest: &[u8],
    findings: &[u8],
    coverage: &[u8],
    report: &[u8],
    source: &SourceIdentity,
) -> Result<Scan> {
    let report =
        std::str::from_utf8(report).context("Codex Security provider report is not UTF-8")?;
    ensure!(
        !report.trim().is_empty(),
        "Codex Security provider report is empty"
    );
    let manifest_value = parse_json(manifest, "Codex Security manifest")?;
    let manifest_root = json_object(&manifest_value, "Codex Security manifest")?;
    require_document(manifest_root, "codex-security.scan-manifest")?;
    let scan = json_object(
        json_field(manifest_root, "scan", "scan manifest")?,
        "scan manifest receipt",
    )?;
    require_strict_scan_profile(scan)?;
    let id = nonempty_string(scan, "id", "scan manifest")?;
    require_uuid(id, "Codex Security scan identity")?;
    ensure!(
        string(scan, "status", "scan manifest")? == "completed",
        "Codex Security scan did not complete"
    );
    ensure!(
        string(scan, "coverageRef", "scan manifest")? == COVERAGE
            && string(scan, "findingsRef", "scan manifest")? == FINDINGS,
        "Codex Security manifest has unexpected canonical references"
    );
    let producer_name = PRODUCER_NAME.to_string();
    let producer_version = PRODUCER_VERSION.to_string();
    let target = json_object(json_field(scan, "target", "scan manifest")?, "scan target")?;
    require_json_fields(
        target,
        &["kind", "targetId", "displayName", "revision"],
        "scan target",
    )?;
    nonempty_string(target, "targetId", "scan target")?;
    nonempty_string(target, "displayName", "scan target")?;
    ensure!(
        string(target, "kind", "scan target")? == "git_revision",
        "Codex Security scan target is not an immutable Git revision"
    );
    ensure!(
        string(target, "revision", "scan target")? == source.commit,
        "Codex Security scan belongs to another commit"
    );
    let scope = require_scope(
        json_field(scan, "scope", "scan manifest")?,
        "scan manifest scope",
    )?;
    let threat_model = require_threat_model(json_field(scan, "threatModel", "scan manifest")?)?;
    require_preserved_sources(json_field(scan, "preservedSources", "scan manifest")?)?;
    verify_manifest_artifacts(
        json_field(scan, "artifacts", "scan manifest")?,
        findings,
        coverage,
    )?;
    let counts = verify_findings(findings, id)?;
    let surfaces = verify_coverage(coverage, id)?;
    require_zero_finding_report(report, target, scope, threat_model, source)?;
    Ok(Scan {
        id: id.to_string(),
        producer_name,
        producer_version,
        surfaces,
        critical: counts["critical"],
        high: counts["high"],
        medium: counts["medium"],
        low: counts["low"],
        informational: counts["informational"],
    })
}

fn verify_findings(findings: &[u8], id: &str) -> Result<BTreeMap<&'static str, u64>> {
    let findings_value = parse_json(findings, "Codex Security findings")?;
    let findings_root = json_object(&findings_value, "Codex Security findings")?;
    require_json_fields(
        findings_root,
        &["documentType", "schemaVersion", "scanId", "findings"],
        "Codex Security findings",
    )?;
    require_document(findings_root, "codex-security.findings")?;
    ensure!(
        string(findings_root, "scanId", "findings")? == id,
        "Codex Security findings belong to another scan"
    );
    let mut finding_ids = BTreeSet::new();
    let mut fingerprints = BTreeSet::new();
    let mut counts = BTreeMap::from([
        ("critical", 0u64),
        ("high", 0u64),
        ("medium", 0u64),
        ("low", 0u64),
        ("informational", 0u64),
    ]);
    for finding in json_array(
        json_field(findings_root, "findings", "findings")?,
        "Codex Security finding list",
    )? {
        let finding = json_object(finding, "Codex Security finding")?;
        let finding_id = nonempty_string(finding, "findingId", "finding")?;
        ensure!(
            finding_ids.insert(finding_id.to_string()),
            "Codex Security finding ID is duplicated"
        );
        let fingerprint = json_object(
            json_field(finding, "fingerprints", "finding")?,
            "finding fingerprints",
        )?;
        ensure!(
            string(fingerprint, "algorithm", "finding fingerprints")? == "codex-security/v1",
            "Codex Security finding fingerprint algorithm is unsupported"
        );
        let primary = nonempty_string(fingerprint, "primary", "finding fingerprints")?;
        ensure!(
            fingerprints.insert(primary.to_string()),
            "Codex Security finding fingerprint is duplicated"
        );
        let severity = json_object(
            json_field(finding, "severity", "finding")?,
            "finding severity",
        )?;
        let level = string(severity, "level", "finding severity")?;
        let count = counts
            .get_mut(level)
            .with_context(|| format!("unknown Codex Security severity `{level}`"))?;
        *count += 1;
    }
    ensure!(
        counts.values().all(|count| *count == 0),
        "Codex Security scan has reportable findings without a repository disposition"
    );
    Ok(counts)
}

fn verify_coverage(coverage: &[u8], id: &str) -> Result<u64> {
    let coverage_value = parse_json(coverage, "Codex Security coverage")?;
    let coverage_root = json_object(&coverage_value, "Codex Security coverage")?;
    require_json_fields(
        coverage_root,
        &[
            "documentType",
            "schemaVersion",
            "scanId",
            "mode",
            "completeness",
            "inventoryStrategy",
            "includePaths",
            "excludePaths",
            "surfaces",
            "explicitExclusions",
            "deferred",
            "openQuestions",
        ],
        "Codex Security coverage",
    )?;
    require_document(coverage_root, "codex-security.coverage")?;
    ensure!(
        string(coverage_root, "scanId", "coverage")? == id,
        "Codex Security coverage belongs to another scan"
    );
    ensure!(
        string(coverage_root, "mode", "coverage")? == "deep_repository"
            && string(coverage_root, "inventoryStrategy", "coverage")? == "repository"
            && string(coverage_root, "completeness", "coverage")? == "complete",
        "Codex Security coverage is not the complete deep repository workflow"
    );
    require_scope_value(coverage_root, "Codex Security coverage")?;
    ensure!(
        json_array(
            json_field(coverage_root, "explicitExclusions", "coverage")?,
            "coverage exclusions"
        )?
        .is_empty(),
        "Codex Security coverage has explicit exclusions"
    );
    ensure!(
        json_array(
            json_field(coverage_root, "deferred", "coverage")?,
            "coverage deferred work"
        )?
        .is_empty(),
        "Codex Security coverage has deferred work"
    );
    ensure!(
        json_array(
            json_field(coverage_root, "openQuestions", "coverage")?,
            "coverage open questions"
        )?
        .is_empty(),
        "Codex Security coverage has open questions"
    );
    let surfaces = json_array(
        json_field(coverage_root, "surfaces", "coverage")?,
        "coverage surfaces",
    )?;
    ensure!(
        surfaces.is_empty(),
        "Codex Security deep aggregate has noncanonical surface rows"
    );
    Ok(surfaces.len() as u64)
}

fn require_zero_finding_report(
    report: &str,
    target: &BTreeMap<String, JsonValue>,
    scope: &BTreeMap<String, JsonValue>,
    threat_model: &BTreeMap<String, JsonValue>,
    source: &SourceIdentity,
) -> Result<()> {
    ensure!(
        report.ends_with('\n'),
        "Codex Security provider report has no terminal newline"
    );
    let mut lines = vec![
        format!(
            "# Security Review: {}",
            report_text(
                string(target, "displayName", "scan target")?,
                "Unknown target"
            )?
        ),
        String::new(),
        "## Scope".to_string(),
        String::new(),
        report_text(
            optional_nonempty_string(scope, "summary", "scan manifest scope")?.unwrap_or(""),
            "The scan was configured for the include paths and exclusions listed below.",
        )?,
        String::new(),
        "- Scan mode: deep_repository".to_string(),
        "- Target kind: git_revision".to_string(),
        format!(
            "- Target ID: {}",
            report_text(string(target, "targetId", "scan target")?, "not recorded")?
        ),
        format!("- Revision: {}", report_text(&source.commit, "")?),
        "- Inventory strategy: repository".to_string(),
        "- Included paths: .".to_string(),
        "- Excluded paths: none".to_string(),
    ];
    let artifacts_reviewed = report_strings(
        scope.get("artifactsReviewed"),
        "scan manifest artifacts reviewed",
    )?;
    if !artifacts_reviewed.is_empty() {
        lines.push(format!(
            "- Artifacts reviewed: {}",
            artifacts_reviewed.join(", ")
        ));
    }
    let context = report_text(
        optional_nonempty_string(scope, "context", "scan manifest scope")?.unwrap_or(""),
        "",
    )?;
    if !context.is_empty() {
        lines.push(format!("- Scan context: {context}"));
    }
    let limitations = report_strings(scope.get("limitations"), "scan manifest limitations")?;
    if !limitations.is_empty() {
        lines.extend([String::new(), "Limitations and exclusions:".to_string()]);
        lines.extend(limitations.into_iter().map(|item| format!("- {item}")));
    }
    lines.extend([
        String::new(),
        "### Scan Summary".to_string(),
        String::new(),
        "| Field | Value |".to_string(),
        "| --- | --- |".to_string(),
        "| Scan outcome | completed |".to_string(),
        "| Reportable DSS findings | 0 |".to_string(),
        "| Report instances | 0 |".to_string(),
        "| Report severity mix | none |".to_string(),
        "| Report confidence mix | none |".to_string(),
        "| Coverage | complete |".to_string(),
        format!(
            "| Validation mode | {} |",
            report_cell(
                optional_nonempty_string(scope, "validationMode", "scan manifest scope")?
                    .unwrap_or("not recorded")
            )?
        ),
        String::new(),
        "Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.".to_string(),
        String::new(),
        "## Threat Model".to_string(),
        String::new(),
    ]);
    if threat_model.contains_key("format") {
        lines.push(string(threat_model, "content", "scan manifest threat model")?.to_string());
    } else {
        lines.push(report_text(
            string(threat_model, "summary", "scan manifest threat model")?,
            "No explicit canonical threat-model summary was recorded.",
        )?);
        for (heading, field) in [
            ("Assets", "assets"),
            ("Trust Boundaries", "trustBoundaries"),
            ("Attacker Capabilities", "attackerCapabilities"),
            ("Security Objectives", "securityObjectives"),
            ("Assumptions", "assumptions"),
        ] {
            let values = report_strings(
                threat_model.get(field),
                &format!("scan manifest threat model {field}"),
            )?;
            if !values.is_empty() {
                lines.extend([String::new(), format!("### {heading}"), String::new()]);
                lines.extend(values.into_iter().map(|item| format!("- {item}")));
            }
        }
    }
    lines.extend([
        String::new(),
        "## Findings".to_string(),
        String::new(),
        "### No findings".to_string(),
        String::new(),
        "No reportable findings survived the canonical discovery, validation, and reportability gates."
            .to_string(),
    ]);
    let expected = format!("{}\n", lines.join("\n"));
    ensure!(
        report == expected,
        "Codex Security provider report is not the exact pinned zero-finding projection"
    );
    Ok(())
}

fn require_strict_scan_profile(scan: &BTreeMap<String, JsonValue>) -> Result<()> {
    require_json_fields(
        scan,
        &[
            "id",
            "producer",
            "status",
            "startedAt",
            "completedAt",
            "sealedAt",
            "target",
            "scope",
            "threatModel",
            "preservedSources",
            "coverageRef",
            "findingsRef",
            "artifacts",
        ],
        "scan manifest",
    )?;
    let producer = json_object(
        json_field(scan, "producer", "scan manifest")?,
        "scan producer",
    )?;
    require_json_fields(producer, &["name", "version"], "scan producer")?;
    ensure!(
        string(producer, "name", "scan producer")? == PRODUCER_NAME
            && string(producer, "version", "scan producer")? == PRODUCER_VERSION,
        "Codex Security producer differs from the qualified provider"
    );
    let started = crate::mutants_grade::parse_utc_timestamp(
        string(scan, "startedAt", "scan manifest")?,
        "Codex Security start time",
    )?;
    let completed = crate::mutants_grade::parse_utc_timestamp(
        string(scan, "completedAt", "scan manifest")?,
        "Codex Security completion time",
    )?;
    let sealed = crate::mutants_grade::parse_utc_timestamp(
        string(scan, "sealedAt", "scan manifest")?,
        "Codex Security seal time",
    )?;
    ensure!(
        started <= completed && completed <= sealed,
        "Codex Security timestamps are out of order"
    );
    Ok(())
}

fn require_document(root: &BTreeMap<String, JsonValue>, document: &str) -> Result<()> {
    ensure!(
        string(root, "documentType", document)? == document
            && string(root, "schemaVersion", document)? == "1.0",
        "unsupported {document} schema"
    );
    Ok(())
}

fn require_scope<'json>(
    value: &'json JsonValue,
    label: &str,
) -> Result<&'json BTreeMap<String, JsonValue>> {
    let scope = json_object(value, label)?;
    require_allowed_json_fields(
        scope,
        &["includePaths", "excludePaths"],
        &[
            "includePaths",
            "excludePaths",
            "summary",
            "artifactsReviewed",
            "runtimeStatus",
            "validationMode",
            "context",
            "limitations",
        ],
        label,
    )?;
    require_scope_value(scope, label)?;
    for field in ["summary", "runtimeStatus", "validationMode", "context"] {
        optional_nonempty_string(scope, field, label)?;
    }
    for field in ["artifactsReviewed", "limitations"] {
        if let Some(value) = scope.get(field) {
            require_nonempty_string_array(value, label)?;
        }
    }
    Ok(scope)
}

fn require_threat_model(value: &JsonValue) -> Result<&BTreeMap<String, JsonValue>> {
    let label = "scan manifest threat model";
    let threat_model = json_object(value, label)?;
    if threat_model.contains_key("format") {
        require_allowed_json_fields(
            threat_model,
            &["format", "content"],
            &["format", "content", "scope", "origin"],
            label,
        )?;
        ensure!(
            string(threat_model, "format", label)? == "markdown",
            "scan manifest threat model format is unsupported"
        );
        let content = nonempty_string(threat_model, "content", label)?;
        ensure!(
            !content.trim().is_empty(),
            "scan manifest threat model content is blank"
        );
    } else {
        require_allowed_json_fields(
            threat_model,
            &["summary"],
            &[
                "summary",
                "assets",
                "trustBoundaries",
                "attackerCapabilities",
                "securityObjectives",
                "assumptions",
                "scope",
                "origin",
            ],
            label,
        )?;
        nonempty_string(threat_model, "summary", label)?;
        for field in [
            "assets",
            "trustBoundaries",
            "attackerCapabilities",
            "securityObjectives",
            "assumptions",
        ] {
            if let Some(value) = threat_model.get(field) {
                require_nonempty_string_array(value, label)?;
            }
        }
    }
    if let Some(scope) = threat_model.get("scope") {
        require_threat_model_scope(scope)?;
    }
    if let Some(origin) = optional_nonempty_string(threat_model, "origin", label)? {
        ensure!(
            matches!(
                origin,
                "generated" | "provided" | "reconciled" | "recovered"
            ),
            "scan manifest threat model origin is unsupported"
        );
    }
    Ok(threat_model)
}

fn require_threat_model_scope(value: &JsonValue) -> Result<()> {
    let label = "scan manifest threat model scope";
    let scope = json_object(value, label)?;
    require_allowed_json_fields(
        scope,
        &["includePaths"],
        &["includePaths", "excludePaths", "summary"],
        label,
    )?;
    ensure!(
        json_string_array(json_field(scope, "includePaths", label)?, label)? == ["."],
        "scan manifest threat model scope is not the complete repository"
    );
    if let Some(exclude_paths) = scope.get("excludePaths") {
        ensure!(
            json_string_array(exclude_paths, label)?.is_empty(),
            "scan manifest threat model scope excludes paths"
        );
    }
    optional_nonempty_string(scope, "summary", label)?;
    Ok(())
}

fn require_allowed_json_fields(
    object: &BTreeMap<String, JsonValue>,
    required: &[&str],
    allowed: &[&str],
    label: &str,
) -> Result<()> {
    ensure!(
        required.iter().all(|field| object.contains_key(*field))
            && object.keys().all(|field| allowed.contains(&field.as_str())),
        "{label} differs from the qualified projection profile"
    );
    Ok(())
}

fn optional_nonempty_string<'json>(
    object: &'json BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<Option<&'json str>> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    let value = json_string(value, &format!("{label} `{field}`"))?;
    ensure!(!value.is_empty(), "{label} `{field}` is empty");
    Ok(Some(value))
}

fn require_nonempty_string_array(value: &JsonValue, label: &str) -> Result<()> {
    for item in json_string_array(value, label)? {
        ensure!(!item.is_empty(), "{label} contains an empty string");
    }
    Ok(())
}

fn require_preserved_sources(value: &JsonValue) -> Result<()> {
    let sources = json_object(value, "scan manifest preserved sources")?;
    ensure!(
        !sources.is_empty(),
        "scan manifest preserved sources are empty"
    );
    for (path, digest) in sources {
        plugin_relative_path(path).with_context(|| {
            format!("scan manifest preserved source `{path}` is not a safe relative path")
        })?;
        let digest = json_string(digest, "scan manifest preserved source digest")?;
        ensure!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
            "scan manifest preserved source `{path}` has a noncanonical SHA-256 digest"
        );
    }
    Ok(())
}

fn report_strings(value: Option<&JsonValue>, label: &str) -> Result<Vec<String>> {
    match value {
        Some(value) => {
            let values = json_string_array(value, label)?
                .into_iter()
                .map(|value| report_text(value, ""))
                .collect::<Result<Vec<_>>>()?;
            Ok(values
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect())
        }
        None => Ok(Vec::new()),
    }
}

fn report_cell(value: &str) -> Result<String> {
    Ok(report_text(value, "none")?.replace('|', "\\|"))
}

fn report_text(value: &str, fallback: &str) -> Result<String> {
    let candidate = if value.trim().is_empty() {
        fallback
    } else {
        value
    };
    let mut normalized = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = normalized.chars().collect::<Vec<_>>();
    let numeric_prefix = chars
        .iter()
        .take_while(|character| character.is_numeric())
        .count();
    ensure!(
        !(numeric_prefix > 0
            && chars[..numeric_prefix]
                .iter()
                .any(|character| !character.is_ascii())
            && chars.get(numeric_prefix) == Some(&'.')
            && chars.get(numeric_prefix + 1) == Some(&' ')),
        "Codex Security report text has an unsupported Unicode numeric list prefix"
    );
    let prefixed = normalized.starts_with('|')
        || normalized.starts_with("```")
        || matches!(chars.as_slice(), [first, ' ', ..] if matches!(first, '-' | '*' | '+' | '>'))
        || {
            let hashes = chars
                .iter()
                .take_while(|character| **character == '#')
                .count();
            (1..=6).contains(&hashes) && chars.get(hashes) == Some(&' ')
        }
        || {
            let digits = chars
                .iter()
                .take_while(|character| character.is_ascii_digit())
                .count();
            digits > 0 && chars.get(digits) == Some(&'.') && chars.get(digits + 1) == Some(&' ')
        };
    if prefixed {
        normalized.insert_str(0, "Text: ");
    }

    let characters = normalized.chars().collect::<Vec<_>>();
    let mut escaped = String::new();
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '`'
            && (index == 0 || characters[index - 1] != '`')
            && let Some(relative_end) = characters[index + 1..]
                .iter()
                .position(|character| *character == '`')
        {
            let end = index + 1 + relative_end;
            if end > index + 1
                && characters
                    .get(end + 1)
                    .is_none_or(|character| *character != '`')
            {
                escaped.extend(&characters[index..=end]);
                index = end + 1;
                continue;
            }
        }
        let character = characters[index];
        if matches!(character, '\\' | '`' | '*' | '[' | ']' | '<' | '>') {
            escaped.push('\\');
        }
        escaped.push(character);
        index += 1;
    }
    Ok(escaped)
}

fn require_scope_value(scope: &BTreeMap<String, JsonValue>, label: &str) -> Result<()> {
    ensure!(
        json_string_array(json_field(scope, "includePaths", label)?, label)? == ["."],
        "{label} is not the complete repository"
    );
    ensure!(
        json_string_array(json_field(scope, "excludePaths", label)?, label)?.is_empty(),
        "{label} excludes paths"
    );
    Ok(())
}

fn verify_manifest_artifacts(value: &JsonValue, findings: &[u8], coverage: &[u8]) -> Result<()> {
    let artifacts = json_array(value, "scan manifest artifacts")?;
    ensure!(
        artifacts.len() == 2,
        "scan manifest references unhandled artifacts"
    );
    let expected = BTreeMap::from([
        (FINDINGS, sha256::hex(&sha256::digest(findings))),
        (COVERAGE, sha256::hex(&sha256::digest(coverage))),
    ]);
    let mut observed = BTreeMap::new();
    for artifact in artifacts {
        let artifact = json_object(artifact, "scan manifest artifact")?;
        require_json_fields(
            artifact,
            &["path", "sha256", "mediaType"],
            "scan manifest artifact",
        )?;
        let path = string(artifact, "path", "scan manifest artifact")?;
        let digest = string(artifact, "sha256", "scan manifest artifact")?;
        ensure!(
            string(artifact, "mediaType", "scan manifest artifact")? == "application/json",
            "scan manifest artifact has an unexpected media type"
        );
        ensure!(
            observed.insert(path, digest).is_none(),
            "scan manifest artifact is duplicated"
        );
    }
    ensure!(
        observed.len() == expected.len(),
        "scan manifest artifact set differs"
    );
    for (path, digest) in expected {
        ensure!(
            observed.get(path).copied() == Some(digest.as_str()),
            "scan manifest digest for `{path}` differs"
        );
    }
    Ok(())
}

pub(super) fn string<'json>(
    object: &'json BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<&'json str> {
    json_string(
        json_field(object, field, label)?,
        &format!("{label} `{field}`"),
    )
}

pub(super) fn nonempty_string<'json>(
    object: &'json BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<&'json str> {
    let value = string(object, field, label)?;
    ensure!(!value.is_empty(), "{label} `{field}` is empty");
    Ok(value)
}

pub(crate) fn verify_sealed(
    _paths: &Paths,
    directory: &Path,
    source: &SourceIdentity,
) -> Result<Vec<(String, String)>> {
    let runtime = installed_python_runtime()?;
    let git = crate::git::resolved()?;
    git.require_unchanged()?;
    let git = git_identity(git)?;
    verify_sealed_with_identities(directory, source, &runtime, &git)
}

pub(super) fn verify_sealed_with_identities(
    directory: &Path,
    source: &SourceIdentity,
    runtime: &PythonRuntimeIdentity,
    git: &GitIdentity,
) -> Result<Vec<(String, String)>> {
    let directory = canonical_directory(directory, "sealed Codex Security qualification")?;
    let request = read(
        &directory.join(REQUEST),
        MAX_REQUEST_BYTES,
        "sealed Codex Security request",
    )?;
    let request_fields = verify_owned_request(&request, source, runtime, git)?;
    let request_values: BTreeMap<&str, &str> = request_fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let permissions = WorkerPermissionDigests {
        parent_state: request_values
            .get("execution.worker-parent-state.sha256")
            .context("sealed request has no worker parent-state digest")?
            .to_string(),
        provider_profile: request_values
            .get("execution.worker-provider-profile.sha256")
            .context("sealed request has no worker provider-profile digest")?
            .to_string(),
        effective_profile: request_values
            .get("execution.worker-effective-profile.sha256")
            .context("sealed request has no worker effective-profile digest")?
            .to_string(),
        effective_state: request_values
            .get("execution.worker-effective-state.sha256")
            .context("sealed request has no worker effective-state digest")?
            .to_string(),
    };
    let worker_transitions = read(
        &directory.join(WORKER_TRANSITIONS),
        MAX_WORKER_TRANSITIONS_BYTES,
        "sealed private Codex transition ledger",
    )?;
    let worker_transitions =
        verify_worker_transition_ledger(&worker_transitions, source, &permissions)?;
    let manifest = read(
        &directory.join(MANIFEST),
        MAX_MANIFEST_BYTES,
        "sealed Codex Security manifest",
    )?;
    let findings = read(
        &directory.join(FINDINGS),
        MAX_FINDINGS_BYTES,
        "sealed Codex Security findings",
    )?;
    let coverage = read(
        &directory.join(COVERAGE),
        MAX_COVERAGE_BYTES,
        "sealed Codex Security coverage",
    )?;
    let report = read(
        &directory.join(REPORT),
        MAX_REPORT_BYTES,
        "sealed Codex Security report",
    )?;
    let scan = verify_scan(&manifest, &findings, &coverage, &report, source)?;
    let mut derived = request_fields;
    derived.extend(worker_transition_fields(&worker_transitions));
    derived.extend(scan_fields(&scan));
    let unique: BTreeMap<&str, &str> = derived
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    ensure!(
        unique.len() == derived.len(),
        "sealed Codex Security fields are duplicated"
    );
    Ok(derived)
}

pub(super) fn worker_transition_fields(
    transitions: &WorkerTransitionLedger,
) -> Vec<(String, String)> {
    vec![
        (
            "execution.worker-preflight.allowed-reads".to_string(),
            transitions.probes.allowed_reads.to_string(),
        ),
        (
            "execution.worker-preflight.denied-reads".to_string(),
            transitions.probes.denied_reads.to_string(),
        ),
        (
            "execution.worker-preflight.denied-write-creates".to_string(),
            transitions.probes.denied_write_creates.to_string(),
        ),
        (
            "execution.worker-preflight.denied-write-mutations".to_string(),
            transitions.probes.denied_write_mutations.to_string(),
        ),
        (
            "execution.worker-preflight.denied-network-listeners".to_string(),
            transitions.probes.denied_network_listeners.to_string(),
        ),
        (
            "execution.worker-preflight.denied-network-connections".to_string(),
            transitions.probes.denied_network_connections.to_string(),
        ),
        (
            "execution.worker-preflight.denied-descendants".to_string(),
            transitions.probes.denied_descendants.to_string(),
        ),
        (
            "execution.worker-transitions.app-server".to_string(),
            transitions.app_server.to_string(),
        ),
        (
            "execution.worker-transitions.exec-initial".to_string(),
            transitions.exec_initial.to_string(),
        ),
        (
            "execution.worker-transitions.exec-resumed".to_string(),
            transitions.exec_resumed.to_string(),
        ),
    ]
}

pub(super) fn fields(
    scan: &Scan,
    tools: &Toolchain,
    package: &PluginPackage,
    permissions: &WorkerPermissionDigests,
    transitions: &WorkerTransitionLedger,
) -> Vec<(String, String)> {
    let mut result = owned_identity_fields(OwnedIdentity {
        node: RecordedTool::of(&tools.node),
        python: RecordedTool::of(&tools.python),
        codex: RecordedTool::of(&tools.codex),
        codex_runtime: &tools.codex_runtime,
        integrity_bytes: package.integrity.len() as u64,
        python_runtime: &tools.python_runtime,
    });
    result.extend([
        (
            "execution.worker-effective-profile.sha256".to_string(),
            permissions.effective_profile.clone(),
        ),
        (
            "execution.worker-effective-state.sha256".to_string(),
            permissions.effective_state.clone(),
        ),
        (
            "execution.worker-parent-state.sha256".to_string(),
            permissions.parent_state.clone(),
        ),
        (
            "execution.worker-provider-profile.sha256".to_string(),
            permissions.provider_profile.clone(),
        ),
    ]);
    result.extend(worker_transition_fields(transitions));
    result.extend(scan_fields(scan));
    result
}

fn scan_fields(scan: &Scan) -> Vec<(String, String)> {
    let total = scan.critical + scan.high + scan.medium + scan.low + scan.informational;
    vec![
        ("findings.critical".to_string(), scan.critical.to_string()),
        ("findings.high".to_string(), scan.high.to_string()),
        (
            "findings.informational".to_string(),
            scan.informational.to_string(),
        ),
        ("findings.low".to_string(), scan.low.to_string()),
        ("findings.medium".to_string(), scan.medium.to_string()),
        ("findings.total".to_string(), total.to_string()),
        ("scan.id".to_string(), scan.id.clone()),
        ("scan.producer.name".to_string(), scan.producer_name.clone()),
        (
            "scan.producer.version".to_string(),
            scan.producer_version.clone(),
        ),
        ("scan.surfaces".to_string(), scan.surfaces.to_string()),
    ]
}

#[derive(Clone, Copy, Debug)]
struct RecordedTool<'identity> {
    bytes: u64,
    sha256: &'identity str,
    version: &'identity str,
}

impl<'identity> RecordedTool<'identity> {
    fn of(tool: &'identity ToolIdentity) -> Self {
        Self {
            bytes: tool.bytes,
            sha256: &tool.sha256,
            version: &tool.version,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct OwnedIdentity<'identity> {
    node: RecordedTool<'identity>,
    python: RecordedTool<'identity>,
    codex: RecordedTool<'identity>,
    codex_runtime: &'identity CodexRuntimeIdentity,
    integrity_bytes: u64,
    python_runtime: &'identity PythonRuntimeIdentity,
}

fn owned_identity_fields(identity: OwnedIdentity<'_>) -> Vec<(String, String)> {
    let OwnedIdentity {
        node,
        python,
        codex,
        codex_runtime,
        integrity_bytes,
        python_runtime,
    } = identity;
    vec![
        (
            "authentication.channel".to_string(),
            AUTHENTICATION_CHANNEL.to_string(),
        ),
        (
            "environment.policy".to_string(),
            ENVIRONMENT_POLICY.to_string(),
        ),
        (
            "execution.authority".to_string(),
            EXECUTION_AUTHORITY.to_string(),
        ),
        (
            "execution.complete-tool".to_string(),
            COMPLETE_TOOL.to_string(),
        ),
        ("execution.model".to_string(), MODEL.to_string()),
        ("execution.protocol".to_string(), MCP_PROTOCOL.to_string()),
        ("execution.reasoning".to_string(), REASONING.to_string()),
        (
            "execution.server-name".to_string(),
            MCP_SERVER_NAME.to_string(),
        ),
        (
            "execution.server-version".to_string(),
            MCP_SERVER_VERSION.to_string(),
        ),
        ("execution.start-tool".to_string(), START_TOOL.to_string()),
        (
            "execution.tools.bytes".to_string(),
            MCP_TOOLS_BYTES.to_string(),
        ),
        (
            "execution.tools.count".to_string(),
            MCP_TOOLS_COUNT.to_string(),
        ),
        (
            "execution.tools.sha256".to_string(),
            MCP_TOOLS_SHA256.to_string(),
        ),
        (
            "execution.worker-permission".to_string(),
            WORKER_PERMISSION.to_string(),
        ),
        (
            "tool.codex-security.integrity.bytes".to_string(),
            integrity_bytes.to_string(),
        ),
        (
            "tool.codex-security.integrity.sha256".to_string(),
            PLUGIN_INTEGRITY_SHA256.to_string(),
        ),
        (
            "tool.codex-security.tree.files".to_string(),
            PLUGIN_FILES.to_string(),
        ),
        (
            "tool.codex-security.version".to_string(),
            PRODUCER_VERSION.to_string(),
        ),
        ("tool.node.bytes".to_string(), node.bytes.to_string()),
        ("tool.node.sha256".to_string(), node.sha256.to_string()),
        ("tool.node.version".to_string(), node.version.to_string()),
        ("tool.python.bytes".to_string(), python.bytes.to_string()),
        ("tool.python.sha256".to_string(), python.sha256.to_string()),
        (
            "tool.python.version".to_string(),
            python.version.to_string(),
        ),
        (
            "tool.python.runtime.bytes".to_string(),
            python_runtime.bytes.to_string(),
        ),
        (
            "tool.python.runtime.files".to_string(),
            python_runtime.entries.to_string(),
        ),
        (
            "tool.python.runtime.root".to_string(),
            "parent-of-tool.python".to_string(),
        ),
        (
            "tool.python.runtime.sha256".to_string(),
            python_runtime.sha256.clone(),
        ),
        ("tool.codex.bytes".to_string(), codex.bytes.to_string()),
        ("tool.codex.sha256".to_string(), codex.sha256.to_string()),
        ("tool.codex.version".to_string(), codex.version.to_string()),
        (
            "tool.codex.runtime.bytes".to_string(),
            codex_runtime.bytes.to_string(),
        ),
        (
            "tool.codex.runtime.files".to_string(),
            codex_runtime.entries.to_string(),
        ),
        (
            "tool.codex.runtime.root".to_string(),
            "parent-of-tool.codex".to_string(),
        ),
        (
            "tool.codex.runtime.sha256".to_string(),
            codex_runtime.sha256.clone(),
        ),
    ]
}

pub(super) fn render_request(source: &SourceIdentity) -> Vec<u8> {
    let body = format!(
        "wizard-codex-security-request-v1\nsource.commit={}\nsource.tree={}\ntarget.kind=git_revision\ntarget.revision={}\nworkflow=deep_repository\npasses=variance-reducing-multi-pass-plus-canonical-rerun\npermission=managed-read-only\nscope.include=.\nscope.exclude=\n",
        source.commit, source.tree, source.commit
    );
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    format!("{body}checksum.sha256={checksum}\n").into_bytes()
}

pub(super) fn verify_request(bytes: &[u8], source: &SourceIdentity) -> Result<()> {
    ensure!(
        bytes == render_request(source),
        "Codex Security request differs from the exact source-bound workflow"
    );
    Ok(())
}

pub(super) fn render_owned_request(
    source: &SourceIdentity,
    tools: &Toolchain,
    git: &GitIdentity,
    package: &PluginPackage,
    permissions: &WorkerPermissionDigests,
) -> Vec<u8> {
    let body = format!(
        "wizard-codex-security-request-v3\n\
authentication.channel={AUTHENTICATION_CHANNEL}\n\
environment.policy={ENVIRONMENT_POLICY}\n\
execution.authority={EXECUTION_AUTHORITY}\n\
execution.complete-tool={COMPLETE_TOOL}\n\
execution.model={MODEL}\n\
execution.protocol={MCP_PROTOCOL}\n\
execution.reasoning={REASONING}\n\
execution.server-name={MCP_SERVER_NAME}\n\
execution.server-version={MCP_SERVER_VERSION}\n\
execution.start-tool={START_TOOL}\n\
execution.tools.bytes={MCP_TOOLS_BYTES}\n\
execution.tools.count={MCP_TOOLS_COUNT}\n\
execution.tools.sha256={MCP_TOOLS_SHA256}\n\
execution.worker-permission={WORKER_PERMISSION}\n\
execution.worker-effective-profile.sha256={}\n\
execution.worker-effective-state.sha256={}\n\
execution.worker-parent-state.sha256={}\n\
execution.worker-provider-profile.sha256={}\n\
source.commit={}\n\
source.tree={}\n\
tool.git.bytes={}\n\
tool.git.path={}\n\
tool.git.sha256={}\n\
tool.git.version={}\n\
target.kind=git_revision\n\
target.revision={}\n\
workflow=deep_repository\n\
passes=variance-reducing-multi-pass-plus-canonical-rerun\n\
permission=managed-read-only\n\
scope.include=.\n\
scope.exclude=\n\
tool.codex-security.integrity.bytes={}\n\
tool.codex-security.integrity.sha256={PLUGIN_INTEGRITY_SHA256}\n\
tool.codex-security.tree.files={PLUGIN_FILES}\n\
tool.codex-security.version={PRODUCER_VERSION}\n\
tool.node.bytes={}\n\
tool.node.sha256={}\n\
tool.node.version={}\n\
tool.python.bytes={}\n\
tool.python.sha256={}\n\
tool.python.version={}\n\
tool.python.runtime.bytes={}\n\
tool.python.runtime.files={}\n\
tool.python.runtime.root=parent-of-tool.python\n\
tool.python.runtime.sha256={}\n\
tool.codex.bytes={}\n\
tool.codex.sha256={}\n\
tool.codex.version={}\n\
tool.codex.runtime.bytes={}\n\
tool.codex.runtime.files={}\n\
tool.codex.runtime.root=parent-of-tool.codex\n\
tool.codex.runtime.sha256={}\n",
        permissions.effective_profile,
        permissions.effective_state,
        permissions.parent_state,
        permissions.provider_profile,
        source.commit,
        source.tree,
        git.bytes,
        git.path.to_string_lossy(),
        git.sha256,
        git.version,
        source.commit,
        package.integrity.len(),
        tools.node.bytes,
        tools.node.sha256,
        tools.node.version,
        tools.python.bytes,
        tools.python.sha256,
        tools.python.version,
        tools.python_runtime.bytes,
        tools.python_runtime.entries,
        tools.python_runtime.sha256,
        tools.codex.bytes,
        tools.codex.sha256,
        tools.codex.version,
        tools.codex_runtime.bytes,
        tools.codex_runtime.entries,
        tools.codex_runtime.sha256,
    );
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    format!("{body}checksum.sha256={checksum}\n").into_bytes()
}

const WORKER_POLICY_DIGEST_FIELDS: [&str; 4] = [
    "execution.worker-effective-profile.sha256",
    "execution.worker-effective-state.sha256",
    "execution.worker-parent-state.sha256",
    "execution.worker-provider-profile.sha256",
];

pub(super) fn verify_owned_request(
    bytes: &[u8],
    source: &SourceIdentity,
    python_runtime: &PythonRuntimeIdentity,
    git: &GitIdentity,
) -> Result<Vec<(String, String)>> {
    let text = std::str::from_utf8(bytes).context("owned Codex Security request is not UTF-8")?;
    ensure!(
        text.ends_with('\n') && !text.contains('\r') && !text.contains('\0'),
        "owned Codex Security request framing differs"
    );
    let values = read_owned_request(text)?;
    require_owned_request_values(&values, source, python_runtime, git)?;
    let codex_runtime = qualified_codex_runtime_identity();
    let mut fields = owned_identity_fields(OwnedIdentity {
        node: RecordedTool {
            bytes: NODE_BYTES,
            sha256: NODE_SHA256,
            version: NODE_VERSION,
        },
        python: RecordedTool {
            bytes: PYTHON_BYTES,
            sha256: PYTHON_SHA256,
            version: PYTHON_VERSION,
        },
        codex: RecordedTool {
            bytes: CODEX_BYTES,
            sha256: CODEX_SHA256,
            version: CODEX_VERSION,
        },
        codex_runtime: &codex_runtime,
        integrity_bytes: PLUGIN_INTEGRITY_BYTES,
        python_runtime,
    });
    for key in WORKER_POLICY_DIGEST_FIELDS {
        fields.push((
            key.to_string(),
            values
                .get(key)
                .copied()
                .context("owned Codex Security request has no worker-policy digest")?
                .to_string(),
        ));
    }
    Ok(fields)
}

fn read_owned_request(text: &str) -> Result<BTreeMap<&str, &str>> {
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some("wizard-codex-security-request-v3"),
        "owned Codex Security request schema differs"
    );
    let expected_keys = [
        "authentication.channel",
        "environment.policy",
        "execution.authority",
        "execution.complete-tool",
        "execution.model",
        "execution.protocol",
        "execution.reasoning",
        "execution.server-name",
        "execution.server-version",
        "execution.start-tool",
        "execution.tools.bytes",
        "execution.tools.count",
        "execution.tools.sha256",
        "execution.worker-permission",
        "execution.worker-effective-profile.sha256",
        "execution.worker-effective-state.sha256",
        "execution.worker-parent-state.sha256",
        "execution.worker-provider-profile.sha256",
        "source.commit",
        "source.tree",
        "tool.git.bytes",
        "tool.git.path",
        "tool.git.sha256",
        "tool.git.version",
        "target.kind",
        "target.revision",
        "workflow",
        "passes",
        "permission",
        "scope.include",
        "scope.exclude",
        "tool.codex-security.integrity.bytes",
        "tool.codex-security.integrity.sha256",
        "tool.codex-security.tree.files",
        "tool.codex-security.version",
        "tool.node.bytes",
        "tool.node.sha256",
        "tool.node.version",
        "tool.python.bytes",
        "tool.python.sha256",
        "tool.python.version",
        "tool.python.runtime.bytes",
        "tool.python.runtime.files",
        "tool.python.runtime.root",
        "tool.python.runtime.sha256",
        "tool.codex.bytes",
        "tool.codex.sha256",
        "tool.codex.version",
        "tool.codex.runtime.bytes",
        "tool.codex.runtime.files",
        "tool.codex.runtime.root",
        "tool.codex.runtime.sha256",
    ];
    let mut values = BTreeMap::new();
    for expected in expected_keys {
        let line = lines
            .next()
            .with_context(|| format!("owned Codex Security request has no {expected}"))?;
        let (key, value) = line
            .split_once('=')
            .context("owned Codex Security request line has no separator")?;
        ensure!(
            key == expected && values.insert(key, value).is_none(),
            "owned Codex Security request field order or uniqueness differs"
        );
    }
    let checksum_line = lines
        .next()
        .context("owned Codex Security request has no checksum")?;
    let checksum = checksum_line
        .strip_prefix("checksum.sha256=")
        .context("owned Codex Security request checksum field differs")?;
    require_sha256(checksum, "owned Codex Security request checksum")?;
    ensure!(
        lines.next().is_none(),
        "owned Codex Security request has trailing fields"
    );
    for field in WORKER_POLICY_DIGEST_FIELDS {
        require_sha256(
            values
                .get(field)
                .copied()
                .context("owned Codex Security request has no worker-policy digest")?,
            "owned Codex Security worker-policy digest",
        )?;
    }
    let checksum_offset = text
        .rfind("checksum.sha256=")
        .context("owned Codex Security request has no checksum offset")?;
    ensure!(
        sha256::hex(&sha256::digest(&text.as_bytes()[..checksum_offset])) == checksum,
        "owned Codex Security request checksum differs"
    );
    Ok(values)
}

fn require_owned_request_values(
    values: &BTreeMap<&str, &str>,
    source: &SourceIdentity,
    python_runtime: &PythonRuntimeIdentity,
    git: &GitIdentity,
) -> Result<()> {
    for (key, expected) in [
        ("authentication.channel", AUTHENTICATION_CHANNEL),
        ("environment.policy", ENVIRONMENT_POLICY),
        ("execution.authority", EXECUTION_AUTHORITY),
        ("execution.complete-tool", COMPLETE_TOOL),
        ("execution.model", MODEL),
        ("execution.protocol", MCP_PROTOCOL),
        ("execution.reasoning", REASONING),
        ("execution.server-name", MCP_SERVER_NAME),
        ("execution.server-version", MCP_SERVER_VERSION),
        ("execution.start-tool", START_TOOL),
        ("execution.tools.bytes", "79316"),
        ("execution.tools.count", "47"),
        ("execution.tools.sha256", MCP_TOOLS_SHA256),
        ("execution.worker-permission", WORKER_PERMISSION),
        ("source.commit", source.commit.as_str()),
        ("source.tree", source.tree.as_str()),
        ("tool.git.sha256", git.sha256.as_str()),
        ("tool.git.version", git.version.as_str()),
        ("target.kind", "git_revision"),
        ("target.revision", source.commit.as_str()),
        ("workflow", "deep_repository"),
        (
            "passes",
            "variance-reducing-multi-pass-plus-canonical-rerun",
        ),
        ("permission", "managed-read-only"),
        ("scope.include", "."),
        ("scope.exclude", ""),
        ("tool.codex-security.integrity.bytes", "17465"),
        (
            "tool.codex-security.integrity.sha256",
            PLUGIN_INTEGRITY_SHA256,
        ),
        ("tool.codex-security.tree.files", "142"),
        ("tool.codex-security.version", PRODUCER_VERSION),
        ("tool.node.bytes", "93381936"),
        ("tool.node.sha256", NODE_SHA256),
        ("tool.node.version", NODE_VERSION),
        ("tool.python.bytes", "104952"),
        ("tool.python.sha256", PYTHON_SHA256),
        ("tool.python.version", PYTHON_VERSION),
        ("tool.python.runtime.root", "parent-of-tool.python"),
        ("tool.codex.bytes", "341312816"),
        ("tool.codex.sha256", CODEX_SHA256),
        ("tool.codex.version", CODEX_VERSION),
        ("tool.codex.runtime.bytes", "442152128"),
        ("tool.codex.runtime.files", "4"),
        ("tool.codex.runtime.root", "parent-of-tool.codex"),
        ("tool.codex.runtime.sha256", CODEX_RUNTIME_SHA256),
    ] {
        ensure!(
            values.get(key).copied() == Some(expected),
            "owned Codex Security request {key} differs"
        );
    }
    ensure!(
        values.get("tool.git.bytes").copied() == Some(git.bytes.to_string().as_str())
            && values.get("tool.git.path").copied() == git.path.to_str(),
        "owned Codex Security request Git identity differs"
    );
    ensure!(
        values.get("tool.python.runtime.bytes").copied()
            == Some(python_runtime.bytes.to_string().as_str())
            && values.get("tool.python.runtime.files").copied()
                == Some(python_runtime.entries.to_string().as_str())
            && values.get("tool.python.runtime.sha256").copied()
                == Some(python_runtime.sha256.as_str()),
        "owned Codex Security request Python runtime identity differs"
    );
    Ok(())
}

pub(super) fn qualified_codex_runtime_identity() -> CodexRuntimeIdentity {
    CodexRuntimeIdentity {
        root: PathBuf::from("parent-of-tool.codex"),
        entries: CODEX_RUNTIME_FILES.len() as u64,
        bytes: CODEX_RUNTIME_BYTES,
        sha256: CODEX_RUNTIME_SHA256.to_string(),
    }
}

pub(super) fn read(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    qualification::read_bounded_file(path, maximum, label)
}

pub(super) fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    crate::mutate::workspace::canonical_existing_real_directory(path, label)
}
