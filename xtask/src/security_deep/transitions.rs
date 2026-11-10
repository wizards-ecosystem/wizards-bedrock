use super::*;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum WorkerTransitionKind {
    AppServerPassThrough,
    ExecInitialAttenuated,
    ExecResumedAttenuated,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum WorkerTransitionStage {
    Launched,
    Started,
    ExitedZero,
}

impl WorkerTransitionStage {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Launched => "launched",
            Self::Started => "started",
            Self::ExitedZero => "exited-zero",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "launched" => Ok(Self::Launched),
            "started" => Ok(Self::Started),
            "exited-zero" => Ok(Self::ExitedZero),
            _ => bail!("private Codex worker transition stage differs"),
        }
    }
}

impl WorkerTransitionKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::AppServerPassThrough => "app-server-pass-through",
            Self::ExecInitialAttenuated => "exec-initial-attenuated",
            Self::ExecResumedAttenuated => "exec-resumed-attenuated",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "app-server-pass-through" => Ok(Self::AppServerPassThrough),
            "exec-initial-attenuated" => Ok(Self::ExecInitialAttenuated),
            "exec-resumed-attenuated" => Ok(Self::ExecResumedAttenuated),
            _ => bail!("private Codex worker transition kind differs"),
        }
    }

    pub(super) fn applied_digest<'digest>(
        self,
        provider: &'digest str,
        effective: &'digest str,
    ) -> &'digest str {
        match self {
            Self::AppServerPassThrough => provider,
            Self::ExecInitialAttenuated | Self::ExecResumedAttenuated => effective,
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct WorkerTransitionReceipt {
    pub(super) kind: WorkerTransitionKind,
    pub(super) stage: WorkerTransitionStage,
    pub(super) process_id: u32,
    pub(super) source_commit: String,
    pub(super) provider_profile: String,
    pub(super) effective_profile: String,
    pub(super) applied_profile: String,
    pub(super) worker: String,
    pub(super) execution_thread: Option<String>,
    pub(super) arguments: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct WorkerTransitionLedger {
    pub(super) bytes: Vec<u8>,
    pub(super) app_server: usize,
    pub(super) exec_initial: usize,
    pub(super) exec_resumed: usize,
    pub(super) exec_exited_zero: usize,
    pub(super) thread_ids: BTreeSet<String>,
    pub(super) execution_threads: BTreeSet<String>,
    pub(super) probes: WorkerProbeSummary,
}

fn worker_transition_worker_digest(worker: &Path) -> Result<String> {
    let worker = worker
        .to_str()
        .context("private Codex worker directory is not UTF-8")?;
    ensure!(
        !worker.is_empty() && !worker.contains(['\r', '\n', '\0']),
        "private Codex worker directory is malformed"
    );
    let mut digest = sha256::Hasher::new();
    digest.update(&(worker.len() as u64).to_be_bytes())?;
    digest.update(worker.as_bytes())?;
    Ok(sha256::hex(&digest.finish()))
}

fn worker_transition_argument_digest(arguments: &[OsString]) -> Result<String> {
    let mut digest = sha256::Hasher::new();
    for argument in arguments {
        let argument = argument
            .to_str()
            .context("private Codex transition argument is not UTF-8")?;
        ensure!(
            !argument.contains(['\r', '\n', '\0']),
            "private Codex transition argument is malformed"
        );
        digest.update(&(argument.len() as u64).to_be_bytes())?;
        digest.update(argument.as_bytes())?;
    }
    Ok(sha256::hex(&digest.finish()))
}

fn require_worker_transition_shape(receipt: &WorkerTransitionReceipt) -> Result<()> {
    ensure!(
        receipt.process_id != 0
            && receipt.source_commit.len() == 40
            && receipt
                .source_commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "private Codex transition identity is malformed"
    );
    match (
        receipt.kind,
        receipt.stage,
        receipt.execution_thread.as_deref(),
    ) {
        (WorkerTransitionKind::AppServerPassThrough, WorkerTransitionStage::Launched, None) => {}
        (
            WorkerTransitionKind::ExecInitialAttenuated
            | WorkerTransitionKind::ExecResumedAttenuated,
            WorkerTransitionStage::Started | WorkerTransitionStage::ExitedZero,
            Some(thread),
        ) => require_safe_identity(thread, "private Codex transition execution thread")?,
        _ => bail!("private Codex transition kind, stage, and thread differ"),
    }
    Ok(())
}

pub(super) fn render_worker_transition_receipt(
    receipt: &WorkerTransitionReceipt,
) -> Result<Vec<u8>> {
    require_worker_transition_shape(receipt)?;
    for digest in [
        receipt.provider_profile.as_str(),
        receipt.effective_profile.as_str(),
        receipt.applied_profile.as_str(),
        receipt.worker.as_str(),
        receipt.arguments.as_str(),
    ] {
        require_sha256(digest, "private Codex transition digest")?;
    }
    let execution_thread = receipt.execution_thread.as_deref().unwrap_or("none");
    let body = format!(
        "{WORKER_TRANSITION_RECEIPT_SCHEMA}\nsource.commit={}\nwrapper.id={}\ntransition.kind={}\ntransition.stage={}\nworker.sha256={}\nexecution.thread={}\nprofile.provider.sha256={}\nprofile.effective.sha256={}\nprofile.applied.sha256={}\narguments.sha256={}\n",
        receipt.source_commit,
        receipt.process_id,
        receipt.kind.as_str(),
        receipt.stage.as_str(),
        receipt.worker,
        execution_thread,
        receipt.provider_profile,
        receipt.effective_profile,
        receipt.applied_profile,
        receipt.arguments,
    );
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    Ok(format!("{body}checksum.sha256={checksum}\n").into_bytes())
}

pub(super) fn parse_worker_transition_receipt(bytes: &[u8]) -> Result<WorkerTransitionReceipt> {
    ensure!(
        bytes.len() as u64 <= MAX_WORKER_TRANSITION_RECEIPT_BYTES,
        "private Codex transition receipt exceeds its size limit"
    );
    let text =
        std::str::from_utf8(bytes).context("private Codex transition receipt is not UTF-8")?;
    ensure!(
        text.ends_with('\n') && !text.contains('\r') && !text.contains('\0'),
        "private Codex transition receipt framing differs"
    );
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some(WORKER_TRANSITION_RECEIPT_SCHEMA),
        "private Codex transition receipt schema differs"
    );
    let mut values = BTreeMap::new();
    for expected in [
        "source.commit",
        "wrapper.id",
        "transition.kind",
        "transition.stage",
        "worker.sha256",
        "execution.thread",
        "profile.provider.sha256",
        "profile.effective.sha256",
        "profile.applied.sha256",
        "arguments.sha256",
    ] {
        let line = lines
            .next()
            .with_context(|| format!("private Codex transition receipt has no {expected}"))?;
        let (key, value) = line
            .split_once('=')
            .context("private Codex transition receipt field has no separator")?;
        ensure!(
            key == expected && !value.is_empty() && values.insert(key, value).is_none(),
            "private Codex transition receipt field order or uniqueness differs"
        );
    }
    let checksum = lines
        .next()
        .context("private Codex transition receipt has no checksum")?
        .strip_prefix("checksum.sha256=")
        .context("private Codex transition receipt checksum field differs")?;
    require_sha256(checksum, "private Codex transition receipt checksum")?;
    ensure!(
        lines.next().is_none(),
        "private Codex transition receipt has trailing fields"
    );
    let checksum_offset = text
        .rfind("checksum.sha256=")
        .context("private Codex transition receipt has no checksum offset")?;
    ensure!(
        sha256::hex(&sha256::digest(&text.as_bytes()[..checksum_offset])) == checksum,
        "private Codex transition receipt checksum differs"
    );
    let value = |key: &str| -> Result<&str> {
        values
            .get(key)
            .copied()
            .with_context(|| format!("private Codex transition receipt has no {key}"))
    };
    let process_id = value("wrapper.id")?
        .parse::<u32>()
        .context("private Codex transition wrapper identity is not an integer")?;
    let execution_thread = match value("execution.thread")? {
        "none" => None,
        value => Some(value.to_string()),
    };
    let receipt = WorkerTransitionReceipt {
        kind: WorkerTransitionKind::parse(value("transition.kind")?)?,
        stage: WorkerTransitionStage::parse(value("transition.stage")?)?,
        process_id,
        source_commit: value("source.commit")?.to_string(),
        provider_profile: value("profile.provider.sha256")?.to_string(),
        effective_profile: value("profile.effective.sha256")?.to_string(),
        applied_profile: value("profile.applied.sha256")?.to_string(),
        worker: value("worker.sha256")?.to_string(),
        execution_thread,
        arguments: value("arguments.sha256")?.to_string(),
    };
    ensure!(
        render_worker_transition_receipt(&receipt)? == bytes,
        "private Codex transition receipt is noncanonical"
    );
    Ok(receipt)
}

pub(super) fn write_worker_transition_receipt(
    source: &str,
    kind: WorkerTransitionKind,
    stage: WorkerTransitionStage,
    worker: &Path,
    execution_thread: Option<&str>,
    arguments: &[OsString],
) -> Result<()> {
    let transitions = PathBuf::from(
        std::env::var_os(WRAPPER_TRANSITIONS_ENV)
            .context("private Codex wrapper has no transition directory")?,
    );
    let transitions = canonical_directory(&transitions, "private Codex transition directory")?;
    let pycache = PathBuf::from(
        std::env::var_os(PYTHON_WRAPPER_PYCACHE_ENV)
            .context("private Codex wrapper has no Python bytecode directory")?,
    );
    let scratch = canonical_directory(
        pycache
            .parent()
            .context("private Python bytecode directory has no scratch parent")?,
        "private Codex transition scratch",
    )?;
    require_scratch_cleanup_target(&scratch)?;
    ensure!(
        transitions == scratch.join("transitions"),
        "private Codex transition directory escaped owned scratch"
    );
    let provider = std::env::var(WRAPPER_PROVIDER_PERMISSION_ENV)
        .context("private Codex transition has no provider profile")?;
    let effective = std::env::var(WRAPPER_WORKER_PERMISSION_ENV)
        .context("private Codex transition has no effective profile")?;
    let provider_profile = sha256::hex(&sha256::digest(provider.as_bytes()));
    let effective_profile = sha256::hex(&sha256::digest(effective.as_bytes()));
    let process_id = std::process::id();
    let receipt = WorkerTransitionReceipt {
        kind,
        stage,
        process_id,
        source_commit: source.to_string(),
        provider_profile: provider_profile.clone(),
        effective_profile: effective_profile.clone(),
        applied_profile: kind
            .applied_digest(&provider_profile, &effective_profile)
            .to_string(),
        worker: worker_transition_worker_digest(worker)?,
        execution_thread: execution_thread.map(str::to_string),
        arguments: worker_transition_argument_digest(arguments)?,
    };
    let name = format!("{}-{}-{process_id}.receipt", kind.as_str(), stage.as_str());
    write_new_file(
        &transitions.join(name),
        &render_worker_transition_receipt(&receipt)?,
    )
}

pub(super) fn require_worker_transition_receipt(
    receipt: &WorkerTransitionReceipt,
    source: &SourceIdentity,
    permissions: &WorkerPermissionDigests,
) -> Result<()> {
    ensure!(
        receipt.source_commit == source.commit
            && receipt.provider_profile == permissions.provider_profile
            && receipt.effective_profile == permissions.effective_profile
            && receipt.applied_profile
                == receipt.kind.applied_digest(
                    &permissions.provider_profile,
                    &permissions.effective_profile,
                ),
        "private Codex transition receipt differs from its source or permission authority"
    );
    require_worker_transition_shape(receipt)?;
    require_sha256(&receipt.worker, "private Codex transition worker digest")?;
    Ok(())
}

pub(super) fn render_worker_transition_ledger(
    receipts: &[WorkerTransitionReceipt],
    thread_ids: &BTreeSet<String>,
    execution_threads: &BTreeSet<String>,
    source: &SourceIdentity,
    permissions: &WorkerPermissionDigests,
    probes: &WorkerProbeSummary,
) -> Result<WorkerTransitionLedger> {
    probes.require_complete()?;
    for digest in [
        permissions.provider_profile.as_str(),
        permissions.effective_profile.as_str(),
        permissions.effective_state.as_str(),
    ] {
        require_sha256(digest, "private Codex worker-policy digest")?;
    }
    ensure!(
        !receipts.is_empty() && receipts.len() <= MAX_WORKER_TRANSITIONS,
        "private Codex transition count is outside its policy"
    );
    require_worker_thread_sets(thread_ids, execution_threads)?;
    let mut receipts = receipts.to_vec();
    receipts.sort();
    let mut process_stages = BTreeSet::new();
    let mut process_kinds = BTreeMap::new();
    for receipt in &receipts {
        require_worker_transition_receipt(receipt, source, permissions)?;
        ensure!(
            process_stages.insert((receipt.process_id, receipt.stage)),
            "private Codex transition wrapper stage is duplicated"
        );
        ensure!(
            process_kinds
                .insert(receipt.process_id, receipt.kind)
                .is_none_or(|kind| kind == receipt.kind),
            "private Codex transition wrapper changed kind"
        );
    }
    let app_server = receipts
        .iter()
        .filter(|receipt| {
            receipt.kind == WorkerTransitionKind::AppServerPassThrough
                && receipt.stage == WorkerTransitionStage::Launched
        })
        .count();
    let exec_initial = receipts
        .iter()
        .filter(|receipt| {
            receipt.kind == WorkerTransitionKind::ExecInitialAttenuated
                && receipt.stage == WorkerTransitionStage::Started
        })
        .count();
    let exec_resumed = receipts
        .iter()
        .filter(|receipt| {
            receipt.kind == WorkerTransitionKind::ExecResumedAttenuated
                && receipt.stage == WorkerTransitionStage::Started
        })
        .count();
    let exec_exited_zero = receipts
        .iter()
        .filter(|receipt| receipt.stage == WorkerTransitionStage::ExitedZero)
        .count();
    ensure!(
        app_server >= 1 && exec_initial >= 1,
        "private Codex transition ledger lacks an app-server or initial exec crossing"
    );
    let initial_starts = receipts
        .iter()
        .filter(|receipt| {
            receipt.kind == WorkerTransitionKind::ExecInitialAttenuated
                && receipt.stage == WorkerTransitionStage::Started
        })
        .collect::<Vec<_>>();
    let mut initial_threads = BTreeSet::new();
    let mut initial_pairs = BTreeSet::new();
    for receipt in &initial_starts {
        let thread = receipt
            .execution_thread
            .as_ref()
            .context("private Codex initial transition has no execution thread")?;
        ensure!(
            initial_threads.insert(thread.clone()),
            "private Codex initial execution thread is duplicated"
        );
        initial_pairs.insert((receipt.worker.clone(), thread.clone()));
    }
    ensure!(
        execution_threads.is_subset(&initial_threads),
        "Codex Security completion execution threads lack observed initial wrapper starts"
    );
    for receipt in receipts.iter().filter(|receipt| {
        receipt.kind == WorkerTransitionKind::ExecResumedAttenuated
            && receipt.stage == WorkerTransitionStage::Started
    }) {
        let thread = receipt
            .execution_thread
            .as_ref()
            .context("private Codex resumed transition has no execution thread")?;
        ensure!(
            initial_pairs.contains(&(receipt.worker.clone(), thread.clone())),
            "private Codex resumed execution thread lacks same-worker initial ancestry"
        );
    }
    let mut app_server_by_worker = BTreeMap::new();
    let mut exec_starts_by_worker = BTreeMap::new();
    for receipt in &receipts {
        if receipt.kind == WorkerTransitionKind::AppServerPassThrough
            && receipt.stage == WorkerTransitionStage::Launched
        {
            *app_server_by_worker
                .entry(receipt.worker.clone())
                .or_insert(0usize) += 1;
        } else if receipt.stage == WorkerTransitionStage::Started {
            *exec_starts_by_worker
                .entry(receipt.worker.clone())
                .or_insert(0usize) += 1;
        }
    }
    for (worker, exec_starts) in exec_starts_by_worker {
        ensure!(
            app_server_by_worker.get(&worker).copied().unwrap_or(0) >= exec_starts,
            "private Codex exec starts lack same-worker app-server launches"
        );
    }
    for receipt in receipts
        .iter()
        .filter(|receipt| receipt.stage == WorkerTransitionStage::ExitedZero)
    {
        ensure!(
            receipts.iter().any(|started| {
                started.stage == WorkerTransitionStage::Started
                    && started.kind == receipt.kind
                    && started.process_id == receipt.process_id
                    && started.worker == receipt.worker
                    && started.execution_thread == receipt.execution_thread
                    && started.applied_profile == receipt.applied_profile
                    && started.arguments == receipt.arguments
            }),
            "private Codex zero-exit transition lacks its matching start"
        );
    }
    let mut body = format!(
        "{WORKER_TRANSITIONS_SCHEMA}\nsource.commit={}\nprofile.provider.sha256={}\nprofile.effective.sha256={}\nprofile.effective-state.sha256={}\npreflight.allowed-reads={}\npreflight.denied-reads={}\npreflight.denied-write-creates={}\npreflight.denied-write-mutations={}\npreflight.denied-network-listeners={}\npreflight.denied-network-connections={}\npreflight.denied-descendants={}\ncompletion.threads={}\ncompletion.execution-threads={}\ntransition.app-server-launched={}\ntransition.exec-initial-started={}\ntransition.exec-resumed-started={}\ntransition.exec-exited-zero={}\ntransition.count={}\n",
        source.commit,
        permissions.provider_profile,
        permissions.effective_profile,
        permissions.effective_state,
        probes.allowed_reads,
        probes.denied_reads,
        probes.denied_write_creates,
        probes.denied_write_mutations,
        probes.denied_network_listeners,
        probes.denied_network_connections,
        probes.denied_descendants,
        thread_ids.len(),
        execution_threads.len(),
        app_server,
        exec_initial,
        exec_resumed,
        exec_exited_zero,
        receipts.len(),
    );
    for (index, thread) in thread_ids.iter().enumerate() {
        body.push_str(&format!("completion.thread.{index:04}={thread}\n"));
    }
    for (index, thread) in execution_threads.iter().enumerate() {
        body.push_str(&format!(
            "completion.execution-thread.{index:04}={thread}\n"
        ));
    }
    for (index, receipt) in receipts.iter().enumerate() {
        body.push_str(&format!(
            "transition.{index:04}.kind={}\ntransition.{index:04}.stage={}\ntransition.{index:04}.wrapper={}\ntransition.{index:04}.worker.sha256={}\ntransition.{index:04}.execution.thread={}\ntransition.{index:04}.profile.applied.sha256={}\ntransition.{index:04}.arguments.sha256={}\n",
            receipt.kind.as_str(),
            receipt.stage.as_str(),
            receipt.process_id,
            receipt.worker,
            receipt.execution_thread.as_deref().unwrap_or("none"),
            receipt.applied_profile,
            receipt.arguments,
        ));
    }
    let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
    let bytes = format!("{body}checksum.sha256={checksum}\n").into_bytes();
    ensure!(
        bytes.len() as u64 <= MAX_WORKER_TRANSITIONS_BYTES,
        "private Codex transition ledger exceeds its size limit"
    );
    Ok(WorkerTransitionLedger {
        bytes,
        app_server,
        exec_initial,
        exec_resumed,
        exec_exited_zero,
        thread_ids: thread_ids.clone(),
        execution_threads: execution_threads.clone(),
        probes: probes.clone(),
    })
}

pub(super) fn require_worker_thread_sets(
    thread_ids: &BTreeSet<String>,
    execution_threads: &BTreeSet<String>,
) -> Result<()> {
    ensure!(
        !thread_ids.is_empty()
            && !execution_threads.is_empty()
            && execution_threads.is_subset(thread_ids),
        "Codex Security completion execution threads differ from its thread set"
    );
    for thread in thread_ids {
        require_safe_identity(thread, "Codex Security completion thread identity")?;
    }
    Ok(())
}

pub(super) fn aggregate_worker_transitions(
    directory: &Path,
    thread_ids: &BTreeSet<String>,
    execution_threads: &BTreeSet<String>,
    source: &SourceIdentity,
    permissions: &WorkerPermissionDigests,
    probes: &WorkerProbeSummary,
) -> Result<WorkerTransitionLedger> {
    let directory = canonical_directory(directory, "private Codex transition directory")?;
    let mut entries = BTreeMap::new();
    for entry in
        std::fs::read_dir(&directory).context("listing private Codex transition directory")?
    {
        let entry = entry.context("reading private Codex transition directory")?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("private Codex transition name is not UTF-8"))?;
        ensure!(
            entries.insert(name, entry.path()).is_none() && entries.len() <= MAX_WORKER_TRANSITIONS,
            "private Codex transition name is duplicated or count exceeds its policy"
        );
    }
    let mut receipts = Vec::with_capacity(entries.len());
    for (name, path) in entries {
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("inspecting private Codex transition {name}"))?;
        ensure!(
            metadata.file_type().is_file()
                && !metadata.file_type().is_symlink()
                && !metadata_is_reparse_point(&metadata)
                && metadata.len() <= MAX_WORKER_TRANSITION_RECEIPT_BYTES,
            "private Codex transition receipt is redirected, special, or oversized"
        );
        let bytes = read(
            &path,
            MAX_WORKER_TRANSITION_RECEIPT_BYTES,
            "private Codex transition receipt",
        )?;
        let receipt = parse_worker_transition_receipt(&bytes)?;
        ensure!(
            name == format!(
                "{}-{}-{}.receipt",
                receipt.kind.as_str(),
                receipt.stage.as_str(),
                receipt.process_id
            ),
            "private Codex transition receipt name differs"
        );
        receipts.push(receipt);
    }
    render_worker_transition_ledger(
        &receipts,
        thread_ids,
        execution_threads,
        source,
        permissions,
        probes,
    )
}

pub(super) fn verify_worker_transition_ledger(
    bytes: &[u8],
    source: &SourceIdentity,
    permissions: &WorkerPermissionDigests,
) -> Result<WorkerTransitionLedger> {
    ensure!(
        bytes.len() as u64 <= MAX_WORKER_TRANSITIONS_BYTES,
        "sealed private Codex transition ledger exceeds its size limit"
    );
    let text = std::str::from_utf8(bytes)
        .context("sealed private Codex transition ledger is not UTF-8")?;
    ensure!(
        text.ends_with('\n') && !text.contains('\r') && !text.contains('\0'),
        "sealed private Codex transition ledger framing differs"
    );
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some(WORKER_TRANSITIONS_SCHEMA),
        "sealed private Codex transition ledger schema differs"
    );
    let read_field = |lines: &mut std::str::Lines<'_>, expected: &str| -> Result<String> {
        let line = lines
            .next()
            .with_context(|| format!("sealed private Codex transition ledger has no {expected}"))?;
        let (key, value) = line
            .split_once('=')
            .context("sealed private Codex transition field has no separator")?;
        ensure!(
            key == expected && !value.is_empty(),
            "sealed private Codex transition field order differs"
        );
        Ok(value.to_string())
    };
    ensure!(
        read_field(&mut lines, "source.commit")? == source.commit
            && read_field(&mut lines, "profile.provider.sha256")? == permissions.provider_profile
            && read_field(&mut lines, "profile.effective.sha256")? == permissions.effective_profile
            && read_field(&mut lines, "profile.effective-state.sha256")?
                == permissions.effective_state,
        "sealed private Codex transition ledger source or profiles differ"
    );
    let probes = WorkerProbeSummary {
        allowed_reads: read_field(&mut lines, "preflight.allowed-reads")?
            .parse::<usize>()
            .context("sealed allowed-read preflight count is not an integer")?,
        denied_reads: read_field(&mut lines, "preflight.denied-reads")?
            .parse::<usize>()
            .context("sealed denied-read preflight count is not an integer")?,
        denied_write_creates: read_field(&mut lines, "preflight.denied-write-creates")?
            .parse::<usize>()
            .context("sealed denied-write-create preflight count is not an integer")?,
        denied_write_mutations: read_field(&mut lines, "preflight.denied-write-mutations")?
            .parse::<usize>()
            .context("sealed denied-write-mutation preflight count is not an integer")?,
        denied_network_listeners: read_field(&mut lines, "preflight.denied-network-listeners")?
            .parse::<usize>()
            .context("sealed denied-network-listener preflight count is not an integer")?,
        denied_network_connections: read_field(&mut lines, "preflight.denied-network-connections")?
            .parse::<usize>()
            .context("sealed denied-network-connection preflight count is not an integer")?,
        denied_descendants: read_field(&mut lines, "preflight.denied-descendants")?
            .parse::<usize>()
            .context("sealed denied-descendant preflight count is not an integer")?,
    };
    probes.require_complete()?;
    let thread_count = read_field(&mut lines, "completion.threads")?
        .parse::<usize>()
        .context("sealed completion thread count is not an integer")?;
    let execution_thread_count = read_field(&mut lines, "completion.execution-threads")?
        .parse::<usize>()
        .context("sealed completion execution-thread count is not an integer")?;
    let app_server = read_field(&mut lines, "transition.app-server-launched")?
        .parse::<usize>()
        .context("sealed app-server transition count is not an integer")?;
    let exec_initial = read_field(&mut lines, "transition.exec-initial-started")?
        .parse::<usize>()
        .context("sealed initial-exec transition count is not an integer")?;
    let exec_resumed = read_field(&mut lines, "transition.exec-resumed-started")?
        .parse::<usize>()
        .context("sealed resumed-exec transition count is not an integer")?;
    let exec_exited_zero = read_field(&mut lines, "transition.exec-exited-zero")?
        .parse::<usize>()
        .context("sealed zero-exit transition count is not an integer")?;
    let count = read_field(&mut lines, "transition.count")?
        .parse::<usize>()
        .context("sealed transition count is not an integer")?;
    ensure!(
        count <= MAX_WORKER_TRANSITIONS
            && thread_count <= MAX_WORKER_TRANSITIONS
            && execution_thread_count <= MAX_WORKER_TRANSITIONS
            && app_server >= 1
            && exec_initial >= 1
            && count == app_server + exec_initial + exec_resumed + exec_exited_zero,
        "sealed private Codex transition counts differ"
    );
    let mut thread_ids = BTreeSet::new();
    for index in 0..thread_count {
        let thread = read_field(&mut lines, &format!("completion.thread.{index:04}"))?;
        require_safe_identity(&thread, "sealed completion thread identity")?;
        ensure!(
            thread_ids.insert(thread),
            "sealed completion thread identity is duplicated"
        );
    }
    let mut execution_threads = BTreeSet::new();
    for index in 0..execution_thread_count {
        let thread = read_field(
            &mut lines,
            &format!("completion.execution-thread.{index:04}"),
        )?;
        require_safe_identity(&thread, "sealed completion execution-thread identity")?;
        ensure!(
            execution_threads.insert(thread),
            "sealed completion execution-thread identity is duplicated"
        );
    }
    require_worker_thread_sets(&thread_ids, &execution_threads)?;
    let mut receipts = Vec::with_capacity(count);
    let mut process_stages = BTreeSet::new();
    for index in 0..count {
        let kind = WorkerTransitionKind::parse(&read_field(
            &mut lines,
            &format!("transition.{index:04}.kind"),
        )?)?;
        let stage = WorkerTransitionStage::parse(&read_field(
            &mut lines,
            &format!("transition.{index:04}.stage"),
        )?)?;
        let process_id = read_field(&mut lines, &format!("transition.{index:04}.wrapper"))?
            .parse::<u32>()
            .context("sealed transition wrapper identity is not an integer")?;
        ensure!(
            process_id != 0 && process_stages.insert((process_id, stage)),
            "sealed transition wrapper stage is zero or duplicated"
        );
        let worker = read_field(&mut lines, &format!("transition.{index:04}.worker.sha256"))?;
        let execution_thread = match read_field(
            &mut lines,
            &format!("transition.{index:04}.execution.thread"),
        )?
        .as_str()
        {
            "none" => None,
            value => Some(value.to_string()),
        };
        let applied_profile = read_field(
            &mut lines,
            &format!("transition.{index:04}.profile.applied.sha256"),
        )?;
        let arguments = read_field(
            &mut lines,
            &format!("transition.{index:04}.arguments.sha256"),
        )?;
        require_sha256(&worker, "sealed transition worker digest")?;
        require_sha256(&applied_profile, "sealed transition applied-profile digest")?;
        require_sha256(&arguments, "sealed transition argument digest")?;
        let receipt = WorkerTransitionReceipt {
            kind,
            stage,
            process_id,
            source_commit: source.commit.clone(),
            provider_profile: permissions.provider_profile.clone(),
            effective_profile: permissions.effective_profile.clone(),
            applied_profile,
            worker,
            execution_thread,
            arguments,
        };
        require_worker_transition_receipt(&receipt, source, permissions)?;
        receipts.push(receipt);
    }
    let checksum = lines
        .next()
        .context("sealed private Codex transition ledger has no checksum")?
        .strip_prefix("checksum.sha256=")
        .context("sealed private Codex transition checksum field differs")?;
    require_sha256(checksum, "sealed private Codex transition checksum")?;
    ensure!(
        lines.next().is_none(),
        "sealed private Codex transition ledger has trailing fields"
    );
    let canonical = render_worker_transition_ledger(
        &receipts,
        &thread_ids,
        &execution_threads,
        source,
        permissions,
        &probes,
    )?;
    ensure!(
        canonical.bytes == bytes,
        "sealed private Codex transition ledger is noncanonical or has an invalid checksum"
    );
    Ok(canonical)
}
