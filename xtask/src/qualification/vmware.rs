use super::*;

#[derive(Debug)]
pub(crate) struct VmwareAttempt {
    paths: Paths,
    kind: Kind,
    record: Record,
    vmrun: Tool,
}

pub(crate) struct VmwareArtifacts<'artifacts> {
    pub(crate) image: &'artifacts Path,
    pub(crate) namespace: &'artifacts Path,
    pub(crate) vmdk: &'artifacts Path,
    pub(crate) namespace_vmdk: &'artifacts Path,
    pub(crate) vmx: &'artifacts Path,
    pub(crate) handoff: &'artifacts Path,
    pub(crate) transcript: &'artifacts Path,
    pub(crate) receipt: &'artifacts Path,
}

#[derive(Debug)]
pub(super) struct CapturedVmwareFile {
    prefix: &'static str,
    name: &'static str,
    source: PathBuf,
    pub(super) captured: PathBuf,
    pub(super) digest: Digest,
}

#[derive(Debug)]
pub(super) struct CapturedVmwareGeneration {
    files: Vec<CapturedVmwareFile>,
}

impl CapturedVmwareGeneration {
    fn read(artifacts: &VmwareArtifacts<'_>, directory: &Path) -> Result<Self> {
        Self::read_with_sizes(artifacts, directory, boot_image_bytes(), namespace_bytes())
    }

    pub(super) fn read_with_sizes(
        artifacts: &VmwareArtifacts<'_>,
        directory: &Path,
        image_bytes: u64,
        namespace_bytes: u64,
    ) -> Result<Self> {
        let mut files = Vec::new();
        for (prefix, name, source) in vmware_artifact_sources(artifacts) {
            require_regular_file(source, "VMware generation capture")?;
            let captured = directory.join(name);
            let digest = capture_vmware_file(source, &captured, name, image_bytes, namespace_bytes)
                .with_context(|| format!("capturing VMware artifact {}", source.display()))?;
            files.push(CapturedVmwareFile {
                prefix,
                name,
                source: source.to_path_buf(),
                captured,
                digest,
            });
        }
        Ok(Self { files })
    }

    pub(super) fn validate_generation(&self, source: &SourceIdentity, kind: Kind) -> Result<()> {
        let receipt = self.file(VMWARE_GENERATION_RECEIPT)?;
        let receipt_text = read_bounded_text(
            &receipt.captured,
            MAX_VMWARE_TEXT_BYTES,
            "captured VMware generation receipt",
        )?;
        let receipt = parse_record(&receipt_text)?;
        validate_generation_record(&receipt, kind)?;
        let generation = receipt.get("generation.id")?;
        let transcript = self.file(VMWARE_TRANSCRIPT)?;
        ensure!(
            transcript.source.file_name().and_then(|name| name.to_str())
                == Some(generation_transcript_name(generation)?.as_str()),
            "captured VMware transcript belongs to another generation"
        );
        let mut digests = self
            .files
            .iter()
            .map(|file| (file.name, file.digest.clone()))
            .collect::<BTreeMap<_, _>>();
        digests.insert(
            VMWARE_VMX,
            verified_generation_vmx_digest(kind, generation, &self.file(VMWARE_VMX)?.captured)?,
        );
        digests.insert(
            VMWARE_HANDOFF,
            verified_dma_handoff_digest(&self.file(VMWARE_HANDOFF)?.captured)?,
        );
        ensure!(
            receipt == generation_record_from_digests(source, kind, generation, &digests)?,
            "captured VMware receipt and machine artifacts are not one generation"
        );
        ensure!(
            transcript.digest.bytes != 0 && transcript.digest.sha256 != empty_digest(),
            "captured VMware transcript has not changed since generation reset"
        );
        Ok(())
    }

    pub(super) fn verify_sources(&self) -> Result<()> {
        for file in &self.files {
            require_regular_file(&file.source, "captured VMware source")?;
            ensure!(
                digest_file(&file.source)? == file.digest,
                "VMware source {} changed after its generation was captured",
                file.source.display()
            );
        }
        Ok(())
    }

    pub(super) fn publish(&self, directory: &Path, record: &mut Record) -> Result<()> {
        for file in &self.files {
            let destination = directory.join(file.name);
            ensure!(
                destination == file.captured && digest_file(&destination)? == file.digest,
                "captured VMware artifact changed during publication"
            );
            add_artifact_digest(record, file.prefix, file.name, &file.digest)?;
        }
        Ok(())
    }

    pub(super) fn file(&self, name: &str) -> Result<&CapturedVmwareFile> {
        self.files
            .iter()
            .find(|file| file.name == name)
            .with_context(|| format!("captured VMware generation has no `{name}`"))
    }
}

fn vmware_artifact_sources<'artifacts>(
    artifacts: &VmwareArtifacts<'artifacts>,
) -> [(&'static str, &'static str, &'artifacts Path); 8] {
    [
        (
            "artifact.generation",
            VMWARE_GENERATION_RECEIPT,
            artifacts.receipt,
        ),
        ("artifact.image", VMWARE_IMAGE, artifacts.image),
        ("artifact.namespace", VMWARE_NAMESPACE, artifacts.namespace),
        ("artifact.vmdk", VMWARE_VMDK, artifacts.vmdk),
        (
            "artifact.namespace-vmdk",
            VMWARE_NAMESPACE_VMDK,
            artifacts.namespace_vmdk,
        ),
        ("artifact.vmx", VMWARE_VMX, artifacts.vmx),
        ("artifact.handoff", VMWARE_HANDOFF, artifacts.handoff),
        (
            "artifact.transcript",
            VMWARE_TRANSCRIPT,
            artifacts.transcript,
        ),
    ]
}

pub(crate) fn begin_vmware(paths: &Paths, kernel: Kernel) -> Result<VmwareAttempt> {
    let kind = Kind::vmware(kernel);
    invalidate(paths, kind)?;
    let vmrun = vmrun(paths.root())?
        .context("vmrun is unavailable; VMware execution cannot be qualified")?;
    let mut record = base_record(paths, kind)?;
    add_tool(&mut record, "input.vmrun", &vmrun)?;
    Ok(VmwareAttempt {
        paths: paths.clone(),
        kind,
        record,
        vmrun,
    })
}

impl VmwareAttempt {
    pub(crate) fn pass(mut self, artifacts: &VmwareArtifacts<'_>) -> Result<()> {
        let (current_tool, current) = current_vmware_record(&self.paths, self.kind)?;
        ensure!(
            current_tool.path == self.vmrun.path && current == self.record,
            "source or vmrun changed during VMware transcript qualification"
        );
        let generated_directory = require_vmware_layout(&self.paths, self.kind, artifacts)?;
        ensure!(
            artifacts.transcript
                == generated_transcript_path(artifacts.receipt, kernel_for(self.kind)?)?,
            "VMware transcript belongs to another generation"
        );
        verify_generation_directory(
            &self.paths,
            self.kind,
            &generated_directory,
            artifacts.transcript,
        )?;
        ensure!(
            digest_file(artifacts.image)? == digest_file(&self.paths.image())?,
            "VMware boot image differs from the current generated input"
        );
        ensure!(
            digest_file(artifacts.namespace)?
                == digest_file(&crate::qemu::nvme_path(self.paths.root()))?,
            "VMware storage namespace differs from the current generated input"
        );
        let observation = observe_running_vm(self.paths.root(), &self.vmrun, artifacts.vmx)?;
        let qualification_directory = kind_directory(&self.paths, self.kind)?;
        let captured = CapturedVmwareGeneration::read(artifacts, &qualification_directory)?;
        let source = source_identity(self.paths.root())?;
        require_source(&self.record, &source)?;
        captured.validate_generation(&source, self.kind)?;
        crate::vmware::verify_transcript(
            self.paths.root(),
            kernel_for(self.kind)?,
            &read_bounded_file(
                &captured.file(VMWARE_TRANSCRIPT)?.captured,
                MAX_VMWARE_TRANSCRIPT_BYTES,
                "captured VMware transcript",
            )?,
        )?;
        captured.publish(&qualification_directory, &mut self.record)?;
        let observer = qualification_directory.join(VMWARE_OBSERVER);
        write_new_file(&observer, format!("{observation}\n").as_bytes())?;
        add_artifact(
            &mut self.record,
            "artifact.observer",
            VMWARE_OBSERVER,
            &observer,
        )?;
        self.record
            .insert("execution.vmrun.command", "-T ws list")?;
        self.record.insert("execution.vmrun.running", "1")?;
        verify_generation_directory(
            &self.paths,
            self.kind,
            &qualification_directory,
            &qualification_directory.join(VMWARE_TRANSCRIPT),
        )?;
        ensure!(
            digest_file(&qualification_directory.join(VMWARE_IMAGE))?
                == digest_file(&self.paths.image())?,
            "copied VMware boot image differs from the current generated input"
        );
        ensure!(
            digest_file(&qualification_directory.join(VMWARE_NAMESPACE))?
                == digest_file(&crate::qemu::nvme_path(self.paths.root()))?,
            "copied VMware namespace differs from the current generated input"
        );
        let transcript = read_bounded_file(
            &qualification_directory.join(VMWARE_TRANSCRIPT),
            MAX_VMWARE_TRANSCRIPT_BYTES,
            "copied VMware transcript",
        )?;
        crate::vmware::verify_transcript(self.paths.root(), kernel_for(self.kind)?, &transcript)?;
        let (current_tool, current) = current_vmware_record(&self.paths, self.kind)?;
        ensure!(
            current_tool.path == self.vmrun.path,
            "the resolved vmrun changed before VMware qualification committed"
        );
        require_record_inputs(&self.record, &current)?;
        captured.verify_sources()?;
        ensure!(
            artifacts.transcript
                == generated_transcript_path(artifacts.receipt, kernel_for(self.kind)?)?,
            "VMware transcript generation changed before qualification committed"
        );
        verify_generation_directory(
            &self.paths,
            self.kind,
            &generated_directory,
            artifacts.transcript,
        )?;
        ensure!(
            captured.file(VMWARE_IMAGE)?.digest == digest_file(&self.paths.image())?,
            "current boot image changed after the VMware generation was captured"
        );
        ensure!(
            captured.file(VMWARE_NAMESPACE)?.digest
                == digest_file(&crate::qemu::nvme_path(self.paths.root()))?,
            "current namespace changed after the VMware generation was captured"
        );
        captured.verify_sources()?;
        self.record.insert("result", PASS)?;
        commit_record(&self.paths, self.kind, &self.record)
    }
}

pub(crate) fn invalidate_vmware_generation(receipt: &Path) -> Result<()> {
    ensure!(
        receipt.file_name().and_then(|name| name.to_str()) == Some(VMWARE_GENERATION_RECEIPT),
        "refusing to invalidate an unexpected VMware generation receipt"
    );
    let parent = receipt
        .parent()
        .context("VMware generation receipt has no parent")?;
    if let Ok(metadata) = std::fs::symlink_metadata(parent) {
        ensure!(
            metadata.file_type().is_dir() && !metadata.file_type().is_symlink(),
            "VMware directory is not a real directory"
        );
    }
    for path in [receipt.to_path_buf(), parent.join(VMWARE_GENERATION_TEMP)] {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() => {
                bail!("VMware generation receipt is unexpectedly a directory")
            }
            Ok(_) => std::fs::remove_file(&path)
                .with_context(|| format!("invalidating {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", path.display()));
            }
        }
    }
    Ok(())
}

pub(crate) fn fresh_vmware_transcript_name() -> Result<String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("the host clock predates the Unix epoch")?;
    let sequence = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    let material = format!("{}:{}:{sequence}", elapsed.as_nanos(), std::process::id());
    let generation = sha256::hex(&sha256::digest(material.as_bytes()));
    generation_transcript_name(&generation)
}

pub(crate) fn generated_transcript_path(receipt: &Path, kernel: Kernel) -> Result<PathBuf> {
    let record = read_generation_record(receipt, Kind::vmware(kernel))?;
    let name = record.get("reset.transcript.path")?;
    let mut components = Path::new(name).components();
    ensure!(
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none(),
        "VMware generation transcript path is not one file name"
    );
    let directory = receipt
        .parent()
        .context("VMware generation receipt has no parent")?;
    Ok(directory.join(name))
}

pub(crate) fn commit_vmware_generation(
    paths: &Paths,
    kernel: Kernel,
    artifacts: &VmwareArtifacts<'_>,
) -> Result<()> {
    let directory = require_vmware_layout(paths, Kind::vmware(kernel), artifacts)?;
    let generation = transcript_generation(artifacts.transcript)?;
    require_generation_inventory(&directory, artifacts.transcript, false)?;
    let reset = digest_file(artifacts.transcript)?;
    require_empty_reset(&reset)?;
    let source = source_identity(paths.root())?;
    let record = generation_record(&source, Kind::vmware(kernel), &directory, &generation)?;
    let temporary = directory.join(VMWARE_GENERATION_TEMP);
    write_new_file(&temporary, record.render().as_bytes())?;
    require_generation_inventory(&directory, artifacts.transcript, true)?;
    ensure!(
        read_generation_record(&temporary, Kind::vmware(kernel))? == record,
        "VMware generation receipt changed before commit"
    );
    ensure!(
        generation_record(
            &source_identity(paths.root())?,
            Kind::vmware(kernel),
            &directory,
            &generation,
        )? == record
            && digest_file(artifacts.transcript)? == reset,
        "source or VMware generation artifacts changed before receipt commit"
    );
    std::fs::rename(&temporary, artifacts.receipt)
        .with_context(|| format!("committing {}", artifacts.receipt.display()))
}

pub(crate) fn invalidate_vmware(paths: &Paths, kernel: Kernel) -> Result<()> {
    invalidate(paths, Kind::vmware(kernel))
}

pub(super) fn verify_vmware_observer(
    paths: &Paths,
    record: &Record,
    kind: Kind,
    root: &Path,
) -> Result<()> {
    ensure!(
        record.get("artifact.observer.path")? == VMWARE_OBSERVER,
        "artifact.observer names an unexpected qualification path"
    );
    let observer = root.join(kind.slug()).join(VMWARE_OBSERVER);
    require_regular_file(&observer, "artifact.observer")?;
    require_artifact_size(kind, VMWARE_OBSERVER, &observer)?;
    let digest = digest_file(&observer)?;
    ensure!(
        record.get("artifact.observer.bytes")? == digest.bytes.to_string(),
        "artifact.observer length differs from its qualification record"
    );
    ensure!(
        record.get("artifact.observer.sha256")? == digest.sha256,
        "artifact.observer digest differs from its qualification record"
    );
    let text = read_bounded_text(&observer, MAX_VMWARE_TEXT_BYTES, "VMware observer")?;
    let body = text
        .strip_suffix('\n')
        .context("VMware observer has no terminal LF")?;
    ensure!(
        !body.ends_with('\n'),
        "VMware observer is not canonical LF text"
    );
    let observed = parse_vmrun_list(body)?;
    let selector = match kind {
        Kind::VmwareSelfTest => "self-test",
        Kind::VmwareShipped => "shipped",
        Kind::Advisory
        | Kind::Mutation
        | Kind::CargoMutantsGrade
        | Kind::CodexSecurityDeep
        | Kind::Model => {
            bail!("qualification kind has no VMware selector")
        }
    };
    let expected = paths.vm_dir().join(selector).join(VMWARE_VMX);
    require_observed_vmx(&observed, &expected)
}

fn require_vmware_layout(
    paths: &Paths,
    kind: Kind,
    artifacts: &VmwareArtifacts<'_>,
) -> Result<PathBuf> {
    let directory = artifacts
        .receipt
        .parent()
        .context("VMware generation receipt has no parent")?;
    require_exact_vmware_directory(paths, kind, directory)?;
    for (path, name) in [
        (artifacts.receipt, VMWARE_GENERATION_RECEIPT),
        (artifacts.image, VMWARE_IMAGE),
        (artifacts.namespace, VMWARE_NAMESPACE),
        (artifacts.vmdk, VMWARE_VMDK),
        (artifacts.namespace_vmdk, VMWARE_NAMESPACE_VMDK),
        (artifacts.vmx, VMWARE_VMX),
        (artifacts.handoff, VMWARE_HANDOFF),
    ] {
        ensure!(
            path.parent() == Some(directory)
                && path.file_name().and_then(|value| value.to_str()) == Some(name),
            "VMware artifact does not have the generated `{name}` layout"
        );
    }
    ensure!(
        artifacts.transcript.parent() == Some(directory),
        "VMware transcript is outside its generated machine"
    );
    transcript_generation(artifacts.transcript)?;
    Ok(directory.to_path_buf())
}

pub(super) fn require_exact_vmware_directory(
    paths: &Paths,
    kind: Kind,
    directory: &Path,
) -> Result<()> {
    let selector = match kind {
        Kind::VmwareSelfTest => "self-test",
        Kind::VmwareShipped => "shipped",
        Kind::Advisory
        | Kind::Mutation
        | Kind::CargoMutantsGrade
        | Kind::CodexSecurityDeep
        | Kind::Model => {
            bail!("qualification kind has no VMware directory")
        }
    };
    let root = paths.vm_dir();
    ensure!(
        root == paths.root().join("vm") && root.parent() == Some(paths.root()),
        "VMware root escaped the workspace"
    );
    let expected = root.join(selector);
    ensure!(
        directory == expected && directory.parent() == Some(root.as_path()),
        "VMware selector directory is not the generated `{selector}` directory"
    );
    require_real_directory(&root, "VMware root")?;
    require_real_directory(directory, "VMware selector directory")?;
    let canonical_workspace = std::fs::canonicalize(paths.root())
        .with_context(|| format!("canonicalizing {}", paths.root().display()))?;
    let canonical_root = std::fs::canonicalize(&root)
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let canonical_directory = std::fs::canonicalize(directory)
        .with_context(|| format!("canonicalizing {}", directory.display()))?;
    ensure!(
        canonical_root.parent() == Some(canonical_workspace.as_path()),
        "VMware root escaped the canonical workspace"
    );
    ensure!(
        canonical_directory.parent() == Some(canonical_root.as_path()),
        "VMware selector directory escaped its canonical root"
    );
    Ok(())
}

fn verify_generation_directory(
    paths: &Paths,
    kind: Kind,
    directory: &Path,
    transcript: &Path,
) -> Result<()> {
    let source = source_identity(paths.root())?;
    verify_generation_with_source(&source, kind, directory, transcript)
}

pub(super) fn verify_generation_with_source(
    source: &SourceIdentity,
    kind: Kind,
    directory: &Path,
    transcript: &Path,
) -> Result<()> {
    let receipt = read_generation_record(&directory.join(VMWARE_GENERATION_RECEIPT), kind)?;
    let generation = receipt.get("generation.id")?;
    ensure!(
        receipt == generation_record(source, kind, directory, generation)?,
        "VMware generation receipt differs from the current source or machine artifacts"
    );
    require_regular_file(transcript, "VMware transcript")?;
    let transcript = digest_file(transcript)?;
    ensure!(
        transcript.bytes != 0 && transcript.sha256 != empty_digest(),
        "VMware transcript has not changed since generation reset"
    );
    Ok(())
}

pub(super) fn generation_record(
    source: &SourceIdentity,
    kind: Kind,
    directory: &Path,
    generation: &str,
) -> Result<Record> {
    let mut digests = BTreeMap::new();
    for name in [
        VMWARE_IMAGE,
        VMWARE_NAMESPACE,
        VMWARE_VMDK,
        VMWARE_NAMESPACE_VMDK,
        VMWARE_VMX,
        VMWARE_HANDOFF,
    ] {
        let path = directory.join(name);
        require_regular_file(&path, "VMware generation artifact")?;
        digests.insert(name, digest_file(&path)?);
    }
    digests.insert(
        VMWARE_VMX,
        verified_generation_vmx_digest(kind, generation, &directory.join(VMWARE_VMX))?,
    );
    digests.insert(
        VMWARE_HANDOFF,
        verified_dma_handoff_digest(&directory.join(VMWARE_HANDOFF))?,
    );
    generation_record_from_digests(source, kind, generation, &digests)
}

fn verified_generation_vmx_digest(kind: Kind, generation: &str, path: &Path) -> Result<Digest> {
    let generated =
        crate::vmware::generation_vmx(kernel_for(kind)?, &generation_transcript_name(generation)?)?;
    let runtime = read_bounded_file(path, MAX_VMWARE_TEXT_BYTES, "runtime VMX")?;
    crate::vmware::verify_runtime_vmx(generated.as_bytes(), &runtime)?;
    Ok(digest_bytes(generated.as_bytes()))
}

pub(super) fn verified_dma_handoff_digest(path: &Path) -> Result<Digest> {
    let bytes = read_bounded_file(
        path,
        acpi_tables::DmaHandoff::LEN as u64,
        "DMA handoff authority",
    )?;
    let expected = crate::platform_acpi::dma_handoff_table();
    ensure!(
        bytes == expected,
        "VMware DMA handoff authority differs from the source-defined contract"
    );
    acpi_tables::DmaHandoff::new(&bytes).context("parsing VMware DMA handoff authority")?;
    Ok(digest_bytes(&bytes))
}

fn generation_record_from_digests(
    source: &SourceIdentity,
    kind: Kind,
    generation: &str,
    digests: &BTreeMap<&str, Digest>,
) -> Result<Record> {
    ensure!(is_digest(generation), "VMware generation ID is malformed");
    let mut record = Record::default();
    record.insert("generation.id", generation)?;
    record.insert("kind", VMWARE_GENERATION_KIND)?;
    record.insert("input.kernel", kind.slug())?;
    record.insert("source.commit", &source.commit)?;
    record.insert("source.tree", &source.tree)?;
    for (prefix, name) in [
        ("artifact.image", VMWARE_IMAGE),
        ("artifact.namespace", VMWARE_NAMESPACE),
        ("artifact.vmdk", VMWARE_VMDK),
        ("artifact.namespace-vmdk", VMWARE_NAMESPACE_VMDK),
        ("artifact.vmx", VMWARE_VMX),
        ("artifact.handoff", VMWARE_HANDOFF),
    ] {
        let digest = digests
            .get(name)
            .with_context(|| format!("VMware generation has no `{name}` digest"))?;
        add_artifact_digest(&mut record, prefix, name, digest)?;
    }
    record.insert("reset.inventory.files", INITIAL_VMWARE_FILES.to_string())?;
    record.insert(
        "reset.transcript.path",
        generation_transcript_name(generation)?,
    )?;
    record.insert("reset.transcript.bytes", "0")?;
    record.insert("reset.transcript.sha256", empty_digest())?;
    record.insert("result", RESET)?;
    Ok(record)
}

pub(super) fn read_generation_record(path: &Path, kind: Kind) -> Result<Record> {
    require_regular_file(path, "VMware generation receipt")?;
    let text = read_bounded_text(path, MAX_VMWARE_TEXT_BYTES, "VMware generation receipt")?;
    let record = parse_record(&text)?;
    validate_generation_record(&record, kind)?;
    Ok(record)
}

fn validate_generation_record(record: &Record, kind: Kind) -> Result<()> {
    let mut expected = BTreeSet::from([
        "input.kernel".to_string(),
        "generation.id".to_string(),
        "kind".to_string(),
        "reset.inventory.files".to_string(),
        "reset.transcript.bytes".to_string(),
        "reset.transcript.path".to_string(),
        "reset.transcript.sha256".to_string(),
        "result".to_string(),
        "source.commit".to_string(),
        "source.tree".to_string(),
    ]);
    for prefix in [
        "artifact.image",
        "artifact.namespace",
        "artifact.vmdk",
        "artifact.namespace-vmdk",
        "artifact.vmx",
        "artifact.handoff",
    ] {
        add_artifact_keys(&mut expected, prefix);
    }
    let actual: BTreeSet<String> = record.fields.keys().cloned().collect();
    ensure!(
        actual == expected,
        "VMware generation receipt schema differs"
    );
    ensure!(
        record.get("kind")? == VMWARE_GENERATION_KIND && record.get("input.kernel")? == kind.slug(),
        "VMware generation receipt names another kernel selector"
    );
    ensure!(
        record.get("result")? == RESET,
        "VMware generation was not reset"
    );
    ensure!(
        is_git_id(record.get("source.commit")?) && is_git_id(record.get("source.tree")?),
        "VMware generation source identity is malformed"
    );
    ensure!(
        is_digest(record.get("generation.id")?),
        "VMware generation ID is malformed"
    );
    ensure!(
        record.get("reset.transcript.path")?
            == generation_transcript_name(record.get("generation.id")?)?
            && record.get("reset.transcript.bytes")? == "0"
            && record.get("reset.transcript.sha256")? == empty_digest(),
        "VMware generation receipt does not prove an empty transcript reset"
    );
    ensure!(
        record.get("reset.inventory.files")? == INITIAL_VMWARE_FILES.to_string(),
        "VMware generation receipt does not prove the exact initial inventory"
    );
    for (key, value) in &record.fields {
        if key.ends_with(".sha256") {
            ensure!(
                is_digest(value),
                "VMware generation digest `{key}` is malformed"
            );
        }
        if key.starts_with("artifact.") && key.ends_with(".bytes") {
            ensure!(
                <u64 as core::str::FromStr>::from_str(value).is_ok_and(|bytes| bytes != 0),
                "VMware generation length `{key}` is invalid"
            );
        }
    }
    Ok(())
}

fn empty_digest() -> String {
    sha256::hex(&sha256::digest(&[]))
}

pub(super) fn require_empty_reset(reset: &Digest) -> Result<()> {
    ensure!(
        reset.bytes == 0 && reset.sha256 == empty_digest(),
        "VMware transcript was not empty at generation reset"
    );
    Ok(())
}

pub(super) fn generation_transcript_name(generation: &str) -> Result<String> {
    ensure!(is_digest(generation), "VMware generation ID is malformed");
    Ok(format!("{SERIAL_PREFIX}{generation}{SERIAL_SUFFIX}"))
}

fn transcript_generation(path: &Path) -> Result<String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("VMware transcript name is not UTF-8")?;
    let generation = name
        .strip_prefix(SERIAL_PREFIX)
        .and_then(|name| name.strip_suffix(SERIAL_SUFFIX))
        .context("VMware transcript does not name one generation")?;
    ensure!(is_digest(generation), "VMware generation ID is malformed");
    Ok(generation.to_string())
}

pub(super) fn require_generation_inventory(
    directory: &Path,
    transcript: &Path,
    temporary_receipt: bool,
) -> Result<()> {
    let transcript = transcript
        .file_name()
        .and_then(|name| name.to_str())
        .context("VMware transcript name is not UTF-8")?;
    let mut expected = BTreeSet::from([
        VMWARE_IMAGE.to_string(),
        VMWARE_NAMESPACE.to_string(),
        VMWARE_VMDK.to_string(),
        VMWARE_NAMESPACE_VMDK.to_string(),
        VMWARE_VMX.to_string(),
        VMWARE_HANDOFF.to_string(),
        transcript.to_string(),
    ]);
    if temporary_receipt {
        expected.insert(VMWARE_GENERATION_TEMP.to_string());
    }
    let mut actual = BTreeSet::new();
    for entry in std::fs::read_dir(directory)
        .with_context(|| format!("reading VMware directory {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("VMware directory has a non-UTF-8 entry"))?;
        ensure!(
            entry
                .file_type()
                .with_context(|| format!("inspecting {}", entry.path().display()))?
                .is_file(),
            "VMware generation entry is not a regular file: {}",
            entry.path().display()
        );
        actual.insert(name);
    }
    ensure!(
        actual == expected,
        "VMware generation inventory differs: missing {:?}; unexpected {:?}",
        expected.difference(&actual).collect::<Vec<_>>(),
        actual.difference(&expected).collect::<Vec<_>>()
    );
    Ok(())
}

pub(super) fn vmware_record_for_source(
    paths: &Paths,
    kind: Kind,
    source: &SourceIdentity,
) -> Result<Record> {
    let vmrun =
        vmrun(paths.root())?.context("vmrun is unavailable; VMware evidence cannot verify")?;
    let mut record = base_record_for_source(kind, source)?;
    add_tool(&mut record, "input.vmrun", &vmrun)?;
    Ok(record)
}

fn current_vmware_record(paths: &Paths, kind: Kind) -> Result<(Tool, Record)> {
    let vmrun =
        vmrun(paths.root())?.context("vmrun is unavailable; VMware evidence cannot verify")?;
    let mut record = base_record(paths, kind)?;
    add_tool(&mut record, "input.vmrun", &vmrun)?;
    Ok((vmrun, record))
}

pub(super) fn capture_vmware_file(
    source: &Path,
    destination: &Path,
    name: &str,
    image_bytes: u64,
    namespace_bytes: u64,
) -> Result<Digest> {
    capture_vmware_file_with(
        source,
        destination,
        name,
        image_bytes,
        namespace_bytes,
        || Ok(()),
    )
}

pub(super) fn capture_vmware_file_with<F>(
    source: &Path,
    destination: &Path,
    name: &str,
    image_bytes: u64,
    namespace_bytes: u64,
    after_measurement: F,
) -> Result<Digest>
where
    F: FnOnce() -> Result<()>,
{
    let (maximum, exact, label) = match name {
        VMWARE_IMAGE => (image_bytes, true, "VMware boot image"),
        VMWARE_NAMESPACE => (namespace_bytes, true, "VMware storage namespace"),
        VMWARE_HANDOFF => (
            acpi_tables::DmaHandoff::LEN as u64,
            true,
            "VMware DMA handoff authority",
        ),
        VMWARE_TRANSCRIPT => (MAX_VMWARE_TRANSCRIPT_BYTES, false, "VMware transcript"),
        VMWARE_GENERATION_RECEIPT
        | VMWARE_VMDK
        | VMWARE_NAMESPACE_VMDK
        | VMWARE_VMX
        | VMWARE_OBSERVER => (MAX_VMWARE_TEXT_BYTES, false, "VMware text artifact"),
        _ => bail!("unregistered VMware artifact `{name}`"),
    };
    let mut input = crate::mutate::workspace::open_real_file(source, label)?;
    let initial = input
        .metadata()
        .with_context(|| format!("measuring {label} {}", source.display()))?
        .len();
    if exact {
        ensure!(
            initial == maximum,
            "{label} is {initial} bytes, expected exactly {maximum}"
        );
    } else {
        ensure!(
            initial <= maximum,
            "{label} is {initial} bytes, exceeding the {maximum}-byte capture limit"
        );
    }
    after_measurement()?;
    let mut options = OpenOptions::new();
    OpenOptions::create_new(&mut options, true);
    OpenOptions::write(&mut options, true);
    let mut output = OpenOptions::open(&options, destination)
        .with_context(|| format!("creating captured {label} {}", destination.display()))?;
    let copied = std::io::copy(
        &mut (&mut input).take(
            maximum
                .checked_add(1)
                .context("VMware capture limit overflowed")?,
        ),
        &mut output,
    )
    .with_context(|| {
        format!(
            "streaming {label} {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    ensure!(
        copied <= maximum,
        "{label} grew beyond its {maximum}-byte capture limit while being copied"
    );
    ensure!(
        !exact || copied == maximum,
        "{label} changed outside its exact {maximum}-byte capture extent"
    );
    output
        .sync_all()
        .with_context(|| format!("syncing captured {label} {}", destination.display()))?;
    drop(output);
    require_regular_file(destination, "captured VMware artifact")?;
    ensure!(
        input
            .metadata()
            .with_context(|| format!("remeasuring {label} {}", source.display()))?
            .len()
            == copied,
        "{label} changed length while being captured"
    );
    let digest = digest_file(destination)?;
    ensure!(
        digest.bytes == copied,
        "captured {label} changed before its digest was bound"
    );
    Ok(digest)
}

/// The VMware Workstation control executable, if it can be resolved.
pub(crate) fn vmrun(root: &Path) -> Result<Option<Tool>> {
    let name = executable_name("vmrun");
    let path = if cfg!(windows) {
        let default = PathBuf::from(WINDOWS_VMWARE).join(&name);
        default
            .is_file()
            .then_some(default)
            .or_else(|| find_on_path("vmrun"))
    } else {
        find_on_path("vmrun")
    };
    let Some(path) = path else {
        return Ok(None);
    };
    probe_vmrun(root, path).map(Some)
}

fn probe_vmrun(root: &Path, path: PathBuf) -> Result<Tool> {
    let report = output(root, &path, &[], ExitPolicy::Any)?;
    let version = vmrun_version(&report)?;
    Ok(Tool { path, version })
}

fn observe_running_vm(root: &Path, tool: &Tool, expected_vmx: &Path) -> Result<String> {
    observe_running_vm_with(tool, expected_vmx, |program, args, policy| {
        output(root, program, args, policy)
    })
}

pub(super) fn observe_running_vm_with<F>(
    tool: &Tool,
    expected_vmx: &Path,
    mut run: F,
) -> Result<String>
where
    F: FnMut(&Path, &[&str], ExitPolicy) -> Result<String>,
{
    let before = run(&tool.path, &[], ExitPolicy::Any)?;
    ensure!(
        vmrun_version(&before)? == tool.version,
        "vmrun changed before its execution observation"
    );
    let report = run(&tool.path, VMRUN_LIST_ARGS, ExitPolicy::Success)?;
    let observed = parse_vmrun_list(&report)?;
    require_observed_vmx(&observed, expected_vmx)?;
    let after = run(&tool.path, &[], ExitPolicy::Any)?;
    ensure!(
        vmrun_version(&after)? == tool.version,
        "vmrun changed during its execution observation"
    );
    Ok(report)
}

pub(super) fn require_observed_vmx(observed: &Path, expected_vmx: &Path) -> Result<()> {
    ensure!(observed.is_absolute(), "vmrun reported a relative VMX path");
    require_regular_file(expected_vmx, "generated VMware configuration")?;
    let observed = std::fs::canonicalize(observed)
        .with_context(|| format!("canonicalizing running VMX {}", observed.display()))?;
    let expected = std::fs::canonicalize(expected_vmx)
        .with_context(|| format!("canonicalizing generated VMX {}", expected_vmx.display()))?;
    ensure!(
        observed == expected,
        "vmrun reported another machine: {}",
        observed.display()
    );
    Ok(())
}

pub(super) fn parse_vmrun_list(report: &str) -> Result<PathBuf> {
    ensure!(
        !report.contains(['\0', '\r']),
        "vmrun list contains a forbidden control byte"
    );
    let mut lines = report.lines();
    let count = lines
        .next()
        .and_then(|line| line.strip_prefix(VMRUN_LIST_HEADER))
        .context("vmrun list has no supported count header")?;
    let parsed = <usize as core::str::FromStr>::from_str(count)
        .context("vmrun list count is not a number")?;
    ensure!(
        parsed.to_string() == count,
        "vmrun list count is not canonical"
    );
    let paths: Vec<&str> = lines.collect();
    ensure!(
        parsed == 1 && paths.len() == parsed,
        "vmrun reported {parsed} running VMs and {} VMX paths",
        paths.len()
    );
    ensure!(!paths[0].is_empty(), "vmrun reported an empty VMX path");
    Ok(PathBuf::from(paths[0]))
}

pub(super) fn vmrun_version(report: &str) -> Result<String> {
    let version = report
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .context("vmrun printed no version")?;
    ensure!(
        version.starts_with("vmrun version "),
        "vmrun's first line is not a version: {version}"
    );
    Ok(version.to_string())
}
