//! Source-bound request and canonical artifact checks for the final deep scan.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::cli::SecurityDeepAction;
use crate::json::{
    JsonValue, json_array, json_field, json_object, json_string, json_string_array, parse_json,
    require_json_fields,
};
use crate::paths::Paths;
use crate::qualification::{self, ExternalArtifact, Kind, SourceIdentity};
use crate::sha256;

const REQUEST: &str = "request";
const MANIFEST: &str = "scan-manifest.json";
const FINDINGS: &str = "findings.json";
const COVERAGE: &str = "coverage.json";
const REPORT: &str = "report.md";
const MAX_REQUEST_BYTES: u64 = 256 * 1024;
const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_FINDINGS_BYTES: u64 = 32 * 1024 * 1024;
const MAX_COVERAGE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_REPORT_BYTES: u64 = 32 * 1024 * 1024;
const PRODUCER_NAME: &str = "codex-security-plugin";
const PRODUCER_VERSION: &str = "0.1.32";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Scan {
    id: String,
    producer_name: String,
    producer_version: String,
    surfaces: u64,
    critical: u64,
    high: u64,
    medium: u64,
    low: u64,
    informational: u64,
}

impl Scan {
    fn fields(&self) -> Vec<(String, String)> {
        [
            ("findings.critical", self.critical.to_string()),
            ("findings.high", self.high.to_string()),
            ("findings.informational", self.informational.to_string()),
            ("findings.low", self.low.to_string()),
            ("findings.medium", self.medium.to_string()),
            (
                "findings.total",
                (self.critical + self.high + self.medium + self.low + self.informational)
                    .to_string(),
            ),
            ("scan.id", self.id.clone()),
            ("scan.producer.name", self.producer_name.clone()),
            ("scan.producer.version", self.producer_version.clone()),
            ("scan.surfaces", self.surfaces.to_string()),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
    }
}

pub(crate) fn run(paths: &Paths, action: SecurityDeepAction) -> Result<()> {
    match action {
        SecurityDeepAction::Begin => begin(paths),
        SecurityDeepAction::Seal(directory) => seal(paths, &directory),
    }
}

fn begin(paths: &Paths) -> Result<()> {
    let attempt = qualification::begin_external(paths, Kind::CodexSecurityDeep, false)?;
    let request = render_request(attempt.source());
    attempt.write_security_request(&request)?;
    println!(
        "[xtask] Codex Security deep-scan request ready at {}",
        attempt.directory().join(REQUEST).display()
    );
    Ok(())
}

fn seal(paths: &Paths, scan_directory: &Path) -> Result<()> {
    let attempt = qualification::begin_external(paths, Kind::CodexSecurityDeep, true)?;
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
    let fields = scan.fields();
    attempt.seal(
        &fields,
        vec![
            ExternalArtifact::SecurityRequest(request),
            ExternalArtifact::SecurityManifest(manifest),
            ExternalArtifact::SecurityFindings(findings),
            ExternalArtifact::SecurityCoverage(coverage),
            ExternalArtifact::SecurityReport(report),
        ],
    )
}

pub(crate) fn verify_sealed(
    directory: &Path,
    source: &SourceIdentity,
) -> Result<Vec<(String, String)>> {
    let request = read(
        &directory.join(REQUEST),
        MAX_REQUEST_BYTES,
        "sealed scan request",
    )?;
    let manifest = read(
        &directory.join(MANIFEST),
        MAX_MANIFEST_BYTES,
        "sealed scan manifest",
    )?;
    let findings = read(
        &directory.join(FINDINGS),
        MAX_FINDINGS_BYTES,
        "sealed findings",
    )?;
    let coverage = read(
        &directory.join(COVERAGE),
        MAX_COVERAGE_BYTES,
        "sealed coverage",
    )?;
    let report = read(&directory.join(REPORT), MAX_REPORT_BYTES, "sealed report")?;
    verify_request(&request, source)?;
    Ok(verify_scan(&manifest, &findings, &coverage, &report, source)?.fields())
}

fn verify_scan(
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
    require_scope(
        json_field(scan, "scope", "scan manifest")?,
        "scan manifest scope",
    )?;
    verify_manifest_artifacts(
        json_field(scan, "artifacts", "scan manifest")?,
        findings,
        coverage,
    )?;

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
    let surfaces = json_array(
        json_field(coverage_root, "surfaces", "coverage")?,
        "coverage surfaces",
    )?;
    ensure!(
        surfaces.is_empty(),
        "Codex Security deep aggregate has noncanonical surface rows"
    );
    Ok(Scan {
        id: id.to_string(),
        producer_name,
        producer_version,
        surfaces: surfaces.len() as u64,
        critical: counts["critical"],
        high: counts["high"],
        medium: counts["medium"],
        low: counts["low"],
        informational: counts["informational"],
    })
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

fn require_scope(value: &JsonValue, label: &str) -> Result<()> {
    let scope = json_object(value, label)?;
    require_json_fields(scope, &["includePaths", "excludePaths"], label)?;
    require_scope_value(scope, label)
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

fn string<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<&'a str> {
    json_string(
        json_field(object, field, label)?,
        &format!("{label} `{field}`"),
    )
}

fn nonempty_string<'a>(
    object: &'a BTreeMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<&'a str> {
    let value = string(object, field, label)?;
    ensure!(!value.is_empty(), "{label} `{field}` is empty");
    Ok(value)
}

fn render_request(source: &SourceIdentity) -> Vec<u8> {
    let body = format!(
        "wizard-codex-security-request-v1\nsource.commit={}\nsource.tree={}\ntarget.kind=git_revision\ntarget.revision={}\nworkflow=deep_repository\npasses=variance-reducing-multi-pass-plus-canonical-rerun\npermission=managed-read-only\nscope.include=.\nscope.exclude=\n",
        source.commit, source.tree, source.commit
    );
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    format!("{body}checksum.sha256={checksum}\n").into_bytes()
}

fn verify_request(bytes: &[u8], source: &SourceIdentity) -> Result<()> {
    ensure!(
        bytes == render_request(source),
        "Codex Security request differs from the exact source-bound workflow"
    );
    Ok(())
}

fn read(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    qualification::read_bounded_file(path, maximum, label)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf> {
    crate::mutation_workspace::canonical_existing_real_directory(path, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SourceIdentity {
        SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        }
    }

    fn artifacts(
        finding_level: Option<&str>,
        completeness: &str,
        revision: &str,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let finding = finding_level.map_or(String::new(), |level| format!(r#"{{"findingId":"csf_000000000000000000000001","fingerprints":{{"algorithm":"codex-security/v1","primary":"codex-security/v1:sha256:{}"}},"severity":{{"level":"{level}"}}}}"#, "1".repeat(64)));
        let findings = format!(r#"{{"documentType":"codex-security.findings","schemaVersion":"1.0","scanId":"scan-1","findings":[{finding}]}}"#).into_bytes();
        let coverage = format!(r#"{{"documentType":"codex-security.coverage","schemaVersion":"1.0","scanId":"scan-1","mode":"deep_repository","completeness":"{completeness}","inventoryStrategy":"repository","includePaths":["."],"excludePaths":[],"surfaces":[],"explicitExclusions":[],"deferred":[]}}"#).into_bytes();
        let manifest = format!(r#"{{"documentType":"codex-security.scan-manifest","schemaVersion":"1.0","scan":{{"id":"scan-1","producer":{{"name":"codex-security-plugin","version":"0.1.32"}},"status":"completed","startedAt":"2026-09-28T00:00:00Z","completedAt":"2026-09-28T01:00:00Z","sealedAt":"2026-09-28T01:00:00Z","target":{{"kind":"git_revision","targetId":"target-1","displayName":"wizards-bedrock","revision":"{revision}"}},"scope":{{"includePaths":["."],"excludePaths":[]}},"coverageRef":"coverage.json","findingsRef":"findings.json","artifacts":[{{"path":"findings.json","sha256":"{}","mediaType":"application/json"}},{{"path":"coverage.json","sha256":"{}","mediaType":"application/json"}}]}}}}"#, sha256::hex(&sha256::digest(&findings)), sha256::hex(&sha256::digest(&coverage))).into_bytes();
        (manifest, findings, coverage, b"# report\n".to_vec())
    }

    #[test]
    fn canonical_complete_deep_scan_is_bound_to_the_requested_revision() {
        let source = source();
        let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
        let scan = verify_scan(&manifest, &findings, &coverage, &report, &source).unwrap();
        assert_eq!(scan.id, "scan-1");
        assert_eq!(scan.critical, 0);
    }

    #[test]
    fn foreign_partial_and_high_finding_scans_are_refused() {
        let source = source();
        let (manifest, findings, coverage, report) = artifacts(None, "complete", &"c".repeat(40));
        assert!(verify_scan(&manifest, &findings, &coverage, &report, &source).is_err());
        let (manifest, findings, coverage, report) = artifacts(None, "partial", &source.commit);
        assert!(verify_scan(&manifest, &findings, &coverage, &report, &source).is_err());
        let (manifest, findings, coverage, report) =
            artifacts(Some("high"), "complete", &source.commit);
        assert!(verify_scan(&manifest, &findings, &coverage, &report, &source).is_err());
    }

    #[test]
    fn strict_profile_rejects_provider_drift_reversed_time_and_caveats() {
        let source = source();
        let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
        let manifest = String::from_utf8(manifest).unwrap();
        verify_scan(manifest.as_bytes(), &findings, &coverage, &report, &source)
            .expect("current qualified producer");
        let foreign = manifest.replace("codex-security-plugin", "other-provider");
        assert!(verify_scan(foreign.as_bytes(), &findings, &coverage, &report, &source).is_err());
        let stale = manifest.replace("0.1.32", "0.1.31");
        assert!(verify_scan(stale.as_bytes(), &findings, &coverage, &report, &source).is_err());
        let reversed = manifest.replace("2026-09-28T00:00:00Z", "2026-09-28T02:00:00Z");
        assert!(verify_scan(reversed.as_bytes(), &findings, &coverage, &report, &source).is_err());
        let limited = manifest.replace(
            r#""scope":{"includePaths"#,
            r#""limitations":["partial"],"scope":{"includePaths"#,
        );
        assert!(verify_scan(limited.as_bytes(), &findings, &coverage, &report, &source).is_err());
        let limited_scope = manifest.replace(
            r#""scope":{"includePaths":["."],"excludePaths":[]}"#,
            r#""scope":{"includePaths":["."],"excludePaths":[],"validationMode":"sampled"}"#,
        );
        assert!(
            verify_scan(
                limited_scope.as_bytes(),
                &findings,
                &coverage,
                &report,
                &source,
            )
            .is_err()
        );
    }

    /// Replace one coverage fragment and re-bind the manifest to the new bytes,
    /// so only the coverage rule under test can refuse the scan.
    fn with_coverage(from: &str, to: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let source = source();
        let (manifest, findings, coverage, report) = artifacts(None, "complete", &source.commit);
        let old = String::from_utf8(coverage).unwrap();
        assert!(old.contains(from), "fixture lacks {from}");
        let coverage = old.replacen(from, to, 1).into_bytes();
        let manifest = String::from_utf8(manifest)
            .unwrap()
            .replace(
                &sha256::hex(&sha256::digest(old.as_bytes())),
                &sha256::hex(&sha256::digest(&coverage)),
            )
            .into_bytes();
        (manifest, findings, coverage, report)
    }

    #[test]
    fn coverage_must_be_the_complete_unqualified_repository_workflow() {
        let source = source();
        let (manifest, findings, coverage, report) =
            with_coverage(r#""mode":"deep_repository""#, r#""mode":"deep_repository""#);
        verify_scan(&manifest, &findings, &coverage, &report, &source).unwrap();
        for (from, to) in [
            (
                r#""mode":"deep_repository""#,
                r#""mode":"quick_repository""#,
            ),
            (
                r#""inventoryStrategy":"repository""#,
                r#""inventoryStrategy":"sampled""#,
            ),
            (
                r#""explicitExclusions":[]"#,
                r#""explicitExclusions":["vm"]"#,
            ),
            (r#""deferred":[]"#, r#""deferred":["kernel"]"#),
            (r#""surfaces":[]"#, r#""surfaces":[{}]"#),
        ] {
            let (manifest, findings, coverage, report) = with_coverage(from, to);
            assert!(
                verify_scan(&manifest, &findings, &coverage, &report, &source).is_err(),
                "accepted coverage with {to}"
            );
        }
    }

    #[test]
    fn manifest_hashes_are_semantic_not_decorative() {
        let source = source();
        let (manifest, mut findings, coverage, report) =
            artifacts(None, "complete", &source.commit);
        findings.push(b' ');
        assert!(verify_scan(&manifest, &findings, &coverage, &report, &source).is_err());
    }

    #[test]
    fn request_is_exactly_source_and_workflow_bound() {
        let source = source();
        let request = render_request(&source);
        verify_request(&request, &source).unwrap();
        assert!(
            verify_request(
                &request,
                &SourceIdentity {
                    commit: "c".repeat(40),
                    tree: source.tree
                }
            )
            .is_err()
        );
    }
}
