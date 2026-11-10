use super::*;

pub(super) fn kernel_for(kind: Kind) -> Result<Kernel> {
    match kind {
        Kind::VmwareSelfTest => Ok(Kernel::SelfTest),
        Kind::VmwareShipped => Ok(Kernel::Shipped),
        Kind::Advisory
        | Kind::Mutation
        | Kind::CargoMutantsGrade
        | Kind::CodexSecurityDeep
        | Kind::Model => {
            bail!("qualification kind has no VMware kernel")
        }
    }
}

pub(super) fn base_record(paths: &Paths, kind: Kind) -> Result<Record> {
    let source = source_identity(paths.root())?;
    base_record_for_source(kind, &source)
}

pub(super) fn base_record_for_source(kind: Kind, source: &SourceIdentity) -> Result<Record> {
    let mut record = Record::default();
    record.insert("kind", kind.slug())?;
    record.insert("source.commit", &source.commit)?;
    record.insert("source.tree", &source.tree)?;
    for (key, value) in crate::git::resolved()?.fields() {
        record.insert(key, value)?;
    }
    Ok(record)
}

pub(super) fn model_record(paths: &Paths) -> Result<Record> {
    let source = source_identity(paths.root())?;
    model_record_for_source(paths, &source)
}

pub(super) fn model_record_for_source(paths: &Paths, source: &SourceIdentity) -> Result<Record> {
    let java = java(paths.root())?.context(
        "Java is unavailable; model evidence cannot verify without a JVM (java) on PATH",
    )?;
    model_record_with_tool_and_source(paths, &java, source)
}

pub(super) fn model_record_with_tool(paths: &Paths, java: &Tool) -> Result<Record> {
    let source = source_identity(paths.root())?;
    model_record_with_tool_and_source(paths, java, &source)
}

fn model_record_with_tool_and_source(
    paths: &Paths,
    java: &Tool,
    source: &SourceIdentity,
) -> Result<Record> {
    let mut record = base_record_for_source(Kind::Model, source)?;
    add_tool(&mut record, "input.java", java)?;
    add_digest(
        &mut record,
        "input.tlc-jar",
        &paths.root().join("tools/tla/tla2tools.jar"),
    )?;
    let models = crate::git::tracked_tree_digest(paths.root(), "models")?;
    record.insert("input.models.files", models.files.to_string())?;
    record.insert("input.models.bytes", models.bytes.to_string())?;
    record.insert("input.models.sha256", models.sha256)?;
    Ok(record)
}

pub(super) fn require_current_source(paths: &Paths, record: &Record) -> Result<()> {
    require_source(record, &source_identity(paths.root())?)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct SourceIdentity {
    pub(crate) commit: String,
    pub(crate) tree: String,
}

pub(crate) fn source_identity(root: &Path) -> Result<SourceIdentity> {
    let before_status = git_output(
        root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        MAX_GIT_STATUS_BYTES,
        "qualification source status",
    )?;
    let before = git_source_identity(root, "qualification source")?;
    let after_status = git_output(
        root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
        MAX_GIT_STATUS_BYTES,
        "settled qualification source status",
    )?;
    let after = git_source_identity(root, "settled qualification source")?;
    require_stable_source_capture(&before_status, &before, &after_status, &after)?;
    Ok(after)
}

fn git_source_identity(root: &Path, label: &str) -> Result<SourceIdentity> {
    let commit = git_output(
        root,
        &["rev-parse", "HEAD"],
        MAX_GIT_IDENTITY_BYTES,
        &format!("{label} commit"),
    )?;
    let tree = git_output(
        root,
        &["rev-parse", "HEAD^{tree}"],
        MAX_GIT_IDENTITY_BYTES,
        &format!("{label} tree"),
    )?;
    ensure!(is_git_id(&commit), "Git returned an invalid commit ID");
    ensure!(is_git_id(&tree), "Git returned an invalid tree ID");
    Ok(SourceIdentity { commit, tree })
}

pub(super) fn require_stable_source_capture(
    before_status: &str,
    before: &SourceIdentity,
    after_status: &str,
    after: &SourceIdentity,
) -> Result<()> {
    ensure!(
        before_status.trim().is_empty() && after_status.trim().is_empty(),
        "qualification requires a stable clean checkout:\n{before_status}{after_status}"
    );
    ensure!(
        before == after,
        "qualification source identity changed while it was captured"
    );
    Ok(())
}

pub(super) fn require_source(record: &Record, source: &SourceIdentity) -> Result<()> {
    ensure!(
        record.get("source.commit")? == source.commit,
        "qualification record belongs to another commit"
    );
    ensure!(
        record.get("source.tree")? == source.tree,
        "qualification record belongs to another tree"
    );
    Ok(())
}

pub(super) fn invalidate(paths: &Paths, kind: Kind) -> Result<()> {
    let root = crate::mutate::workspace::prepare_real_directory_tree(
        &qualification_root(paths),
        "qualification root",
    )?;
    require_real_root(&root, "qualification root")?;
    let directory = checked_kind_directory(&root, kind)?;
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            require_contained_real_directory(&root, &directory, "qualification kind directory")?;
            std::fs::remove_dir_all(&directory)
                .with_context(|| format!("invalidating stale {} qualification", kind.slug()))?;
        }
        Ok(_) => {
            std::fs::remove_file(&directory).with_context(|| {
                format!("removing unexpected {} qualification entry", kind.slug())
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", directory.display()));
        }
    }
    std::fs::create_dir(&directory).with_context(|| format!("creating {}", directory.display()))
}

pub(super) fn invalidate_record(paths: &Paths, kind: Kind) -> Result<()> {
    let root = crate::mutate::workspace::prepare_real_directory_tree(
        &qualification_root(paths),
        "qualification root",
    )?;
    let directory = checked_kind_directory(&root, kind)?;
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => ensure!(
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
            "qualification kind directory is redirected or not a directory"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&directory)
                .with_context(|| format!("creating {}", directory.display()))?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", directory.display()));
        }
    }
    require_contained_real_directory(&root, &directory, "qualification kind directory")?;
    for name in [RECORD, RECORD_TEMP] {
        let path = directory.join(name);
        if crate::mutate::workspace::require_direct_file_if_present(
            &directory,
            &path,
            "external qualification record",
        )? {
            std::fs::remove_file(&path).with_context(|| {
                format!("invalidating stale qualification record {}", path.display())
            })?;
        }
    }
    Ok(())
}

pub(super) fn commit_record(paths: &Paths, kind: Kind, record: &Record) -> Result<()> {
    validate_record(record, kind)?;
    crate::git::resolved()?.require_unchanged()?;
    let directory = kind_directory(paths, kind)?;
    let temporary = directory.join(RECORD_TEMP);
    let final_path = directory.join(RECORD);
    write_new_file(&temporary, record.render().as_bytes())?;
    ensure!(
        read_record(&temporary, kind)? == *record,
        "qualification record changed before commit"
    );
    require_current_source(paths, record)?;
    std::fs::rename(&temporary, &final_path)
        .with_context(|| format!("committing {}", final_path.display()))
}

pub(super) fn read_record(path: &Path, kind: Kind) -> Result<Record> {
    require_regular_file(path, "qualification record")?;
    let text = read_bounded_text(path, MAX_RECORD_BYTES, "qualification record")?;
    let record = parse_record(&text)?;
    validate_record(&record, kind)?;
    Ok(record)
}

pub(super) fn parse_record(text: &str) -> Result<Record> {
    ensure!(
        !text.contains('\r'),
        "qualification record must use LF endings"
    );
    let without_lf = text
        .strip_suffix('\n')
        .context("qualification record has no terminal LF")?;
    ensure!(
        !without_lf.ends_with('\n'),
        "qualification record has more than one terminal LF"
    );
    let mut lines: Vec<&str> = without_lf.split('\n').collect();
    ensure!(
        lines.first().copied() == Some(FORMAT),
        "qualification record has no supported header"
    );
    let checksum = lines
        .pop()
        .context("qualification record has no checksum")?;
    let expected = checksum
        .strip_prefix("checksum.sha256=")
        .context("qualification checksum line is malformed")?;
    ensure!(is_digest(expected), "qualification checksum is malformed");
    let mut body = lines.join("\n");
    body.push('\n');
    let actual = sha256::hex(&sha256::digest(body.as_bytes()));
    ensure!(actual == expected, "qualification checksum does not match");

    let mut record = Record::default();
    let mut previous = None;
    for line in lines.into_iter().skip(1) {
        ensure!(
            !line.is_empty(),
            "qualification record contains a blank line"
        );
        let (key, value) = line
            .split_once('=')
            .with_context(|| format!("qualification line has no value: {line}"))?;
        if let Some(previous) = previous {
            ensure!(previous < key, "qualification keys are not strictly sorted");
        }
        record.insert(key, value)?;
        previous = Some(key);
    }
    ensure!(!record.fields.is_empty(), "qualification record is empty");
    Ok(record)
}

struct RecordSchema {
    fields: &'static [&'static str],
    tools: &'static [&'static str],
    artifacts: Vec<&'static str>,
}

fn record_schema(kind: Kind) -> RecordSchema {
    match kind {
        Kind::Advisory => RecordSchema {
            fields: &[
                "database.last-updated",
                "database.revision",
                "database.tree",
                "database.url",
                "execution.command",
                "execution.exit-code",
                "execution.refresh-command",
                "execution.refresh-exit-code",
                "input.cargo-lock.bytes",
                "input.cargo-lock.sha256",
                "observation.utc-unix-seconds",
            ],
            tools: &["input.cargo-audit"],
            artifacts: vec![
                "artifact.policy",
                "artifact.refresh-transcript",
                "artifact.transcript",
            ],
        },
        Kind::Mutation => RecordSchema {
            fields: &[
                "input.index.sha256",
                "input.mutations",
                "input.registry.sha256",
                "input.source.sha256",
            ],
            tools: &[],
            artifacts: vec!["artifact.report"],
        },
        Kind::CargoMutantsGrade => RecordSchema {
            fields: &[
                "execution.authority",
                "grade.adjudicated",
                "grade.caught",
                "grade.missed",
                "grade.timeout",
                "grade.total",
                "grade.unviable",
                "tool.cargo-mutants.bytes",
                "tool.cargo-mutants.sha256",
                "tool.cargo-mutants.version",
                "tool.cargo-nextest.archive.sha256",
                "tool.cargo-nextest.bytes",
                "tool.cargo-nextest.commit",
                "tool.cargo-nextest.sha256",
                "tool.cargo-nextest.url",
                "tool.cargo-nextest.version",
            ],
            tools: &[],
            artifacts: vec![
                "artifact.request",
                "artifact.inventory",
                "artifact.mutants",
                "artifact.outcomes",
                "artifact.adjudications",
                "artifact.doctest",
            ],
        },
        Kind::CodexSecurityDeep => RecordSchema {
            fields: &[
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
                "execution.worker-preflight.allowed-reads",
                "execution.worker-preflight.denied-reads",
                "execution.worker-preflight.denied-write-creates",
                "execution.worker-preflight.denied-write-mutations",
                "execution.worker-preflight.denied-network-listeners",
                "execution.worker-preflight.denied-network-connections",
                "execution.worker-preflight.denied-descendants",
                "execution.worker-transitions.app-server",
                "execution.worker-transitions.exec-initial",
                "execution.worker-transitions.exec-resumed",
                "findings.critical",
                "findings.high",
                "findings.informational",
                "findings.low",
                "findings.medium",
                "findings.total",
                "scan.id",
                "scan.producer.name",
                "scan.producer.version",
                "scan.surfaces",
                "tool.codex-security.integrity.bytes",
                "tool.codex-security.integrity.sha256",
                "tool.codex-security.tree.files",
                "tool.codex-security.version",
                "tool.codex.runtime.bytes",
                "tool.codex.runtime.files",
                "tool.codex.runtime.root",
                "tool.codex.runtime.sha256",
                "tool.python.runtime.bytes",
                "tool.python.runtime.files",
                "tool.python.runtime.root",
                "tool.python.runtime.sha256",
            ],
            tools: &["tool.node", "tool.python", "tool.codex"],
            artifacts: vec![
                "artifact.request",
                "artifact.manifest",
                "artifact.findings",
                "artifact.coverage",
                "artifact.report",
                "artifact.worker-transitions",
            ],
        },
        Kind::Model => RecordSchema {
            fields: &[
                "input.models.bytes",
                "input.models.files",
                "input.models.sha256",
                "input.tlc-jar.bytes",
                "input.tlc-jar.sha256",
            ],
            tools: &["input.java"],
            artifacts: crate::model::TRANSCRIPT_ARTIFACTS
                .iter()
                .map(|artifact| artifact.prefix)
                .collect(),
        },
        Kind::VmwareSelfTest | Kind::VmwareShipped => RecordSchema {
            fields: &["execution.vmrun.command", "execution.vmrun.running"],
            tools: &["input.vmrun"],
            artifacts: vec![
                "artifact.generation",
                "artifact.image",
                "artifact.namespace",
                "artifact.vmdk",
                "artifact.namespace-vmdk",
                "artifact.vmx",
                "artifact.handoff",
                "artifact.transcript",
                "artifact.observer",
            ],
        },
    }
}

fn expected_keys(kind: Kind) -> BTreeSet<String> {
    let schema = record_schema(kind);
    let mut expected: BTreeSet<String> = ["kind", "result", "source.commit", "source.tree"]
        .into_iter()
        .chain(crate::git::FIELDS)
        .chain(schema.fields.iter().copied())
        .map(str::to_string)
        .collect();
    for prefix in schema.tools {
        add_tool_keys(&mut expected, prefix);
    }
    for prefix in schema.artifacts {
        add_artifact_keys(&mut expected, prefix);
    }
    expected
}

pub(super) fn validate_record(record: &Record, kind: Kind) -> Result<()> {
    let expected = expected_keys(kind);
    let actual: BTreeSet<String> = record.fields.keys().cloned().collect();
    ensure!(
        actual == expected,
        "qualification schema differs: missing {:?}; unknown {:?}",
        expected.difference(&actual).collect::<Vec<_>>(),
        actual.difference(&expected).collect::<Vec<_>>()
    );
    ensure!(
        record.get("kind")? == kind.slug(),
        "wrong qualification kind"
    );
    ensure!(record.get("result")? == PASS, "qualification did not pass");
    if kind == Kind::Advisory {
        validate_advisory_record(record)?;
    }
    if matches!(kind, Kind::VmwareSelfTest | Kind::VmwareShipped) {
        ensure!(
            record.get("execution.vmrun.command")? == "-T ws list"
                && record.get("execution.vmrun.running")? == "1",
            "VMware qualification has no exact running-machine observation"
        );
    }
    ensure!(
        is_git_id(record.get("source.commit")?),
        "qualification commit is malformed"
    );
    ensure!(
        is_git_id(record.get("source.tree")?),
        "qualification tree is malformed"
    );
    crate::git::validate_fields(|key| record.get(key))?;
    for (key, value) in &record.fields {
        if key.ends_with(".sha256") {
            ensure!(
                is_digest(value),
                "qualification digest `{key}` is malformed"
            );
        }
        if key.ends_with(".bytes") || key.ends_with(".files") || key == "input.mutations" {
            ensure!(
                <u64 as core::str::FromStr>::from_str(value).is_ok_and(|count| count != 0),
                "qualification count `{key}` is invalid"
            );
        }
    }
    Ok(())
}

fn validate_advisory_record(record: &Record) -> Result<()> {
    ensure!(
        record.get("database.url")? == ADVISORY_DATABASE_URL
            && is_git_id(record.get("database.revision")?)
            && is_git_id(record.get("database.tree")?),
        "advisory qualification database identity is malformed"
    );
    ensure!(
        is_rfc3339_timestamp(record.get("database.last-updated")?),
        "advisory qualification names no RFC3339 database update time"
    );
    ensure!(
        record.get("execution.command")? == ADVISORY_COMMAND
            && record.get("execution.exit-code")? == "0"
            && record.get("execution.refresh-command")? == ADVISORY_REFRESH_COMMAND
            && record.get("execution.refresh-exit-code")? == "0",
        "advisory qualification has no exact successful command"
    );
    let observed =
        <u64 as core::str::FromStr>::from_str(record.get("observation.utc-unix-seconds")?)
            .context("advisory UTC observation is not a number")?;
    ensure!(
        observed != 0 && observed.to_string() == record.get("observation.utc-unix-seconds")?,
        "advisory UTC observation is not canonical"
    );
    ensure!(
        cargo_audit_version(record.get("input.cargo-audit.version")?)?.as_str()
            == record.get("input.cargo-audit.version")?,
        "cargo-audit version identity is not canonical"
    );
    Ok(())
}

fn add_tool_keys(keys: &mut BTreeSet<String>, prefix: &str) {
    keys.extend([
        format!("{prefix}.bytes"),
        format!("{prefix}.sha256"),
        format!("{prefix}.version"),
    ]);
}

pub(super) fn add_artifact_keys(keys: &mut BTreeSet<String>, prefix: &str) {
    keys.extend([
        format!("{prefix}.bytes"),
        format!("{prefix}.path"),
        format!("{prefix}.sha256"),
    ]);
}

pub(super) fn add_tool(record: &mut Record, prefix: &str, tool: &Tool) -> Result<()> {
    let version = tool.version.replace('\n', "\\n");
    ensure!(!version.is_empty(), "{prefix} has no version identity");
    record.insert(format!("{prefix}.version"), version)?;
    add_digest(record, prefix, &tool.path)
}

pub(super) fn add_artifact(
    record: &mut Record,
    prefix: &str,
    name: &str,
    path: &Path,
) -> Result<()> {
    add_artifact_digest(record, prefix, name, &digest_file(path)?)
}

pub(super) fn add_artifact_digest(
    record: &mut Record,
    prefix: &str,
    name: &str,
    digest: &Digest,
) -> Result<()> {
    record.insert(format!("{prefix}.path"), name)?;
    record.insert(format!("{prefix}.bytes"), digest.bytes.to_string())?;
    record.insert(format!("{prefix}.sha256"), &digest.sha256)
}

pub(super) fn add_digest(record: &mut Record, prefix: &str, path: &Path) -> Result<()> {
    let digest = digest_file(path)?;
    record.insert(format!("{prefix}.bytes"), digest.bytes.to_string())?;
    record.insert(format!("{prefix}.sha256"), digest.sha256)?;
    Ok(())
}

pub(crate) fn validate_field(owner: &str, key: &str, value: &str) -> Result<()> {
    ensure!(
        !key.is_empty()
            && key.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte)
            }),
        "invalid {owner} key `{key}`"
    );
    ensure!(
        !value.contains(['\r', '\n', '\0']),
        "{owner} value for `{key}` contains a forbidden byte"
    );
    Ok(())
}

pub(super) fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
