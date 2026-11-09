use super::*;

#[derive(Debug)]
pub(super) struct AdvisoryAttempt {
    paths: Paths,
    record: Record,
    tool: Tool,
}

pub(crate) fn qualify_advisory(paths: &Paths) -> Result<()> {
    begin_advisory(paths)?.run()
}

fn begin_advisory(paths: &Paths) -> Result<AdvisoryAttempt> {
    begin_advisory_with(paths, || cargo_audit(paths.root()))
}

pub(super) fn begin_advisory_with(
    paths: &Paths,
    resolve: impl FnOnce() -> Result<Option<Tool>>,
) -> Result<AdvisoryAttempt> {
    invalidate(paths, Kind::Advisory)?;
    let tool = resolve()?
        .context("cargo-audit is unavailable; install it before phase-close qualification")?;
    let directory = kind_directory(paths, Kind::Advisory)?;
    let policy_directory = directory.join(".cargo");
    prepare_contained_real_directory(
        &qualification_root(paths),
        &policy_directory,
        "advisory policy directory",
    )?;
    let policy = directory.join(ADVISORY_POLICY_PATH);
    write_new_file(&policy, ADVISORY_POLICY.as_bytes())?;
    let record = advisory_input_record(paths, &tool)?;
    Ok(AdvisoryAttempt {
        paths: paths.clone(),
        record,
        tool,
    })
}

impl AdvisoryAttempt {
    fn run(mut self) -> Result<()> {
        ensure!(
            advisory_input_record(&self.paths, &self.tool)? == self.record,
            "source, Cargo.lock, policy or cargo-audit changed before advisory execution"
        );
        let directory = kind_directory(&self.paths, Kind::Advisory)?;
        let database = directory.join(ADVISORY_DATABASE);
        ensure!(
            std::fs::symlink_metadata(&database)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
            "fresh advisory qualification database already exists"
        );
        let lock = self.paths.root().join("Cargo.lock");
        let refresh = advisory_command(&self.tool, &directory, &database, &lock, true);
        let refresh = crate::bounded_process::output(
            refresh,
            ADVISORY_TIMEOUT,
            "fresh RustSec advisory database population",
        )?;
        ensure!(
            refresh.status.code() == Some(0),
            "fresh cargo-audit population rejected Cargo.lock ({})",
            refresh.status
        );
        let database_identity = advisory_database_identity(&directory, &database)?;
        ensure!(
            advisory_input_record(&self.paths, &self.tool)? == self.record,
            "source, Cargo.lock, policy or cargo-audit changed during advisory refresh"
        );
        let refresh_transcript = render_advisory_transcript(
            &refresh,
            &database_identity.revision,
            DatabaseBinding::Fetched,
        )?;
        let command = advisory_command(&self.tool, &directory, &database, &lock, false);
        let output = crate::bounded_process::output(
            command,
            ADVISORY_TIMEOUT,
            "revision-pinned RustSec cargo-audit qualification",
        )?;
        ensure!(
            output.status.code() == Some(0),
            "cargo-audit rejected Cargo.lock ({})",
            output.status
        );
        ensure!(
            advisory_database_identity(&directory, &database)? == database_identity,
            "advisory database changed during the revision-pinned audit"
        );
        let transcript = render_advisory_transcript(
            &output,
            &database_identity.revision,
            DatabaseBinding::Pinned,
        )?;
        let refresh_transcript_path = directory.join(ADVISORY_REFRESH_TRANSCRIPT);
        write_new_file(&refresh_transcript_path, &refresh_transcript)?;
        add_artifact(
            &mut self.record,
            "artifact.refresh-transcript",
            ADVISORY_REFRESH_TRANSCRIPT,
            &refresh_transcript_path,
        )?;
        let transcript_path = directory.join(ADVISORY_TRANSCRIPT);
        write_new_file(&transcript_path, &transcript)?;
        add_artifact(
            &mut self.record,
            "artifact.transcript",
            ADVISORY_TRANSCRIPT,
            &transcript_path,
        )?;
        self.record
            .insert("database.revision", &database_identity.revision)?;
        self.record
            .insert("database.tree", &database_identity.tree)?;
        self.record.insert("database.url", ADVISORY_DATABASE_URL)?;
        self.record.insert("execution.command", ADVISORY_COMMAND)?;
        self.record.insert("execution.exit-code", "0")?;
        self.record
            .insert("execution.refresh-command", ADVISORY_REFRESH_COMMAND)?;
        self.record.insert("execution.refresh-exit-code", "0")?;
        self.record.insert(
            "observation.utc-unix-seconds",
            utc_unix_seconds()?.to_string(),
        )?;
        let current = advisory_input_record(&self.paths, &self.tool)?;
        require_record_inputs(&self.record, &current)?;
        ensure!(
            advisory_database_identity(&directory, &database)? == database_identity,
            "advisory database changed after cargo-audit completed"
        );
        let published_refresh = read_bounded_file(
            &refresh_transcript_path,
            MAX_ADVISORY_TRANSCRIPT_BYTES,
            "cargo-audit population transcript",
        )?;
        ensure!(
            published_refresh == refresh_transcript,
            "cargo-audit population transcript changed during qualification publication"
        );
        let published = read_bounded_file(
            &transcript_path,
            MAX_ADVISORY_TRANSCRIPT_BYTES,
            "cargo-audit transcript",
        )?;
        ensure!(
            published == transcript,
            "cargo-audit transcript changed during qualification publication"
        );
        let attestation = verify_advisory_transcripts(
            &published_refresh,
            &published,
            &database_identity.revision,
        )?;
        self.record
            .insert("database.last-updated", &attestation.last_updated)?;
        self.record.insert("result", PASS)?;
        commit_record(&self.paths, Kind::Advisory, &self.record)
    }
}

pub(super) fn advisory_command(
    tool: &Tool,
    directory: &Path,
    database: &Path,
    lock: &Path,
    fetch: bool,
) -> Command {
    let mut command = Command::new(&tool.path);
    command
        .current_dir(directory)
        .env_remove("CARGO_NET_OFFLINE")
        .args(["audit", "--color", "never", "--db"])
        .arg(database)
        .args(["--deny", "warnings", "--file"])
        .arg(lock)
        .arg("--json");
    if !fetch {
        command.arg("--no-fetch");
    }
    command.args(["--url", ADVISORY_DATABASE_URL]);
    command
}

fn advisory_input_record(paths: &Paths, tool: &Tool) -> Result<Record> {
    let mut record = base_record(paths, Kind::Advisory)?;
    add_tool(&mut record, "input.cargo-audit", tool)?;
    add_digest(
        &mut record,
        "input.cargo-lock",
        &paths.root().join("Cargo.lock"),
    )?;
    add_artifact(
        &mut record,
        "artifact.policy",
        ADVISORY_POLICY_PATH,
        &kind_directory(paths, Kind::Advisory)?.join(ADVISORY_POLICY_PATH),
    )?;
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdvisoryDatabaseIdentity {
    revision: String,
    tree: String,
}

pub(super) fn advisory_database_identity(
    parent: &Path,
    database: &Path,
) -> Result<AdvisoryDatabaseIdentity> {
    require_contained_real_directory(parent, database, "RustSec advisory database")?;
    let revision = bounded_git_text(
        database,
        &["rev-parse", "HEAD"],
        MAX_GIT_IDENTITY_BYTES,
        "advisory database revision",
    )?;
    let tree = bounded_git_text(
        database,
        &["rev-parse", "HEAD^{tree}"],
        MAX_GIT_IDENTITY_BYTES,
        "advisory database tree",
    )?;
    let origin = bounded_git_text(
        database,
        &["remote", "get-url", "origin"],
        MAX_GIT_IDENTITY_BYTES,
        "advisory database origin",
    )?;
    let status = bounded_git_text(
        database,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        MAX_GIT_STATUS_BYTES,
        "advisory database status",
    )?;
    ensure!(
        is_git_id(&revision) && is_git_id(&tree),
        "advisory database returned a malformed Git identity"
    );
    ensure!(
        origin == ADVISORY_DATABASE_URL,
        "advisory database origin differs from the RustSec authority"
    );
    ensure!(status.is_empty(), "advisory database checkout is dirty");
    Ok(AdvisoryDatabaseIdentity { revision, tree })
}

fn bounded_git_text(root: &Path, args: &[&str], maximum: usize, label: &str) -> Result<String> {
    let text = git_output(root, args, maximum, label)?;
    ensure!(
        !text.contains('\0'),
        "advisory database Git output contains NUL"
    );
    Ok(text)
}

fn utc_unix_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("the host clock predates the Unix epoch")
        .map(|elapsed| elapsed.as_secs())
}

pub(super) fn render_advisory_transcript(
    output: &Output,
    database_revision: &str,
    binding: DatabaseBinding,
) -> Result<Vec<u8>> {
    render_advisory_transcript_with_limit(
        output,
        database_revision,
        binding,
        MAX_ADVISORY_TRANSCRIPT_BYTES,
    )
}

pub(super) fn render_advisory_transcript_with_limit(
    output: &Output,
    database_revision: &str,
    binding: DatabaseBinding,
    maximum: u64,
) -> Result<Vec<u8>> {
    ensure!(
        output.status.code() == Some(0),
        "only a successful cargo-audit result can be recorded"
    );
    verify_advisory_report(&output.stdout, database_revision, binding)?;
    let diagnostic = std::str::from_utf8(&output.stderr)
        .context("cargo-audit diagnostic output is not UTF-8")?;
    ensure!(
        !diagnostic.contains('\0'),
        "cargo-audit diagnostic output contains NUL"
    );
    let stdout_digest = sha256::hex(&sha256::digest(&output.stdout));
    let stderr_digest = sha256::hex(&sha256::digest(&output.stderr));
    let header = format!(
        "{ADVISORY_TRANSCRIPT_FORMAT}\nexit.code=0\nstderr.bytes={}\nstderr.sha256={stderr_digest}\nstdout.bytes={}\nstdout.sha256={stdout_digest}\n\n",
        output.stderr.len(),
        output.stdout.len(),
    );
    let length = header
        .len()
        .checked_add(output.stdout.len())
        .and_then(|length| length.checked_add(output.stderr.len()))
        .context("cargo-audit transcript length overflowed")?;
    ensure!(
        length as u64 <= maximum,
        "cargo-audit transcript exceeds its bounded artifact limit"
    );
    let mut transcript = Vec::with_capacity(length);
    transcript.extend_from_slice(header.as_bytes());
    transcript.extend_from_slice(&output.stdout);
    transcript.extend_from_slice(&output.stderr);
    Ok(transcript)
}

/// The population run's attestation, once the pinned verdict agrees with it.
pub(super) struct AdvisoryAttestation {
    pub(super) last_updated: String,
}

pub(super) fn verify_advisory_transcripts(
    refresh: &[u8],
    pinned: &[u8],
    database_revision: &str,
) -> Result<AdvisoryAttestation> {
    let fetched = verify_advisory_transcript(refresh, database_revision, DatabaseBinding::Fetched)?;
    let audited = verify_advisory_transcript(pinned, database_revision, DatabaseBinding::Pinned)?;
    ensure!(
        audited.advisory_count == fetched.advisory_count,
        "the pinned audit and the population run saw different advisory databases"
    );
    let last_updated = fetched
        .last_updated
        .context("a fetched cargo-audit report names its database update time")?;
    Ok(AdvisoryAttestation { last_updated })
}

fn verify_advisory_transcript(
    transcript: &[u8],
    database_revision: &str,
    binding: DatabaseBinding,
) -> Result<AdvisoryFacts> {
    verify_advisory_transcript_with_limit(
        transcript,
        database_revision,
        binding,
        MAX_ADVISORY_TRANSCRIPT_BYTES,
    )
}

pub(super) fn verify_advisory_transcript_with_limit(
    transcript: &[u8],
    database_revision: &str,
    binding: DatabaseBinding,
    maximum: u64,
) -> Result<AdvisoryFacts> {
    ensure!(
        transcript.len() as u64 <= maximum,
        "cargo-audit transcript exceeds its bounded artifact limit"
    );
    let split = transcript
        .windows(2)
        .position(|window| window == b"\n\n")
        .context("cargo-audit transcript has no payload delimiter")?;
    let payload = split
        .checked_add(2)
        .context("cargo-audit transcript delimiter overflowed")?;
    let header = std::str::from_utf8(&transcript[..split])
        .context("cargo-audit transcript header is not UTF-8")?;
    let lines: Vec<&str> = header.split('\n').collect();
    ensure!(
        lines.len() == 6 && lines[0] == ADVISORY_TRANSCRIPT_FORMAT,
        "cargo-audit transcript header schema differs"
    );
    ensure!(
        lines[1] == "exit.code=0",
        "cargo-audit transcript does not record a successful result"
    );
    let stderr_bytes = canonical_usize_field(lines[2], "stderr.bytes=")?;
    let stderr_digest = digest_field(lines[3], "stderr.sha256=")?;
    let stdout_bytes = canonical_usize_field(lines[4], "stdout.bytes=")?;
    let stdout_digest = digest_field(lines[5], "stdout.sha256=")?;
    let stdout_end = payload
        .checked_add(stdout_bytes)
        .context("cargo-audit stdout length overflowed")?;
    let stderr_end = stdout_end
        .checked_add(stderr_bytes)
        .context("cargo-audit stderr length overflowed")?;
    ensure!(
        stderr_end == transcript.len(),
        "cargo-audit transcript payload lengths differ"
    );
    let stdout = &transcript[payload..stdout_end];
    let stderr = &transcript[stdout_end..stderr_end];
    ensure!(
        sha256::hex(&sha256::digest(stdout)) == stdout_digest,
        "cargo-audit stdout digest differs"
    );
    ensure!(
        sha256::hex(&sha256::digest(stderr)) == stderr_digest,
        "cargo-audit stderr digest differs"
    );
    let facts = verify_advisory_report(stdout, database_revision, binding)?;
    let diagnostic =
        std::str::from_utf8(stderr).context("cargo-audit transcript stderr is not UTF-8")?;
    ensure!(
        !diagnostic.contains('\0'),
        "cargo-audit transcript stderr contains NUL"
    );
    Ok(facts)
}

fn canonical_usize_field(line: &str, prefix: &str) -> Result<usize> {
    let value = line
        .strip_prefix(prefix)
        .with_context(|| format!("cargo-audit transcript has no `{prefix}` field"))?;
    let parsed = <usize as core::str::FromStr>::from_str(value)
        .with_context(|| format!("cargo-audit transcript `{prefix}` is not a number"))?;
    ensure!(
        parsed.to_string() == value,
        "cargo-audit transcript `{prefix}` is not canonical"
    );
    Ok(parsed)
}

fn digest_field<'a>(line: &'a str, prefix: &str) -> Result<&'a str> {
    let value = line
        .strip_prefix(prefix)
        .with_context(|| format!("cargo-audit transcript has no `{prefix}` field"))?;
    ensure!(
        is_digest(value),
        "cargo-audit transcript `{prefix}` is malformed"
    );
    Ok(value)
}

/// How a cargo-audit report can name the database it audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DatabaseBinding {
    /// The run fetched, so cargo-audit itself names the commit it audited.
    Fetched,
    /// The run was `--no-fetch`; cargo-audit learns the commit only from a
    /// fetch, so the report carries null identity and Git binds it (D-297).
    Pinned,
}

/// What one verified cargo-audit report says about its database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AdvisoryFacts {
    pub(super) advisory_count: u64,
    pub(super) last_updated: Option<String>,
}

pub(super) fn verify_advisory_report(
    bytes: &[u8],
    database_revision: &str,
    binding: DatabaseBinding,
) -> Result<AdvisoryFacts> {
    let report = parse_json(bytes)?;
    let report = json_object(&report, "cargo-audit report")?;
    require_json_fields(
        report,
        &[
            "database",
            "lockfile",
            "settings",
            "vulnerabilities",
            "warnings",
        ],
        "cargo-audit report",
    )?;

    let database = json_object(json_field(report, "database")?, "cargo-audit database")?;
    require_json_fields(
        database,
        &["advisory-count", "last-commit", "last-updated"],
        "cargo-audit database",
    )?;
    let advisory_count = json_u64(json_field(database, "advisory-count")?, "advisory-count")?;
    ensure!(
        advisory_count != 0,
        "cargo-audit report names an empty advisory database"
    );
    let last_updated = match (
        binding,
        json_field(database, "last-commit")?,
        json_field(database, "last-updated")?,
    ) {
        (_, JsonValue::String(commit), JsonValue::String(updated)) => {
            ensure!(
                commit == database_revision,
                "cargo-audit report belongs to another advisory database revision"
            );
            ensure!(
                is_rfc3339_timestamp(updated),
                "cargo-audit report has no RFC3339 database update time"
            );
            Some(updated.clone())
        }
        (DatabaseBinding::Pinned, JsonValue::Null, JsonValue::Null) => None,
        _ => bail!("cargo-audit report has no usable advisory database identity"),
    };

    let lockfile = json_object(json_field(report, "lockfile")?, "cargo-audit lockfile")?;
    require_json_fields(lockfile, &["dependency-count"], "cargo-audit lockfile")?;
    ensure!(
        json_u64(
            json_field(lockfile, "dependency-count")?,
            "dependency-count"
        )? != 0,
        "cargo-audit report names an empty lockfile"
    );

    let settings = json_object(json_field(report, "settings")?, "cargo-audit settings")?;
    require_json_fields(
        settings,
        &[
            "ignore",
            "informational_warnings",
            "severity",
            "target_arch",
            "target_os",
        ],
        "cargo-audit settings",
    )?;
    ensure!(
        json_array(json_field(settings, "target_arch")?, "target_arch")?.is_empty()
            && json_array(json_field(settings, "target_os")?, "target_os")?.is_empty()
            && matches!(json_field(settings, "severity")?, JsonValue::Null)
            && json_array(json_field(settings, "ignore")?, "ignore")?.is_empty()
            && json_string_array(
                json_field(settings, "informational_warnings")?,
                "informational_warnings"
            )? == ["unmaintained", "unsound", "notice"],
        "cargo-audit report was generated with different audit settings"
    );

    let vulnerabilities = json_object(
        json_field(report, "vulnerabilities")?,
        "cargo-audit vulnerabilities",
    )?;
    require_json_fields(
        vulnerabilities,
        &["count", "found", "list"],
        "cargo-audit vulnerabilities",
    )?;
    ensure!(
        matches!(
            json_field(vulnerabilities, "found")?,
            JsonValue::Bool(false)
        ) && json_u64(json_field(vulnerabilities, "count")?, "vulnerability count")? == 0
            && json_array(json_field(vulnerabilities, "list")?, "vulnerability list")?.is_empty(),
        "cargo-audit report is not a consistent vulnerability-free result"
    );

    let warnings = json_object(json_field(report, "warnings")?, "cargo-audit warnings")?;
    ensure!(
        warnings.is_empty(),
        "cargo-audit report contains a dependency warning"
    );
    Ok(AdvisoryFacts {
        advisory_count,
        last_updated,
    })
}

/// The resolved RustSec scanner, if one is available on `PATH`.
pub(crate) fn cargo_audit(root: &Path) -> Result<Option<Tool>> {
    let Some(path) = find_on_path("cargo-audit") else {
        return Ok(None);
    };
    let path = std::fs::canonicalize(&path)
        .with_context(|| format!("resolving cargo-audit {}", path.display()))?;
    ensure!(path.is_file(), "resolved cargo-audit is not a regular file");
    let report = output(root, &path, &["audit", "--version"], ExitPolicy::Success)?;
    let version = cargo_audit_version(&report)?;
    Ok(Some(Tool { path, version }))
}

pub(super) fn cargo_audit_version(report: &str) -> Result<String> {
    ensure!(
        !report.contains(['\0', '\r', '\n']),
        "cargo-audit version identity is not one canonical line"
    );
    let version = report
        .strip_prefix("cargo-audit-audit ")
        .or_else(|| report.strip_prefix("cargo-audit "))
        .context("cargo-audit printed no supported version identity")?;
    ensure!(
        !version.is_empty()
            && version.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+' | b'_')
            }),
        "cargo-audit version is malformed"
    );
    ensure!(
        report == ADVISORY_TOOL_VERSION,
        "cargo-audit version differs from the report schema qualification"
    );
    Ok(report.to_string())
}
