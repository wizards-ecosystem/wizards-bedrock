use super::*;

pub(crate) fn recorded_source(paths: &Paths) -> Result<SourceIdentity> {
    let root = qualification_root(paths);
    require_real_root(&root, "qualification root")?;
    let mutation = read_record(
        &checked_kind_directory(&root, Kind::Mutation)?.join(RECORD),
        Kind::Mutation,
    )?;
    let source = SourceIdentity {
        commit: mutation.get("source.commit")?.to_string(),
        tree: mutation.get("source.tree")?.to_string(),
    };
    let campaigns = CampaignStates::observe(&root)?;
    if !campaigns.deferred(Kind::CodexSecurityDeep) {
        require_authenticated_deep_qualification(paths, &root, &source)?;
    }
    recorded_source_after_authority(paths, &campaigns)
}

fn recorded_source_after_authority(
    paths: &Paths,
    campaigns: &CampaignStates,
) -> Result<SourceIdentity> {
    let root = qualification_root(paths);
    require_real_root(&root, "qualification root")?;
    let mut source = None;
    for kind in [
        Kind::Advisory,
        Kind::Mutation,
        Kind::CargoMutantsGrade,
        Kind::CodexSecurityDeep,
        Kind::Model,
        Kind::VmwareSelfTest,
        Kind::VmwareShipped,
    ] {
        if campaigns.deferred(kind) {
            continue;
        }
        let record = read_record(&checked_kind_directory(&root, kind)?.join(RECORD), kind)?;
        let observed = SourceIdentity {
            commit: record.get("source.commit")?.to_string(),
            tree: record.get("source.tree")?.to_string(),
        };
        if let Some(expected) = &source {
            ensure!(
                *expected == observed,
                "qualification records name different source identities"
            );
        } else {
            source = Some(observed);
        }
    }
    source.context("qualification bundle has no source identity")
}

pub(crate) fn snapshot(
    paths: &Paths,
    registry: &str,
    mutation_count: usize,
    source: &SourceIdentity,
    require_current_mutation: bool,
) -> Result<Snapshot> {
    let root = qualification_root(paths);
    let mutation = if require_current_mutation {
        crate::mutate::workspace::current_pass(paths, registry, mutation_count)?
    } else {
        let directory = checked_kind_directory(&root, Kind::Mutation)?;
        let record = read_record(&directory.join(RECORD), Kind::Mutation)?;
        require_source(&record, source)?;
        crate::mutate::workspace::recorded_pass(
            paths,
            &source.commit,
            registry,
            record.get("input.source.sha256")?,
            record.get("input.index.sha256")?,
            mutation_count,
            record.get("artifact.report.sha256")?,
        )?
    };
    let mutation = RetainedMutationPass::new(mutation);
    let campaigns = CampaignStates::observe(&root)?;
    verify_records(
        paths,
        &root,
        source,
        registry,
        mutation_count,
        Some(mutation.evidence()),
        &campaigns,
    )?;
    let bundle = campaigns.bundle_files();
    let mut digests = Vec::with_capacity(bundle.len());
    for spec in bundle {
        let path = root.join(bundle_tail(spec)?);
        require_bundle_file_size(spec, &path)?;
        let (maximum, _) = bundle_file_size_policy(spec)?;
        digests.push(digest_file_bounded(&path, maximum)?);
    }
    Ok(Snapshot {
        root,
        digests,
        campaigns,
        _mutation: mutation,
    })
}

pub(crate) fn verify_bundle(
    paths: &Paths,
    evidence_root: &Path,
    source: &SourceIdentity,
    registry: &str,
    mutation_count: usize,
    campaigns: &CampaignStates,
) -> Result<()> {
    verify_records(
        paths,
        &evidence_root.join("qualification"),
        source,
        registry,
        mutation_count,
        None,
        campaigns,
    )
}

fn verify_records(
    paths: &Paths,
    root: &Path,
    source: &SourceIdentity,
    registry: &str,
    mutation_count: usize,
    mutation: Option<&PassEvidence>,
    campaigns: &CampaignStates,
) -> Result<()> {
    if !campaigns.deferred(Kind::CodexSecurityDeep) {
        require_authenticated_deep_qualification(paths, root, source)?;
    }
    verify_records_after_authority(
        paths,
        root,
        source,
        registry,
        mutation_count,
        mutation,
        campaigns,
    )
}

pub(super) fn verify_records_after_authority(
    paths: &Paths,
    root: &Path,
    source: &SourceIdentity,
    registry: &str,
    mutation_count: usize,
    mutation: Option<&PassEvidence>,
    campaigns: &CampaignStates,
) -> Result<()> {
    require_real_root(root, "qualification root")?;
    for kind in DEFERRABLE {
        if campaigns.deferred(kind) {
            require_deferred(root, kind)?;
        }
    }
    verify_advisory_qualification(paths, root, source)?;
    for kind in [
        Kind::Mutation,
        Kind::CargoMutantsGrade,
        Kind::CodexSecurityDeep,
        Kind::Model,
        Kind::VmwareSelfTest,
        Kind::VmwareShipped,
    ] {
        if campaigns.deferred(kind) {
            continue;
        }
        let directory = checked_kind_directory(root, kind)?;
        require_contained_real_directory(root, &directory, "qualification kind directory")?;
        let record = read_record(&directory.join(RECORD), kind)?;
        require_source(&record, source)?;
        verify_support(paths, &record, kind, root)?;
        match kind {
            Kind::Mutation => {
                require_mutation_inputs(&record, registry, mutation_count, mutation)?;
                crate::mutate::workspace::verify_recorded_pass(
                    &directory.join(MUTATION_REPORT),
                    record.get("source.commit")?,
                    registry,
                    record.get("input.source.sha256")?,
                    record.get("input.index.sha256")?,
                    mutation_count,
                )?;
            }
            Kind::CargoMutantsGrade => {
                require_external_fields(
                    &record,
                    &crate::mutants_grade::verify_sealed(paths, &directory, source)?,
                )?;
            }
            Kind::CodexSecurityDeep => {}
            Kind::Model => {
                let expected = model_record_for_source(paths, source)?;
                require_record_inputs(&record, &expected)?;
            }
            Kind::VmwareSelfTest | Kind::VmwareShipped => {
                let expected = vmware_record_for_source(paths, kind, source)?;
                require_record_inputs(&record, &expected)?;
            }
            Kind::Advisory => bail!("advisory qualification was verified twice"),
        }
        if matches!(kind, Kind::VmwareSelfTest | Kind::VmwareShipped) {
            let directory = root.join(kind.slug());
            let transcript = directory.join(VMWARE_TRANSCRIPT);
            verify_generation_with_source(source, kind, &directory, &transcript)?;
            let bytes = read_bounded_file(
                &transcript,
                MAX_VMWARE_TRANSCRIPT_BYTES,
                "VMware transcript",
            )?;
            crate::vmware::verify_transcript(
                paths.root(),
                if kind == Kind::VmwareSelfTest {
                    Kernel::SelfTest
                } else {
                    Kernel::Shipped
                },
                &bytes,
            )?;
        }
    }
    Ok(())
}

fn require_authenticated_deep_qualification(
    paths: &Paths,
    root: &Path,
    source: &SourceIdentity,
) -> Result<()> {
    let directory = checked_kind_directory(root, Kind::CodexSecurityDeep)?;
    require_contained_real_directory(root, &directory, "qualification kind directory")?;
    let record = read_record(&directory.join(RECORD), Kind::CodexSecurityDeep)?;
    require_source(&record, source)?;
    verify_support(paths, &record, Kind::CodexSecurityDeep, root)?;
    let derived = crate::security_deep::verify_sealed(paths, &directory, source)?;
    require_external_fields(&record, &derived)
}

pub(super) fn verify_advisory_qualification(
    paths: &Paths,
    root: &Path,
    source: &SourceIdentity,
) -> Result<()> {
    let directory = checked_kind_directory(root, Kind::Advisory)?;
    require_contained_real_directory(root, &directory, "advisory qualification directory")?;
    let record = read_record(&directory.join(RECORD), Kind::Advisory)?;
    require_source(&record, source)?;
    require_advisory_inputs(paths, &record)?;
    verify_support(paths, &record, Kind::Advisory, root)
}

pub(super) fn require_mutation_inputs(
    record: &Record,
    registry: &str,
    mutation_count: usize,
    mutation: Option<&PassEvidence>,
) -> Result<()> {
    ensure!(
        record.get("input.registry.sha256")? == registry,
        "mutation qualification belongs to another registry"
    );
    ensure!(
        record.get("input.mutations")? == mutation_count.to_string(),
        "mutation qualification names another registry cardinality"
    );
    if let Some(mutation) = mutation {
        ensure!(
            mutation.registry == registry
                && mutation.mutation_count == mutation_count
                && record.get("source.commit")? == mutation.commit
                && record.get("input.source.sha256")? == mutation.source_digest
                && record.get("input.index.sha256")? == mutation.index_digest
                && record.get("artifact.report.sha256")? == mutation.report_digest,
            "mutation qualification differs from the newest current pass"
        );
    }
    Ok(())
}

fn require_advisory_inputs(paths: &Paths, record: &Record) -> Result<()> {
    let lock = digest_file(&paths.root().join("Cargo.lock"))?;
    ensure!(
        record.get("input.cargo-lock.bytes")? == lock.bytes.to_string()
            && record.get("input.cargo-lock.sha256")? == lock.sha256,
        "advisory qualification belongs to another Cargo.lock"
    );
    Ok(())
}

pub(super) fn verify_support(
    paths: &Paths,
    record: &Record,
    kind: Kind,
    root: &Path,
) -> Result<()> {
    let artifacts: Vec<(&str, &str)> = match kind {
        Kind::Advisory => vec![
            ("artifact.policy", ADVISORY_POLICY_PATH),
            ("artifact.refresh-transcript", ADVISORY_REFRESH_TRANSCRIPT),
            ("artifact.transcript", ADVISORY_TRANSCRIPT),
        ],
        Kind::Mutation => vec![("artifact.report", MUTATION_REPORT)],
        Kind::CargoMutantsGrade => vec![
            ("artifact.request", "request"),
            ("artifact.inventory", "inventory.json"),
            ("artifact.mutants", "mutants.json"),
            ("artifact.outcomes", "outcomes.json"),
            ("artifact.adjudications", "adjudications.json"),
            ("artifact.doctest", "doctest.transcript"),
        ],
        Kind::CodexSecurityDeep => vec![
            ("artifact.request", "request"),
            ("artifact.manifest", "scan-manifest.json"),
            ("artifact.findings", "findings.json"),
            ("artifact.coverage", "coverage.json"),
            ("artifact.report", "report.md"),
            ("artifact.worker-transitions", "worker-transitions"),
        ],
        Kind::Model => crate::model::TRANSCRIPT_ARTIFACTS
            .iter()
            .map(|artifact| (artifact.prefix, artifact.name))
            .collect(),
        Kind::VmwareSelfTest | Kind::VmwareShipped => vec![
            ("artifact.generation", VMWARE_GENERATION_RECEIPT),
            ("artifact.image", VMWARE_IMAGE),
            ("artifact.namespace", VMWARE_NAMESPACE),
            ("artifact.vmdk", VMWARE_VMDK),
            ("artifact.namespace-vmdk", VMWARE_NAMESPACE_VMDK),
            ("artifact.vmx", VMWARE_VMX),
            ("artifact.handoff", VMWARE_HANDOFF),
            ("artifact.transcript", VMWARE_TRANSCRIPT),
        ],
    };
    for (prefix, name) in &artifacts {
        ensure!(
            record.get(&format!("{prefix}.path"))? == *name,
            "{prefix} names an unexpected qualification path"
        );
        let path = root.join(kind.slug()).join(name);
        require_regular_file(&path, prefix)?;
        require_artifact_size(kind, name, &path)?;
        let (maximum, _) = artifact_size_policy(kind, name)?;
        let digest = digest_file_bounded(&path, maximum)?;
        ensure!(
            record.get(&format!("{prefix}.bytes"))? == digest.bytes.to_string(),
            "{prefix} length differs from its qualification record"
        );
        ensure!(
            record.get(&format!("{prefix}.sha256"))? == digest.sha256,
            "{prefix} digest differs from its qualification record"
        );
        if kind == Kind::Model {
            let bytes = read_bounded_file(
                &path,
                MAX_MODEL_TRANSCRIPT_BYTES,
                "model qualification transcript",
            )?;
            crate::model::verify_transcript(name, &bytes)?;
        }
        if kind == Kind::Advisory && *name == ADVISORY_POLICY_PATH {
            let bytes = read_bounded_file(&path, MAX_ADVISORY_POLICY_BYTES, "advisory policy")?;
            ensure!(
                bytes == ADVISORY_POLICY.as_bytes(),
                "advisory policy differs from the no-ignore phase-close policy"
            );
        }
    }
    if kind == Kind::Advisory {
        let directory = root.join(kind.slug());
        let refresh = read_bounded_file(
            &directory.join(ADVISORY_REFRESH_TRANSCRIPT),
            MAX_ADVISORY_TRANSCRIPT_BYTES,
            "cargo-audit population transcript",
        )?;
        let pinned = read_bounded_file(
            &directory.join(ADVISORY_TRANSCRIPT),
            MAX_ADVISORY_TRANSCRIPT_BYTES,
            "cargo-audit transcript",
        )?;
        let attestation =
            verify_advisory_transcripts(&refresh, &pinned, record.get("database.revision")?)?;
        ensure!(
            record.get("database.last-updated")? == attestation.last_updated,
            "advisory record names another database update time"
        );
    }
    if matches!(kind, Kind::VmwareSelfTest | Kind::VmwareShipped) {
        verify_vmware_observer(paths, record, kind, root)?;
    }
    Ok(())
}

pub(super) fn require_external_fields(
    record: &Record,
    expected: &[(String, String)],
) -> Result<()> {
    for (key, value) in expected {
        ensure!(
            record.get(key)? == value,
            "external qualification field `{key}` differs from its sealed artifacts"
        );
    }
    Ok(())
}

pub(super) fn require_record_inputs(record: &Record, expected: &Record) -> Result<()> {
    for (key, value) in &expected.fields {
        ensure!(
            record.get(key)? == value,
            "qualification input `{key}` differs from the current source or tool"
        );
    }
    Ok(())
}
