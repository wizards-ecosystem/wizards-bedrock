use super::*;

#[derive(Debug)]
pub(crate) struct MutationAttempt {
    paths: Paths,
    record: Record,
}

#[derive(Debug)]
pub(crate) struct ModelAttempt {
    paths: Paths,
    record: Record,
    java: Tool,
}

impl ModelAttempt {
    pub(crate) fn tool(&self) -> &Tool {
        &self.java
    }

    pub(crate) fn pass(mut self, transcripts: &[crate::model::RunTranscript]) -> Result<()> {
        let current = model_record(&self.paths)?;
        ensure!(
            current == self.record,
            "source, Java, TLC or model inputs changed during model qualification"
        );
        crate::model::require_complete_transcripts(transcripts)?;
        let directory = kind_directory(&self.paths, Kind::Model)?;
        for transcript in transcripts {
            let artifact = transcript.artifact();
            let destination = directory.join(artifact.name);
            write_new_file(&destination, transcript.bytes())?;
            require_regular_file(&destination, "model qualification transcript")?;
            let published = read_bounded_file(
                &destination,
                MAX_MODEL_TRANSCRIPT_BYTES,
                "model qualification transcript",
            )?;
            ensure!(
                published == transcript.bytes(),
                "model transcript changed during qualification publication"
            );
            crate::model::verify_transcript(artifact.name, &published)?;
            add_artifact_digest(
                &mut self.record,
                artifact.prefix,
                artifact.name,
                &digest_bytes(&published),
            )?;
        }
        let current = model_record(&self.paths)?;
        require_record_inputs(&self.record, &current)?;
        self.record.insert("result", PASS)?;
        commit_record(&self.paths, Kind::Model, &self.record)
    }
}

#[derive(Debug)]
pub(crate) struct ObservedMutantsArtifacts {
    pub(crate) request: Vec<u8>,
    pub(crate) inventory: Vec<u8>,
    pub(crate) mutants: Vec<u8>,
    pub(crate) outcomes: Vec<u8>,
    pub(crate) adjudications: Vec<u8>,
    pub(crate) doctest: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct ObservedDeepArtifacts {
    pub(crate) request: Vec<u8>,
    pub(crate) manifest: Vec<u8>,
    pub(crate) findings: Vec<u8>,
    pub(crate) coverage: Vec<u8>,
    pub(crate) report: Vec<u8>,
    pub(crate) worker_transitions: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct DeepAttempt {
    paths: Paths,
    source: SourceIdentity,
    directory: PathBuf,
}

#[derive(Debug)]
pub(crate) struct OwnedDeepAttempt {
    paths: Paths,
    source: SourceIdentity,
    directory: PathBuf,
}

#[derive(Debug)]
pub(crate) struct MutantsAttempt {
    paths: Paths,
    source: SourceIdentity,
    directory: PathBuf,
}

impl DeepAttempt {
    pub(crate) fn source(&self) -> &SourceIdentity {
        &self.source
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn write_security_request(&self, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() as u64 <= MAX_SECURITY_REQUEST_BYTES,
            "security request exceeds its size policy"
        );
        let directory = kind_directory(&self.paths, Kind::CodexSecurityDeep)?;
        ensure!(
            directory == self.directory,
            "qualification directory changed during request publication"
        );
        write_new_file(&directory.join("request"), bytes)
    }
}

impl OwnedDeepAttempt {
    pub(crate) fn source(&self) -> &SourceIdentity {
        &self.source
    }

    pub(crate) fn pass_observed_deep(
        self,
        _authority: crate::security_deep::DeepAuthority,
        fields: &[(String, String)],
        artifacts: ObservedDeepArtifacts,
    ) -> Result<()> {
        invalidate(&self.paths, Kind::CodexSecurityDeep)?;
        let directory = kind_directory(&self.paths, Kind::CodexSecurityDeep)?;
        ensure!(
            directory == self.directory,
            "qualification directory changed during owned Deep execution"
        );
        let mut record = base_record_for_source(Kind::CodexSecurityDeep, &self.source)?;
        for (key, value) in fields {
            record.insert(key, value)?;
        }
        for (prefix, name, bytes) in [
            ("artifact.request", "request", artifacts.request.as_slice()),
            (
                "artifact.manifest",
                "scan-manifest.json",
                artifacts.manifest.as_slice(),
            ),
            (
                "artifact.findings",
                "findings.json",
                artifacts.findings.as_slice(),
            ),
            (
                "artifact.coverage",
                "coverage.json",
                artifacts.coverage.as_slice(),
            ),
            ("artifact.report", "report.md", artifacts.report.as_slice()),
            (
                "artifact.worker-transitions",
                "worker-transitions",
                artifacts.worker_transitions.as_slice(),
            ),
        ] {
            let (maximum, exact) = artifact_size_policy(Kind::CodexSecurityDeep, name)?;
            ensure!(
                (!exact && bytes.len() as u64 <= maximum)
                    || (exact && bytes.len() as u64 == maximum),
                "observed Codex Security artifact `{name}` violates its size policy"
            );
            let path = directory.join(name);
            ensure!(
                path.parent() == Some(directory.as_path()),
                "observed Codex Security artifact escaped its qualification directory"
            );
            write_new_file(&path, bytes)?;
            add_artifact(&mut record, prefix, name, &path)?;
        }
        record.insert("result", PASS)?;
        verify_support(
            &self.paths,
            &record,
            Kind::CodexSecurityDeep,
            &qualification_root(&self.paths),
        )?;
        let derived = crate::security_deep::verify_sealed(&self.paths, &directory, &self.source)?;
        require_external_fields(&record, &derived)?;
        commit_record(&self.paths, Kind::CodexSecurityDeep, &record)
    }
}

impl MutantsAttempt {
    pub(crate) fn source(&self) -> &SourceIdentity {
        &self.source
    }

    pub(crate) fn write_mutants_request(&self, bytes: &[u8]) -> Result<()> {
        ensure!(
            bytes.len() as u64 <= MAX_EXTERNAL_RECEIPT_BYTES,
            "cargo-mutants request exceeds its size policy"
        );
        let directory = kind_directory(&self.paths, Kind::CargoMutantsGrade)?;
        ensure!(
            directory == self.directory,
            "qualification directory changed during request publication"
        );
        write_new_file(&directory.join("request"), bytes)
    }

    pub(crate) fn pass_observed_mutants(
        self,
        fields: &[(String, String)],
        artifacts: ObservedMutantsArtifacts,
    ) -> Result<()> {
        invalidate(&self.paths, Kind::CargoMutantsGrade)?;
        let directory = kind_directory(&self.paths, Kind::CargoMutantsGrade)?;
        let mut record = base_record_for_source(Kind::CargoMutantsGrade, &self.source)?;
        for (key, value) in fields {
            record.insert(key, value)?;
        }
        for (prefix, name, bytes) in [
            ("artifact.request", "request", artifacts.request.as_slice()),
            (
                "artifact.inventory",
                "inventory.json",
                artifacts.inventory.as_slice(),
            ),
            (
                "artifact.mutants",
                "mutants.json",
                artifacts.mutants.as_slice(),
            ),
            (
                "artifact.outcomes",
                "outcomes.json",
                artifacts.outcomes.as_slice(),
            ),
            (
                "artifact.adjudications",
                "adjudications.json",
                artifacts.adjudications.as_slice(),
            ),
            (
                "artifact.doctest",
                "doctest.transcript",
                artifacts.doctest.as_slice(),
            ),
        ] {
            let (maximum, exact) = artifact_size_policy(Kind::CargoMutantsGrade, name)?;
            ensure!(
                (!exact && bytes.len() as u64 <= maximum)
                    || (exact && bytes.len() as u64 == maximum),
                "observed cargo-mutants artifact `{name}` violates its size policy"
            );
            let path = directory.join(name);
            ensure!(
                path.parent() == Some(directory.as_path()),
                "observed cargo-mutants artifact escaped its qualification directory"
            );
            write_new_file(&path, bytes)?;
            add_artifact(&mut record, prefix, name, &path)?;
        }
        record.insert("result", PASS)?;
        verify_support(
            &self.paths,
            &record,
            Kind::CargoMutantsGrade,
            &qualification_root(&self.paths),
        )?;
        let derived = crate::mutants_grade::verify_sealed(&self.paths, &directory, &self.source)?;
        require_external_fields(&record, &derived)?;
        commit_record(&self.paths, Kind::CargoMutantsGrade, &record)
    }
}

pub(crate) fn begin_mutation(
    paths: &Paths,
    registry: &str,
    mutation_count: usize,
) -> Result<MutationAttempt> {
    invalidate(paths, Kind::Mutation)?;
    ensure!(is_digest(registry), "mutation registry digest is malformed");
    ensure!(
        mutation_count != 0,
        "a mutation qualification has no mutations"
    );
    let mut record = base_record(paths, Kind::Mutation)?;
    record.insert("input.mutations", mutation_count.to_string())?;
    record.insert("input.registry.sha256", registry)?;
    Ok(MutationAttempt {
        paths: paths.clone(),
        record,
    })
}

fn begin_external(
    paths: &Paths,
    kind: Kind,
    preserve_artifacts: bool,
) -> Result<(SourceIdentity, PathBuf)> {
    if preserve_artifacts {
        invalidate_record(paths, kind)?;
    } else {
        invalidate(paths, kind)?;
    }
    Ok((source_identity(paths.root())?, kind_directory(paths, kind)?))
}

pub(crate) fn begin_security_deep(paths: &Paths, preserve_artifacts: bool) -> Result<DeepAttempt> {
    let (source, directory) = begin_external(paths, Kind::CodexSecurityDeep, preserve_artifacts)?;
    Ok(DeepAttempt {
        paths: paths.clone(),
        source,
        directory,
    })
}

pub(crate) fn begin_owned_security_deep(
    paths: &Paths,
    _authority: &crate::security_deep::DeepAuthority,
) -> Result<OwnedDeepAttempt> {
    let (source, directory) = begin_external(paths, Kind::CodexSecurityDeep, false)?;
    Ok(OwnedDeepAttempt {
        paths: paths.clone(),
        source,
        directory,
    })
}

pub(crate) fn begin_mutants_grade(paths: &Paths) -> Result<MutantsAttempt> {
    let (source, directory) = begin_external(paths, Kind::CargoMutantsGrade, false)?;
    Ok(MutantsAttempt {
        paths: paths.clone(),
        source,
        directory,
    })
}

impl MutationAttempt {
    pub(crate) fn pass(mut self, current: &CurrentPass) -> Result<()> {
        let evidence = current.evidence();
        require_current_source(&self.paths, &self.record)?;
        ensure!(
            self.record.get("source.commit")? == evidence.commit,
            "mutation report belongs to another commit"
        );
        ensure!(
            self.record.get("input.registry.sha256")? == evidence.registry
                && self.record.get("input.mutations")? == evidence.mutation_count.to_string(),
            "mutation report belongs to another registry"
        );
        self.record
            .insert("input.source.sha256", &evidence.source_digest)?;
        self.record
            .insert("input.index.sha256", &evidence.index_digest)?;
        let directory = kind_directory(&self.paths, Kind::Mutation)?;
        let destination = directory.join(MUTATION_REPORT);
        let digest = digest_file(&evidence.report_path)?;
        ensure!(
            digest.sha256 == evidence.report_digest,
            "mutation report changed after pass verification"
        );
        copy_exact(&evidence.report_path, &destination, &digest)?;
        add_artifact(
            &mut self.record,
            "artifact.report",
            MUTATION_REPORT,
            &destination,
        )?;
        self.record.insert("result", PASS)?;
        commit_record(&self.paths, Kind::Mutation, &self.record)
    }
}

pub(crate) fn begin_model(paths: &Paths) -> Result<ModelAttempt> {
    invalidate(paths, Kind::Model)?;
    let java = java(paths.root())?.context(
        "Java is unavailable; put the JVM that runs TLC (java) on PATH; \
         tools/tla/tla2tools.jar is TLC itself, not a JVM",
    )?;
    let record = model_record_with_tool(paths, &java)?;
    Ok(ModelAttempt {
        paths: paths.clone(),
        record,
        java,
    })
}
