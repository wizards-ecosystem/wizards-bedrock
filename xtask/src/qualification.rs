//! Exceptional qualification records and their resolved tools.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::time::Instant;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

use crate::image::Kernel;
use crate::json::{
    JsonValue, json_array as parse_json_array, json_field as parse_json_field, json_object,
    json_string_array as parse_json_string_array, json_u64 as parse_json_u64, require_json_fields,
};
use crate::mutation_workspace::{CurrentPass, PassEvidence};
use crate::paths::Paths;
use crate::sha256;

const FORMAT: &str = "wizard-qualification-v6";
const RECORD: &str = "record";
const RECORD_TEMP: &str = "record.incomplete";
const PASS: &str = "pass";
// Qualification probes share a host with compiler-heavy release checks.
// Thirty seconds remains a strict bound while tolerating scheduler latency.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const ADVISORY_TIMEOUT: Duration = Duration::from_secs(300);
const WINDOWS_VMWARE: &str = r"C:\Program Files (x86)\VMware\VMware Workstation";

const MUTATION_REPORT: &str = "report";
const ADVISORY_POLICY: &str = "[advisories]\nignore = []\n";
const ADVISORY_POLICY_PATH: &str = ".cargo/audit.toml";
const ADVISORY_DATABASE: &str = "database";
const ADVISORY_DATABASE_URL: &str = "https://github.com/RustSec/advisory-db.git";
const ADVISORY_TOOL_VERSION: &str = "cargo-audit-audit 0.22.2";
const ADVISORY_TRANSCRIPT: &str = "cargo-audit.transcript";
const ADVISORY_REFRESH_TRANSCRIPT: &str = "cargo-audit-refresh.transcript";
const ADVISORY_TRANSCRIPT_FORMAT: &str = "wizard-cargo-audit-transcript-v1";
const ADVISORY_REFRESH_COMMAND: &str = "cargo-audit audit --color never --db <empty-qualification-database> --deny warnings --file <exact-repository-Cargo.lock> --json --url https://github.com/RustSec/advisory-db.git";
const ADVISORY_COMMAND: &str = "cargo-audit audit --color never --db <refreshed-qualification-database> --deny warnings --file <exact-repository-Cargo.lock> --json --no-fetch --url https://github.com/RustSec/advisory-db.git";
const VMWARE_IMAGE: &str = "bedrock-flat.img";
const VMWARE_NAMESPACE: &str = "wizard-nvm-flat.img";
const VMWARE_VMDK: &str = "bedrock.vmdk";
const VMWARE_NAMESPACE_VMDK: &str = "wizard-nvm.vmdk";
const VMWARE_VMX: &str = "bedrock.vmx";
const VMWARE_HANDOFF: &str = crate::platform_acpi::DMA_HANDOFF_FILE;
const VMWARE_TRANSCRIPT: &str = "serial.log";
const VMWARE_OBSERVER: &str = "vmrun-list.txt";
pub(crate) const VMWARE_GENERATION_RECEIPT: &str = "generation.receipt";
const VMWARE_GENERATION_TEMP: &str = "generation.receipt.incomplete";
const VMWARE_GENERATION_KIND: &str = "vmware-generation";
const RESET: &str = "reset";
const VMRUN_LIST_HEADER: &str = "Total running VMs: ";
const VMRUN_LIST_ARGS: &[&str] = &["-T", "ws", "list"];
const SERIAL_PREFIX: &str = "serial-";
const SERIAL_SUFFIX: &str = ".log";
const INITIAL_VMWARE_FILES: usize = 8;
const MAX_RECORD_BYTES: u64 = 256 * 1024;
const MAX_ADVISORY_POLICY_BYTES: u64 = 4096;
const MAX_ADVISORY_TRANSCRIPT_BYTES: u64 = crate::bounded_process::MAX_CAPTURE_BYTES as u64 + 4096;
const MAX_MODEL_TRANSCRIPT_BYTES: u64 = crate::bounded_process::MAX_CAPTURE_BYTES as u64 + 4096;
const MAX_MUTATION_REPORT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MUTANTS_JSON_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MUTANTS_ADJUDICATION_BYTES: u64 = 16 * 1024 * 1024;
const MAX_EXTERNAL_RECEIPT_BYTES: u64 = 1024 * 1024;
const MAX_SECURITY_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SECURITY_FINDINGS_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SECURITY_COVERAGE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SECURITY_REPORT_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SECURITY_REQUEST_BYTES: u64 = 256 * 1024;
const MAX_SECURITY_TRANSITIONS_BYTES: u64 = 1024 * 1024;
const MAX_VMWARE_TEXT_BYTES: u64 = 1024 * 1024;
const MAX_VMWARE_TRANSCRIPT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_GIT_IDENTITY_BYTES: usize = 4096;
const MAX_GIT_STATUS_BYTES: usize = 1024 * 1024;
const MAX_MODEL_PATH_LIST_BYTES: usize = 64 * 1024;
const MAX_MODEL_FILES: usize = 128;
const MAX_MODEL_FILE_BYTES: u64 = 1024 * 1024;
const MAX_MODEL_TREE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_DIGEST_FILE_BYTES: u64 = 512 * 1024 * 1024;
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(0);

/// One resolved executable and the version it reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tool {
    pub(crate) path: PathBuf,
    pub(crate) version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mutation,
    CargoMutantsGrade,
    CodexSecurityDeep,
    Advisory,
    Model,
    VmwareSelfTest,
    VmwareShipped,
}

impl Kind {
    fn slug(self) -> &'static str {
        match self {
            Self::Mutation => "mutation",
            Self::CargoMutantsGrade => "cargo-mutants-grade",
            Self::CodexSecurityDeep => "codex-security-deep",
            Self::Advisory => "advisory",
            Self::Model => "model",
            Self::VmwareSelfTest => "vmware-self-test",
            Self::VmwareShipped => "vmware-shipped",
        }
    }

    pub(crate) fn vmware(kernel: Kernel) -> Self {
        match kernel {
            Kernel::SelfTest => Self::VmwareSelfTest,
            Kernel::Shipped => Self::VmwareShipped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Digest {
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Record {
    fields: BTreeMap<String, String>,
}

impl Record {
    fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) -> Result<()> {
        let key = key.into();
        let value = value.into();
        validate_field(&key, &value)?;
        ensure!(
            self.fields.insert(key.clone(), value).is_none(),
            "duplicate qualification key `{key}`"
        );
        Ok(())
    }

    fn get(&self, key: &str) -> Result<&str> {
        self.fields
            .get(key)
            .map(String::as_str)
            .with_context(|| format!("qualification record has no `{key}`"))
    }

    fn render(&self) -> String {
        let mut body = format!("{FORMAT}\n");
        for (key, value) in &self.fields {
            body.push_str(key);
            body.push('=');
            body.push_str(value);
            body.push('\n');
        }
        let checksum = sha256::hex(&sha256::digest(body.as_bytes()));
        format!("{body}checksum.sha256={checksum}\n")
    }
}

#[derive(Debug)]
pub(crate) struct MutationAttempt {
    paths: Paths,
    record: Record,
}

#[derive(Debug)]
struct AdvisoryAttempt {
    paths: Paths,
    record: Record,
    tool: Tool,
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
pub(crate) struct VmwareAttempt {
    paths: Paths,
    kind: Kind,
    record: Record,
    vmrun: Tool,
}

pub(crate) struct VmwareArtifacts<'a> {
    pub(crate) image: &'a Path,
    pub(crate) namespace: &'a Path,
    pub(crate) vmdk: &'a Path,
    pub(crate) namespace_vmdk: &'a Path,
    pub(crate) vmx: &'a Path,
    pub(crate) handoff: &'a Path,
    pub(crate) transcript: &'a Path,
    pub(crate) receipt: &'a Path,
}

#[derive(Debug)]
struct CapturedVmwareFile {
    prefix: &'static str,
    name: &'static str,
    source: PathBuf,
    captured: PathBuf,
    digest: Digest,
}

#[derive(Debug)]
struct CapturedVmwareGeneration {
    files: Vec<CapturedVmwareFile>,
}

impl CapturedVmwareGeneration {
    fn read(artifacts: &VmwareArtifacts<'_>, directory: &Path) -> Result<Self> {
        Self::read_with_sizes(artifacts, directory, boot_image_bytes(), namespace_bytes())
    }

    fn read_with_sizes(
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

    fn validate_generation(&self, source: &SourceIdentity, kind: Kind) -> Result<()> {
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

    fn verify_sources(&self) -> Result<()> {
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

    fn publish(&self, directory: &Path, record: &mut Record) -> Result<()> {
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

    fn file(&self, name: &str) -> Result<&CapturedVmwareFile> {
        self.files
            .iter()
            .find(|file| file.name == name)
            .with_context(|| format!("captured VMware generation has no `{name}`"))
    }
}

fn vmware_artifact_sources<'a>(
    artifacts: &VmwareArtifacts<'a>,
) -> [(&'static str, &'static str, &'a Path); 8] {
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct BundleFile {
    pub(crate) prefix: &'static str,
    pub(crate) relative: &'static str,
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

pub(crate) const BUNDLE_FILES: &[BundleFile] = &[
    BundleFile {
        prefix: "qualification.cargo-mutants-grade.record",
        relative: "qualification/cargo-mutants-grade/record",
    },
    BundleFile {
        prefix: "qualification.cargo-mutants-grade.request",
        relative: "qualification/cargo-mutants-grade/request",
    },
    BundleFile {
        prefix: "qualification.cargo-mutants-grade.inventory",
        relative: "qualification/cargo-mutants-grade/inventory.json",
    },
    BundleFile {
        prefix: "qualification.cargo-mutants-grade.mutants",
        relative: "qualification/cargo-mutants-grade/mutants.json",
    },
    BundleFile {
        prefix: "qualification.cargo-mutants-grade.outcomes",
        relative: "qualification/cargo-mutants-grade/outcomes.json",
    },
    BundleFile {
        prefix: "qualification.cargo-mutants-grade.adjudications",
        relative: "qualification/cargo-mutants-grade/adjudications.json",
    },
    BundleFile {
        prefix: "qualification.cargo-mutants-grade.doctest",
        relative: "qualification/cargo-mutants-grade/doctest.transcript",
    },
    BundleFile {
        prefix: "qualification.codex-security-deep.record",
        relative: "qualification/codex-security-deep/record",
    },
    BundleFile {
        prefix: "qualification.codex-security-deep.request",
        relative: "qualification/codex-security-deep/request",
    },
    BundleFile {
        prefix: "qualification.codex-security-deep.manifest",
        relative: "qualification/codex-security-deep/scan-manifest.json",
    },
    BundleFile {
        prefix: "qualification.codex-security-deep.findings",
        relative: "qualification/codex-security-deep/findings.json",
    },
    BundleFile {
        prefix: "qualification.codex-security-deep.coverage",
        relative: "qualification/codex-security-deep/coverage.json",
    },
    BundleFile {
        prefix: "qualification.codex-security-deep.report",
        relative: "qualification/codex-security-deep/report.md",
    },
    BundleFile {
        prefix: "qualification.codex-security-deep.worker-transitions",
        relative: "qualification/codex-security-deep/worker-transitions",
    },
    BundleFile {
        prefix: "qualification.advisory.record",
        relative: "qualification/advisory/record",
    },
    BundleFile {
        prefix: "qualification.advisory.policy",
        relative: "qualification/advisory/.cargo/audit.toml",
    },
    BundleFile {
        prefix: "qualification.advisory.transcript",
        relative: "qualification/advisory/cargo-audit.transcript",
    },
    BundleFile {
        prefix: "qualification.advisory.refresh-transcript",
        relative: "qualification/advisory/cargo-audit-refresh.transcript",
    },
    BundleFile {
        prefix: "qualification.mutation.record",
        relative: "qualification/mutation/record",
    },
    BundleFile {
        prefix: "qualification.mutation.report",
        relative: "qualification/mutation/report",
    },
    BundleFile {
        prefix: "qualification.model.record",
        relative: "qualification/model/record",
    },
    BundleFile {
        prefix: "qualification.model.ring-clean",
        relative: "qualification/model/ring-clean.transcript",
    },
    BundleFile {
        prefix: "qualification.model.ring-memory-clean",
        relative: "qualification/model/ring-memory-clean.transcript",
    },
    BundleFile {
        prefix: "qualification.model.ring-memory-store-relaxed",
        relative: "qualification/model/ring-memory-store-relaxed.transcript",
    },
    BundleFile {
        prefix: "qualification.model.ring-memory-load-relaxed",
        relative: "qualification/model/ring-memory-load-relaxed.transcript",
    },
    BundleFile {
        prefix: "qualification.model.ring-memory-no-reservation",
        relative: "qualification/model/ring-memory-no-reservation.transcript",
    },
    BundleFile {
        prefix: "qualification.model.ring-memory-duplicate-terminal",
        relative: "qualification/model/ring-memory-duplicate-terminal.transcript",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.record",
        relative: "qualification/vmware-self-test/record",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.generation",
        relative: "qualification/vmware-self-test/generation.receipt",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.image",
        relative: "qualification/vmware-self-test/bedrock-flat.img",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.namespace",
        relative: "qualification/vmware-self-test/wizard-nvm-flat.img",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.vmdk",
        relative: "qualification/vmware-self-test/bedrock.vmdk",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.namespace-vmdk",
        relative: "qualification/vmware-self-test/wizard-nvm.vmdk",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.vmx",
        relative: "qualification/vmware-self-test/bedrock.vmx",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.handoff",
        relative: "qualification/vmware-self-test/dma-handoff.bin",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.transcript",
        relative: "qualification/vmware-self-test/serial.log",
    },
    BundleFile {
        prefix: "qualification.vmware-self-test.observer",
        relative: "qualification/vmware-self-test/vmrun-list.txt",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.record",
        relative: "qualification/vmware-shipped/record",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.generation",
        relative: "qualification/vmware-shipped/generation.receipt",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.image",
        relative: "qualification/vmware-shipped/bedrock-flat.img",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.namespace",
        relative: "qualification/vmware-shipped/wizard-nvm-flat.img",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.vmdk",
        relative: "qualification/vmware-shipped/bedrock.vmdk",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.namespace-vmdk",
        relative: "qualification/vmware-shipped/wizard-nvm.vmdk",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.vmx",
        relative: "qualification/vmware-shipped/bedrock.vmx",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.handoff",
        relative: "qualification/vmware-shipped/dma-handoff.bin",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.transcript",
        relative: "qualification/vmware-shipped/serial.log",
    },
    BundleFile {
        prefix: "qualification.vmware-shipped.observer",
        relative: "qualification/vmware-shipped/vmrun-list.txt",
    },
];

const DEFERRED: &str = "deferred";

/// The grade and Deep run continuously against `main`, so a release bundle
/// carries either their exact-source record or an explicit deferral (D-425).
const DEFERRABLE: [Kind; 2] = [Kind::CargoMutantsGrade, Kind::CodexSecurityDeep];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CampaignState {
    Pass,
    Deferred,
}

impl CampaignState {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => PASS,
            Self::Deferred => DEFERRED,
        }
    }
}

/// Whether each continuous campaign passed for a bundle's source or was deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CampaignStates {
    states: [CampaignState; 2],
}

impl CampaignStates {
    /// Observe which continuous campaigns carry a record below `root`.
    pub(crate) fn observe(root: &Path) -> Result<Self> {
        let mut states = [CampaignState::Deferred; 2];
        for (state, kind) in states.iter_mut().zip(DEFERRABLE) {
            if record_present(root, kind)? {
                *state = CampaignState::Pass;
            }
        }
        Ok(Self { states })
    }

    /// The manifest keys declaring each campaign's state, in campaign order.
    pub(crate) fn status_keys() -> [String; 2] {
        DEFERRABLE.map(|kind| format!("qualification.{}.status", kind.slug()))
    }

    /// The manifest value of each campaign's state, in campaign order.
    pub(crate) fn labels(&self) -> [&'static str; 2] {
        self.states.map(CampaignState::label)
    }

    /// Parse declared manifest values, which are exactly `pass` or `deferred`.
    pub(crate) fn from_labels(labels: [&str; 2]) -> Result<Self> {
        let mut states = [CampaignState::Deferred; 2];
        for ((state, label), kind) in states.iter_mut().zip(labels).zip(DEFERRABLE) {
            *state = match label {
                PASS => CampaignState::Pass,
                DEFERRED => CampaignState::Deferred,
                other => bail!(
                    "{} campaign status `{other}` is neither pass nor deferred",
                    kind.slug()
                ),
            };
        }
        Ok(Self { states })
    }

    /// Whether `kind` is a continuous campaign this bundle deferred.
    pub(crate) fn deferred(&self, kind: Kind) -> bool {
        DEFERRABLE
            .iter()
            .zip(self.states)
            .any(|(candidate, state)| *candidate == kind && state == CampaignState::Deferred)
    }

    /// The bundle files this state set carries: every file but a deferred campaign's.
    pub(crate) fn bundle_files(&self) -> Vec<&'static BundleFile> {
        BUNDLE_FILES
            .iter()
            .filter(|spec| {
                !DEFERRABLE.iter().any(|kind| {
                    self.deferred(*kind)
                        && spec
                            .prefix
                            .starts_with(&format!("qualification.{}.", kind.slug()))
                })
            })
            .collect()
    }
}

fn record_present(root: &Path, kind: Kind) -> Result<bool> {
    let directory = checked_kind_directory(root, kind)?;
    match std::fs::symlink_metadata(&directory) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", directory.display()));
        }
    }
    require_contained_real_directory(root, &directory, "qualification kind directory")?;
    crate::mutation_workspace::require_direct_file_if_present(
        &directory,
        &directory.join(RECORD),
        "qualification record",
    )
}

fn require_deferred(root: &Path, kind: Kind) -> Result<()> {
    ensure!(
        !record_present(root, kind)?,
        "deferred {} campaign carries a qualification record",
        kind.slug()
    );
    Ok(())
}

#[derive(Debug)]
pub(crate) struct Snapshot {
    root: PathBuf,
    digests: Vec<Digest>,
    campaigns: CampaignStates,
    _mutation: RetainedMutationPass,
}

#[derive(Debug)]
struct RetainedMutationPass {
    evidence: PassEvidence,
    _lock: Option<CurrentPass>,
}

impl RetainedMutationPass {
    fn new(pass: CurrentPass) -> Self {
        Self {
            evidence: pass.evidence().clone(),
            _lock: Some(pass),
        }
    }

    fn evidence(&self) -> &PassEvidence {
        &self.evidence
    }
}

impl Snapshot {
    /// The campaign states this snapshot observed and digested.
    pub(crate) fn campaigns(&self) -> CampaignStates {
        self.campaigns
    }

    pub(crate) fn install(&self, evidence_root: &Path) -> Result<()> {
        self.verify_unchanged()?;
        for (spec, expected) in self.campaigns.bundle_files().into_iter().zip(&self.digests) {
            let source = self.root.join(bundle_tail(spec)?);
            require_bundle_file_size(spec, &source)?;
            let destination = evidence_root.join(spec.relative);
            let parent = destination
                .parent()
                .context("qualification bundle destination has no parent")?;
            prepare_contained_real_directory(
                evidence_root,
                parent,
                "qualification bundle directory",
            )?;
            copy_exact(&source, &destination, expected)?;
            require_contained_regular_file(
                evidence_root,
                &destination,
                "qualification bundle artifact",
            )?;
        }
        Ok(())
    }

    fn verify_unchanged(&self) -> Result<()> {
        for (spec, expected) in self.campaigns.bundle_files().into_iter().zip(&self.digests) {
            let path = self.root.join(bundle_tail(spec)?);
            require_contained_regular_file(&self.root, &path, "qualification snapshot artifact")?;
            require_bundle_file_size(spec, &path)?;
            ensure!(
                digest_file(&path)? == *expected,
                "qualification artifact {} changed after it was accepted",
                path.display()
            );
        }
        Ok(())
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

pub(crate) fn qualify_advisory(paths: &Paths) -> Result<()> {
    begin_advisory(paths)?.run()
}

fn begin_advisory(paths: &Paths) -> Result<AdvisoryAttempt> {
    begin_advisory_with(paths, || cargo_audit(paths.root()))
}

fn begin_advisory_with(
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

fn advisory_command(
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

pub(crate) fn begin_model(paths: &Paths) -> Result<ModelAttempt> {
    invalidate(paths, Kind::Model)?;
    let java = java(paths.root())?.context("Java is unavailable; put the JVM for TLC on PATH")?;
    let record = model_record_with_tool(paths, &java)?;
    Ok(ModelAttempt {
        paths: paths.clone(),
        record,
        java,
    })
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
        matches!(
            core::iter::Iterator::next(&mut components),
            Some(Component::Normal(_))
        ) && core::iter::Iterator::next(&mut components).is_none(),
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
        crate::mutation_workspace::current_pass(paths, registry, mutation_count)?
    } else {
        let directory = checked_kind_directory(&root, Kind::Mutation)?;
        let record = read_record(&directory.join(RECORD), Kind::Mutation)?;
        require_source(&record, source)?;
        crate::mutation_workspace::recorded_pass(
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

fn verify_records_after_authority(
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
                crate::mutation_workspace::verify_recorded_pass(
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

fn verify_advisory_qualification(
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

fn require_mutation_inputs(
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

fn verify_support(paths: &Paths, record: &Record, kind: Kind, root: &Path) -> Result<()> {
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

fn require_external_fields(record: &Record, expected: &[(String, String)]) -> Result<()> {
    for (key, value) in expected {
        ensure!(
            record.get(key)? == value,
            "external qualification field `{key}` differs from its sealed artifacts"
        );
    }
    Ok(())
}

fn verify_vmware_observer(paths: &Paths, record: &Record, kind: Kind, root: &Path) -> Result<()> {
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

fn require_record_inputs(record: &Record, expected: &Record) -> Result<()> {
    for (key, value) in &expected.fields {
        ensure!(
            record.get(key)? == value,
            "qualification input `{key}` differs from the current source or tool"
        );
    }
    Ok(())
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

fn require_exact_vmware_directory(paths: &Paths, kind: Kind, directory: &Path) -> Result<()> {
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

fn verify_generation_with_source(
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

fn generation_record(
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

fn verified_dma_handoff_digest(path: &Path) -> Result<Digest> {
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

fn read_generation_record(path: &Path, kind: Kind) -> Result<Record> {
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

fn require_empty_reset(reset: &Digest) -> Result<()> {
    ensure!(
        reset.bytes == 0 && reset.sha256 == empty_digest(),
        "VMware transcript was not empty at generation reset"
    );
    Ok(())
}

fn generation_transcript_name(generation: &str) -> Result<String> {
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

fn require_generation_inventory(
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

fn require_regular_file(path: &Path, label: &str) -> Result<()> {
    let parent = path.parent().context("regular file has no parent")?;
    crate::mutation_workspace::canonical_existing_real_directory(parent, label)?;
    crate::mutation_workspace::require_direct_file(parent, path, label)
}

fn require_real_directory(path: &Path, label: &str) -> Result<()> {
    crate::mutation_workspace::canonical_existing_real_directory(path, label)?;
    Ok(())
}

fn require_contained_real_directory(root: &Path, directory: &Path, label: &str) -> Result<()> {
    require_real_root(root, label)?;
    let relative = directory
        .strip_prefix(root)
        .with_context(|| format!("{label} escaped {}", root.display()))?;
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "{label} has an unsafe relative path"
    );
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        require_real_directory(&current, label)?;
    }
    let canonical_root = std::fs::canonicalize(root)
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let canonical_directory = std::fs::canonicalize(directory)
        .with_context(|| format!("canonicalizing {}", directory.display()))?;
    ensure!(
        canonical_directory.starts_with(&canonical_root) && canonical_directory != canonical_root,
        "{label} escaped the canonical root {}",
        root.display()
    );
    Ok(())
}

fn prepare_contained_real_directory(root: &Path, directory: &Path, label: &str) -> Result<()> {
    require_real_root(root, label)?;
    let relative = directory
        .strip_prefix(root)
        .with_context(|| format!("{label} escaped {}", root.display()))?;
    ensure!(
        !relative.as_os_str().is_empty()
            && relative
                .components()
                .all(|component| matches!(component, Component::Normal(_))),
        "{label} has an unsafe relative path"
    );
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current)
                    .with_context(|| format!("creating {label} {}", current.display()))?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", current.display()));
            }
        }
        require_real_directory(&current, label)?;
    }
    require_contained_real_directory(root, directory, label)
}

fn require_real_root(path: &Path, label: &str) -> Result<()> {
    require_real_directory(path, label)?;
    let parent = path.parent().context("contained root has no parent")?;
    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalizing {}", parent.display()))?;
    let canonical_path = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalizing {}", path.display()))?;
    ensure!(
        canonical_path.parent() == Some(canonical_parent.as_path()),
        "{label} escaped its canonical parent"
    );
    Ok(())
}

fn require_contained_regular_file(root: &Path, path: &Path, label: &str) -> Result<()> {
    let parent = path.parent().context("contained file has no parent")?;
    require_contained_real_directory(root, parent, label)?;
    require_regular_file(path, label)?;
    let canonical_root = std::fs::canonicalize(root)
        .with_context(|| format!("canonicalizing {}", root.display()))?;
    let canonical_path = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalizing {}", path.display()))?;
    ensure!(
        canonical_path.starts_with(canonical_root),
        "{label} escaped its canonical root"
    );
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    OpenOptions::create_new(&mut options, true);
    OpenOptions::write(&mut options, true);
    let mut file = OpenOptions::open(&options, path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", path.display()))
}

fn kernel_for(kind: Kind) -> Result<Kernel> {
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

fn base_record(paths: &Paths, kind: Kind) -> Result<Record> {
    let source = source_identity(paths.root())?;
    base_record_for_source(kind, &source)
}

fn base_record_for_source(kind: Kind, source: &SourceIdentity) -> Result<Record> {
    let mut record = Record::default();
    record.insert("kind", kind.slug())?;
    record.insert("source.commit", &source.commit)?;
    record.insert("source.tree", &source.tree)?;
    for (key, value) in crate::git::resolved()?.fields() {
        record.insert(key, value)?;
    }
    Ok(record)
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

fn model_record(paths: &Paths) -> Result<Record> {
    let source = source_identity(paths.root())?;
    model_record_for_source(paths, &source)
}

fn model_record_for_source(paths: &Paths, source: &SourceIdentity) -> Result<Record> {
    let java = java(paths.root())?.context("Java is unavailable; model evidence cannot verify")?;
    model_record_with_tool_and_source(paths, &java, source)
}

fn model_record_with_tool(paths: &Paths, java: &Tool) -> Result<Record> {
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
    let models = tracked_tree_digest(paths.root(), "models")?;
    record.insert("input.models.files", models.files.to_string())?;
    record.insert("input.models.bytes", models.bytes.to_string())?;
    record.insert("input.models.sha256", models.sha256)?;
    Ok(record)
}

fn vmware_record_for_source(paths: &Paths, kind: Kind, source: &SourceIdentity) -> Result<Record> {
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

fn require_current_source(paths: &Paths, record: &Record) -> Result<()> {
    require_source(record, &source_identity(paths.root())?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

fn require_stable_source_capture(
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

fn require_source(record: &Record, source: &SourceIdentity) -> Result<()> {
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

fn invalidate(paths: &Paths, kind: Kind) -> Result<()> {
    let root = crate::mutation_workspace::prepare_real_directory_tree(
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

fn invalidate_record(paths: &Paths, kind: Kind) -> Result<()> {
    let root = crate::mutation_workspace::prepare_real_directory_tree(
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
        if crate::mutation_workspace::require_direct_file_if_present(
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

fn commit_record(paths: &Paths, kind: Kind, record: &Record) -> Result<()> {
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

fn read_record(path: &Path, kind: Kind) -> Result<Record> {
    require_regular_file(path, "qualification record")?;
    let text = read_bounded_text(path, MAX_RECORD_BYTES, "qualification record")?;
    let record = parse_record(&text)?;
    validate_record(&record, kind)?;
    Ok(record)
}

fn parse_record(text: &str) -> Result<Record> {
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

fn validate_record(record: &Record, kind: Kind) -> Result<()> {
    let mut expected = BTreeSet::from([
        "kind".to_string(),
        "result".to_string(),
        "source.commit".to_string(),
        "source.tree".to_string(),
    ]);
    expected.extend(crate::git::FIELDS.map(str::to_string));
    match kind {
        Kind::Advisory => {
            add_tool_keys(&mut expected, "input.cargo-audit");
            expected.extend([
                "database.last-updated".to_string(),
                "database.revision".to_string(),
                "database.tree".to_string(),
                "database.url".to_string(),
                "execution.command".to_string(),
                "execution.exit-code".to_string(),
                "execution.refresh-command".to_string(),
                "execution.refresh-exit-code".to_string(),
                "input.cargo-lock.bytes".to_string(),
                "input.cargo-lock.sha256".to_string(),
                "observation.utc-unix-seconds".to_string(),
            ]);
            add_artifact_keys(&mut expected, "artifact.policy");
            add_artifact_keys(&mut expected, "artifact.refresh-transcript");
            add_artifact_keys(&mut expected, "artifact.transcript");
        }
        Kind::Mutation => {
            expected.extend([
                "input.index.sha256".to_string(),
                "input.mutations".to_string(),
                "input.registry.sha256".to_string(),
                "input.source.sha256".to_string(),
            ]);
            add_artifact_keys(&mut expected, "artifact.report");
        }
        Kind::CargoMutantsGrade => {
            expected.extend([
                "execution.authority".to_string(),
                "grade.adjudicated".to_string(),
                "grade.caught".to_string(),
                "grade.missed".to_string(),
                "grade.timeout".to_string(),
                "grade.total".to_string(),
                "grade.unviable".to_string(),
                "tool.cargo-mutants.bytes".to_string(),
                "tool.cargo-mutants.sha256".to_string(),
                "tool.cargo-mutants.version".to_string(),
                "tool.cargo-nextest.archive.sha256".to_string(),
                "tool.cargo-nextest.bytes".to_string(),
                "tool.cargo-nextest.commit".to_string(),
                "tool.cargo-nextest.sha256".to_string(),
                "tool.cargo-nextest.url".to_string(),
                "tool.cargo-nextest.version".to_string(),
            ]);
            for prefix in [
                "artifact.request",
                "artifact.inventory",
                "artifact.mutants",
                "artifact.outcomes",
                "artifact.adjudications",
                "artifact.doctest",
            ] {
                add_artifact_keys(&mut expected, prefix);
            }
        }
        Kind::CodexSecurityDeep => {
            expected.extend([
                "authentication.channel".to_string(),
                "environment.policy".to_string(),
                "execution.authority".to_string(),
                "execution.complete-tool".to_string(),
                "execution.model".to_string(),
                "execution.protocol".to_string(),
                "execution.reasoning".to_string(),
                "execution.server-name".to_string(),
                "execution.server-version".to_string(),
                "execution.start-tool".to_string(),
                "execution.tools.bytes".to_string(),
                "execution.tools.count".to_string(),
                "execution.tools.sha256".to_string(),
                "execution.worker-permission".to_string(),
                "execution.worker-effective-profile.sha256".to_string(),
                "execution.worker-effective-state.sha256".to_string(),
                "execution.worker-parent-state.sha256".to_string(),
                "execution.worker-provider-profile.sha256".to_string(),
                "execution.worker-preflight.allowed-reads".to_string(),
                "execution.worker-preflight.denied-reads".to_string(),
                "execution.worker-preflight.denied-write-creates".to_string(),
                "execution.worker-preflight.denied-write-mutations".to_string(),
                "execution.worker-preflight.denied-network-listeners".to_string(),
                "execution.worker-preflight.denied-network-connections".to_string(),
                "execution.worker-preflight.denied-descendants".to_string(),
                "execution.worker-transitions.app-server".to_string(),
                "execution.worker-transitions.exec-initial".to_string(),
                "execution.worker-transitions.exec-resumed".to_string(),
                "findings.critical".to_string(),
                "findings.high".to_string(),
                "findings.informational".to_string(),
                "findings.low".to_string(),
                "findings.medium".to_string(),
                "findings.total".to_string(),
                "scan.id".to_string(),
                "scan.producer.name".to_string(),
                "scan.producer.version".to_string(),
                "scan.surfaces".to_string(),
                "tool.codex-security.integrity.bytes".to_string(),
                "tool.codex-security.integrity.sha256".to_string(),
                "tool.codex-security.tree.files".to_string(),
                "tool.codex-security.version".to_string(),
                "tool.codex.runtime.bytes".to_string(),
                "tool.codex.runtime.files".to_string(),
                "tool.codex.runtime.root".to_string(),
                "tool.codex.runtime.sha256".to_string(),
                "tool.python.runtime.bytes".to_string(),
                "tool.python.runtime.files".to_string(),
                "tool.python.runtime.root".to_string(),
                "tool.python.runtime.sha256".to_string(),
            ]);
            for prefix in ["tool.node", "tool.python", "tool.codex"] {
                add_tool_keys(&mut expected, prefix);
            }
            for prefix in [
                "artifact.request",
                "artifact.manifest",
                "artifact.findings",
                "artifact.coverage",
                "artifact.report",
                "artifact.worker-transitions",
            ] {
                add_artifact_keys(&mut expected, prefix);
            }
        }
        Kind::Model => {
            add_tool_keys(&mut expected, "input.java");
            expected.extend([
                "input.models.bytes".to_string(),
                "input.models.files".to_string(),
                "input.models.sha256".to_string(),
                "input.tlc-jar.bytes".to_string(),
                "input.tlc-jar.sha256".to_string(),
            ]);
            for artifact in crate::model::TRANSCRIPT_ARTIFACTS {
                add_artifact_keys(&mut expected, artifact.prefix);
            }
        }
        Kind::VmwareSelfTest | Kind::VmwareShipped => {
            add_tool_keys(&mut expected, "input.vmrun");
            expected.extend([
                "execution.vmrun.command".to_string(),
                "execution.vmrun.running".to_string(),
            ]);
            for prefix in [
                "artifact.generation",
                "artifact.image",
                "artifact.namespace",
                "artifact.vmdk",
                "artifact.namespace-vmdk",
                "artifact.vmx",
                "artifact.handoff",
                "artifact.transcript",
                "artifact.observer",
            ] {
                add_artifact_keys(&mut expected, prefix);
            }
        }
    }
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

fn add_tool_keys(keys: &mut BTreeSet<String>, prefix: &str) {
    keys.extend([
        format!("{prefix}.bytes"),
        format!("{prefix}.sha256"),
        format!("{prefix}.version"),
    ]);
}

fn add_artifact_keys(keys: &mut BTreeSet<String>, prefix: &str) {
    keys.extend([
        format!("{prefix}.bytes"),
        format!("{prefix}.path"),
        format!("{prefix}.sha256"),
    ]);
}

fn add_tool(record: &mut Record, prefix: &str, tool: &Tool) -> Result<()> {
    let version = tool.version.replace('\n', "\\n");
    ensure!(!version.is_empty(), "{prefix} has no version identity");
    record.insert(format!("{prefix}.version"), version)?;
    add_digest(record, prefix, &tool.path)
}

fn add_artifact(record: &mut Record, prefix: &str, name: &str, path: &Path) -> Result<()> {
    add_artifact_digest(record, prefix, name, &digest_file(path)?)
}

fn add_artifact_digest(
    record: &mut Record,
    prefix: &str,
    name: &str,
    digest: &Digest,
) -> Result<()> {
    record.insert(format!("{prefix}.path"), name)?;
    record.insert(format!("{prefix}.bytes"), digest.bytes.to_string())?;
    record.insert(format!("{prefix}.sha256"), &digest.sha256)
}

fn add_digest(record: &mut Record, prefix: &str, path: &Path) -> Result<()> {
    let digest = digest_file(path)?;
    record.insert(format!("{prefix}.bytes"), digest.bytes.to_string())?;
    record.insert(format!("{prefix}.sha256"), digest.sha256)?;
    Ok(())
}

fn digest_file(path: &Path) -> Result<Digest> {
    digest_file_bounded(path, MAX_DIGEST_FILE_BYTES)
}

fn digest_file_bounded(path: &Path, maximum: u64) -> Result<Digest> {
    digest_file_bounded_with(path, maximum, || Ok(()))
}

fn digest_file_bounded_with<F>(path: &Path, maximum: u64, after_measurement: F) -> Result<Digest>
where
    F: FnOnce() -> Result<()>,
{
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{} is {initial} bytes, exceeding its {maximum}-byte digest limit",
        path.display()
    );
    after_measurement()?;
    let (bytes, digest) = sha256::digest_reader((&mut file).take(maximum + 1))
        .with_context(|| format!("hashing {}", path.display()))?;
    ensure!(
        bytes <= maximum,
        "{} grew beyond its {maximum}-byte digest limit while hashing",
        path.display()
    );
    ensure!(
        bytes == initial
            && file
                .metadata()
                .with_context(|| format!("remeasuring {}", path.display()))?
                .len()
                == initial,
        "{} changed length while hashing",
        path.display()
    );
    Ok(Digest {
        bytes,
        sha256: sha256::hex(&digest),
    })
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    Digest {
        bytes: bytes.len() as u64,
        sha256: sha256::hex(&sha256::digest(bytes)),
    }
}

pub(crate) fn read_bounded_file(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    read_bounded_file_with(path, maximum, label, || Ok(()), || Ok(()))
}

fn read_bounded_file_with<E, F>(
    path: &Path,
    maximum: u64,
    label: &str,
    before_open: E,
    after_metadata: F,
) -> Result<Vec<u8>>
where
    E: FnOnce() -> Result<()>,
    F: FnOnce() -> Result<()>,
{
    let parent = path
        .parent()
        .with_context(|| format!("{label} has no parent directory"))?;
    crate::mutation_workspace::canonical_existing_real_directory(parent, label)?;
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    before_open()?;
    let mut file = crate::mutation_workspace::open_real_file(path, label)?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit"
    );
    after_metadata()?;
    let capacity = usize::try_from(initial).context("bounded file length does not fit usize")?;
    let mut bytes = Vec::with_capacity(capacity);
    (&mut file)
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {label} {}", path.display()))?;
    ensure!(
        bytes.len() as u64 <= maximum,
        "{label} grew beyond the {maximum}-byte allocation limit while being read"
    );
    ensure!(
        file.metadata()
            .with_context(|| format!("remeasuring {label} {}", path.display()))?
            .len()
            == bytes.len() as u64,
        "{label} changed length while being read"
    );
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    Ok(bytes)
}

pub(crate) fn read_bounded_text(path: &Path, maximum: u64, label: &str) -> Result<String> {
    String::from_utf8(read_bounded_file(path, maximum, label)?)
        .with_context(|| format!("{label} {} is not UTF-8", path.display()))
}

fn boot_image_bytes() -> u64 {
    crate::paths::DISK_MIB * 1024 * 1024
}

fn namespace_bytes() -> u64 {
    crate::qemu::NVME_VOLUME_SECTOR * crate::qemu::NVME_BLOCK_BYTES as u64
        + crate::image::corpus::VOLUME_BYTES as u64
}

fn capture_vmware_file(
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

fn capture_vmware_file_with<F>(
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
    let mut input = crate::mutation_workspace::open_real_file(source, label)?;
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

fn require_artifact_size(kind: Kind, name: &str, path: &Path) -> Result<()> {
    let length = File::open(path)
        .with_context(|| format!("opening qualification artifact {}", path.display()))?
        .metadata()
        .with_context(|| format!("measuring qualification artifact {}", path.display()))?
        .len();
    let (maximum, exact) = artifact_size_policy(kind, name)?;
    if exact {
        ensure!(
            length == maximum,
            "qualification artifact {name} is {length} bytes, expected exactly {maximum}"
        );
    } else {
        ensure!(
            length <= maximum,
            "qualification artifact {name} is {length} bytes, exceeding its {maximum}-byte limit"
        );
    }
    Ok(())
}

fn artifact_size_policy(kind: Kind, name: &str) -> Result<(u64, bool)> {
    Ok(match kind {
        Kind::Advisory => match name {
            ADVISORY_POLICY_PATH => (MAX_ADVISORY_POLICY_BYTES, false),
            ADVISORY_TRANSCRIPT => (MAX_ADVISORY_TRANSCRIPT_BYTES, false),
            ADVISORY_REFRESH_TRANSCRIPT => (MAX_ADVISORY_TRANSCRIPT_BYTES, false),
            _ => bail!("unregistered advisory artifact `{name}`"),
        },
        Kind::Mutation => (MAX_MUTATION_REPORT_BYTES, false),
        Kind::CargoMutantsGrade => match name {
            "request" | "doctest.transcript" => (MAX_EXTERNAL_RECEIPT_BYTES, false),
            "inventory.json" | "mutants.json" | "outcomes.json" => (MAX_MUTANTS_JSON_BYTES, false),
            "adjudications.json" => (MAX_MUTANTS_ADJUDICATION_BYTES, false),
            _ => bail!("unregistered cargo-mutants grade artifact `{name}`"),
        },
        Kind::CodexSecurityDeep => match name {
            "request" => (MAX_SECURITY_REQUEST_BYTES, false),
            "scan-manifest.json" => (MAX_SECURITY_MANIFEST_BYTES, false),
            "findings.json" => (MAX_SECURITY_FINDINGS_BYTES, false),
            "coverage.json" => (MAX_SECURITY_COVERAGE_BYTES, false),
            "report.md" => (MAX_SECURITY_REPORT_BYTES, false),
            "worker-transitions" => (MAX_SECURITY_TRANSITIONS_BYTES, false),
            _ => bail!("unregistered Codex Security artifact `{name}`"),
        },
        Kind::Model => (MAX_MODEL_TRANSCRIPT_BYTES, false),
        Kind::VmwareSelfTest | Kind::VmwareShipped => match name {
            VMWARE_IMAGE => (boot_image_bytes(), true),
            VMWARE_NAMESPACE => (namespace_bytes(), true),
            VMWARE_HANDOFF => (acpi_tables::DmaHandoff::LEN as u64, true),
            VMWARE_TRANSCRIPT => (MAX_VMWARE_TRANSCRIPT_BYTES, false),
            VMWARE_GENERATION_RECEIPT
            | VMWARE_VMDK
            | VMWARE_NAMESPACE_VMDK
            | VMWARE_VMX
            | VMWARE_OBSERVER => (MAX_VMWARE_TEXT_BYTES, false),
            _ => bail!("unregistered VMware artifact `{name}`"),
        },
    })
}

pub(crate) fn require_bundle_file_size(spec: &BundleFile, path: &Path) -> Result<()> {
    let (maximum, exact) = bundle_file_size_policy(spec)?;
    let length = File::open(path)
        .with_context(|| format!("opening qualification bundle file {}", path.display()))?
        .metadata()
        .with_context(|| format!("measuring qualification bundle file {}", path.display()))?
        .len();
    if exact {
        ensure!(
            length == maximum,
            "qualification bundle file {} is {length} bytes, expected exactly {maximum}",
            spec.relative
        );
    } else {
        ensure!(
            length <= maximum,
            "qualification bundle file {} is {length} bytes, exceeding its {maximum}-byte limit",
            spec.relative
        );
    }
    Ok(())
}

pub(crate) fn bundle_file_size_policy(spec: &BundleFile) -> Result<(u64, bool)> {
    ensure!(
        BUNDLE_FILES
            .iter()
            .any(|known| { known.prefix == spec.prefix && known.relative == spec.relative }),
        "unregistered qualification bundle file {}",
        spec.relative
    );
    if Path::new(spec.relative)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(RECORD)
    {
        return Ok((MAX_RECORD_BYTES, false));
    }

    let kind = if spec.prefix.starts_with("qualification.advisory.") {
        Kind::Advisory
    } else if spec.prefix.starts_with("qualification.mutation.") {
        Kind::Mutation
    } else if spec
        .prefix
        .starts_with("qualification.cargo-mutants-grade.")
    {
        Kind::CargoMutantsGrade
    } else if spec
        .prefix
        .starts_with("qualification.codex-security-deep.")
    {
        Kind::CodexSecurityDeep
    } else if spec.prefix.starts_with("qualification.model.") {
        Kind::Model
    } else if spec.prefix.starts_with("qualification.vmware-self-test.") {
        Kind::VmwareSelfTest
    } else if spec.prefix.starts_with("qualification.vmware-shipped.") {
        Kind::VmwareShipped
    } else {
        bail!("unknown qualification bundle class `{}`", spec.prefix);
    };
    let name = bundle_tail(spec)?
        .strip_prefix(kind.slug())
        .with_context(|| format!("{} escaped its qualification kind", spec.relative))?
        .to_str()
        .context("qualification bundle file name is not UTF-8")?;
    artifact_size_policy(kind, name)
}

fn copy_exact(source: &Path, destination: &Path, expected: &Digest) -> Result<()> {
    require_regular_file(source, "qualification source artifact")?;
    let mut input =
        crate::mutation_workspace::open_real_file(source, "qualification source artifact")?;
    let mut options = OpenOptions::new();
    OpenOptions::create_new(&mut options, true);
    OpenOptions::write(&mut options, true);
    let mut output = OpenOptions::open(&options, destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    let copied = std::io::copy(&mut (&mut input).take(expected.bytes + 1), &mut output)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    ensure!(
        copied == expected.bytes
            && input
                .metadata()
                .with_context(|| format!("remeasuring {}", source.display()))?
                .len()
                == expected.bytes,
        "{} changed length while entering qualification evidence",
        source.display()
    );
    output
        .sync_all()
        .with_context(|| format!("syncing {}", destination.display()))?;
    drop(output);
    ensure!(
        digest_file(destination)? == *expected,
        "{} changed while entering qualification evidence",
        source.display()
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeDigest {
    files: usize,
    bytes: u64,
    sha256: String,
}

fn tracked_tree_digest(root: &Path, directory: &str) -> Result<TreeDigest> {
    let output = git_output_bytes(
        root,
        &["ls-files", "-z", "--", directory],
        MAX_MODEL_PATH_LIST_BYTES,
        "tracked model path inventory",
    )?;
    let mut paths = Vec::new();
    for raw in output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let relative = std::str::from_utf8(raw).context("tracked model path is not UTF-8")?;
        let path = Path::new(relative);
        ensure!(
            path.starts_with(directory)
                && path
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))),
            "Git returned an unsafe model path: {relative}"
        );
        paths.push(relative.to_string());
    }
    ensure!(!paths.is_empty(), "no tracked model inputs were found");
    ensure!(
        paths.len() <= MAX_MODEL_FILES,
        "tracked model tree has {} files, exceeding its {MAX_MODEL_FILES}-file limit",
        paths.len()
    );
    ensure!(
        paths.windows(2).all(|pair| pair[0] < pair[1]),
        "tracked model inputs are not unique and sorted"
    );

    let mut hasher = sha256::Hasher::new();
    let mut total = 0u64;
    for relative in &paths {
        let path = root.join(relative);
        require_contained_regular_file(root, &path, "tracked model input")?;
        let mut file = crate::mutation_workspace::open_real_file(&path, "tracked model input")?;
        let length = file
            .metadata()
            .with_context(|| format!("measuring model input {relative}"))?
            .len();
        ensure!(
            length <= MAX_MODEL_FILE_BYTES,
            "model input {relative} is {length} bytes, exceeding its {MAX_MODEL_FILE_BYTES}-byte limit"
        );
        total = total
            .checked_add(length)
            .context("model input length overflowed")?;
        ensure!(
            total <= MAX_MODEL_TREE_BYTES,
            "tracked model tree is {total} bytes, exceeding its {MAX_MODEL_TREE_BYTES}-byte limit"
        );
        hasher
            .update(&(relative.len() as u64).to_le_bytes())
            .context("hashing a model path length")?;
        hasher
            .update(relative.as_bytes())
            .context("hashing a model path")?;
        hasher
            .update(&length.to_le_bytes())
            .context("hashing a model input length")?;

        let mut observed = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        let mut limited = (&mut file).take(length + 1);
        loop {
            let read = std::io::Read::read(&mut limited, &mut buffer)
                .with_context(|| format!("reading model input {relative}"))?;
            if read == 0 {
                break;
            }
            observed += read as u64;
            hasher
                .update(&buffer[..read])
                .with_context(|| format!("hashing model input {relative}"))?;
        }
        ensure!(
            observed == length
                && file
                    .metadata()
                    .with_context(|| format!("remeasuring model input {relative}"))?
                    .len()
                    == length,
            "model input {relative} changed length while being hashed"
        );
    }
    Ok(TreeDigest {
        files: paths.len(),
        bytes: total,
        sha256: sha256::hex(&hasher.finish()),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AdvisoryDatabaseIdentity {
    revision: String,
    tree: String,
}

fn advisory_database_identity(parent: &Path, database: &Path) -> Result<AdvisoryDatabaseIdentity> {
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

fn render_advisory_transcript(
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

fn render_advisory_transcript_with_limit(
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
struct AdvisoryAttestation {
    last_updated: String,
}

fn verify_advisory_transcripts(
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

fn verify_advisory_transcript_with_limit(
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
enum DatabaseBinding {
    /// The run fetched, so cargo-audit itself names the commit it audited.
    Fetched,
    /// The run was `--no-fetch`; cargo-audit learns the commit only from a
    /// fetch, so the report carries null identity and Git binds it (D-297).
    Pinned,
}

/// What one verified cargo-audit report says about its database.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AdvisoryFacts {
    advisory_count: u64,
    last_updated: Option<String>,
}

fn verify_advisory_report(
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

pub(crate) fn is_rfc3339_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || !value.is_ascii()
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    let Some(year) = decimal(&bytes[..4]) else {
        return false;
    };
    let Some(month) = decimal(&bytes[5..7]) else {
        return false;
    };
    let Some(day) = decimal(&bytes[8..10]) else {
        return false;
    };
    let Some(hour) = decimal(&bytes[11..13]) else {
        return false;
    };
    let Some(minute) = decimal(&bytes[14..16]) else {
        return false;
    };
    let Some(second) = decimal(&bytes[17..19]) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if year == 0 || day == 0 || day > days || hour > 23 || minute > 59 || second > 60 {
        return false;
    }

    let tail = &bytes[19..];
    let zone_at = if tail.first() == Some(&b'.') {
        let Some(zone_at) = tail
            .iter()
            .position(|byte| matches!(byte, b'Z' | b'+' | b'-'))
        else {
            return false;
        };
        if zone_at == 1 || !tail[1..zone_at].iter().all(u8::is_ascii_digit) {
            return false;
        }
        zone_at
    } else {
        0
    };
    let zone = &tail[zone_at..];
    if zone == b"Z" {
        return true;
    }
    zone.len() == 6
        && matches!(zone[0], b'+' | b'-')
        && zone[3] == b':'
        && decimal(&zone[1..3]).is_some_and(|hours| hours <= 23)
        && decimal(&zone[4..6]).is_some_and(|minutes| minutes <= 59)
}

fn decimal(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0u32, |value, byte| {
        byte.is_ascii_digit()
            .then(|| value * 10 + u32::from(*byte - b'0'))
    })
}

fn parse_json(bytes: &[u8]) -> Result<JsonValue> {
    crate::json::parse_json(bytes, "cargo-audit JSON")
}

fn json_array<'a>(value: &'a JsonValue, label: &str) -> Result<&'a [JsonValue]> {
    parse_json_array(value, &format!("cargo-audit `{label}`"))
}

fn json_string_array<'a>(value: &'a JsonValue, label: &str) -> Result<Vec<&'a str>> {
    parse_json_string_array(value, &format!("cargo-audit `{label}`"))
}

fn json_u64(value: &JsonValue, label: &str) -> Result<u64> {
    parse_json_u64(value, &format!("cargo-audit `{label}`"))
}

fn json_field<'a>(object: &'a BTreeMap<String, JsonValue>, name: &str) -> Result<&'a JsonValue> {
    parse_json_field(object, name, "cargo-audit JSON")
}

fn qualification_root(paths: &Paths) -> PathBuf {
    paths.target_dir().join("qualification")
}

fn kind_directory(paths: &Paths, kind: Kind) -> Result<PathBuf> {
    let root = qualification_root(paths);
    let directory = checked_kind_directory(&root, kind)?;
    require_contained_real_directory(&root, &directory, "qualification kind directory")?;
    Ok(directory)
}

fn checked_kind_directory(root: &Path, kind: Kind) -> Result<PathBuf> {
    let directory = root.join(kind.slug());
    ensure!(
        directory.parent() == Some(root),
        "qualification directory escaped {}",
        root.display()
    );
    Ok(directory)
}

fn bundle_tail(spec: &BundleFile) -> Result<&Path> {
    Path::new(spec.relative)
        .strip_prefix("qualification")
        .with_context(|| format!("invalid qualification bundle path {}", spec.relative))
}

fn validate_field(key: &str, value: &str) -> Result<()> {
    ensure!(
        !key.is_empty()
            && key.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte)
            }),
        "invalid qualification key `{key}`"
    );
    ensure!(
        !value.contains(['\r', '\n', '\0']),
        "qualification value for `{key}` contains a forbidden byte"
    );
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_git_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn git_output(root: &Path, args: &[&str], maximum: usize, label: &str) -> Result<String> {
    let output = git_output_bytes(root, args, maximum, label)?;
    String::from_utf8(output)
        .context("Git output is not UTF-8")
        .map(|output| output.trim().to_string())
}

fn git_output_bytes(root: &Path, args: &[&str], maximum: usize, label: &str) -> Result<Vec<u8>> {
    git_output_bytes_with(
        root,
        args,
        maximum,
        label,
        |command, timeout, label, limit| {
            crate::bounded_process::output_with_limit(command, timeout, label, limit)
        },
    )
}

fn git_output_bytes_with<F>(
    root: &Path,
    args: &[&str],
    maximum: usize,
    label: &str,
    run: F,
) -> Result<Vec<u8>>
where
    F: FnOnce(Command, Duration, &str, usize) -> Result<Output>,
{
    let mut command = crate::git::resolved()?.command(root);
    command.args(args);
    let command_label = format!("{label}: git {}", args.join(" "));
    let output = run(command, PROBE_TIMEOUT, &command_label, maximum)?;
    ensure!(
        output.status.success(),
        "{command_label} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    ensure!(
        output.stderr.iter().all(u8::is_ascii_whitespace),
        "{command_label} printed unexpected diagnostics"
    );
    ensure!(
        output.stdout.len() <= maximum,
        "{command_label} exceeded its {maximum}-byte semantic output limit"
    );
    Ok(output.stdout)
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

fn cargo_audit_version(report: &str) -> Result<String> {
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

/// The Java runtime TLC would execute, if one is available on `PATH`.
pub(crate) fn java(root: &Path) -> Result<Option<Tool>> {
    let Some(path) = find_on_path("java") else {
        return Ok(None);
    };
    let version = output(root, &path, &["-version"], ExitPolicy::Success)?;
    ensure!(
        !version.is_empty(),
        "{} printed no Java version",
        path.display()
    );
    Ok(Some(Tool { path, version }))
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

fn observe_running_vm_with<F>(tool: &Tool, expected_vmx: &Path, mut run: F) -> Result<String>
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

fn require_observed_vmx(observed: &Path, expected_vmx: &Path) -> Result<()> {
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

fn parse_vmrun_list(report: &str) -> Result<PathBuf> {
    ensure!(
        !report.contains(['\0', '\r']),
        "vmrun list contains a forbidden control byte"
    );
    let mut lines = report.lines();
    let count = core::iter::Iterator::next(&mut lines)
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

fn vmrun_version(report: &str) -> Result<String> {
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

fn find_on_path(stem: &str) -> Option<PathBuf> {
    let name = executable_name(stem);
    let path = std::env::var_os("PATH").unwrap_or_default();
    crate::qemu::which_in(std::env::split_paths(&path), &name)
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitPolicy {
    Success,
    Any,
}

fn output(root: &Path, program: &Path, args: &[&str], policy: ExitPolicy) -> Result<String> {
    output_with_timeout(root, program, args, policy, PROBE_TIMEOUT)
}

fn output_with_timeout(
    root: &Path,
    program: &Path,
    args: &[&str],
    policy: ExitPolicy,
    timeout: Duration,
) -> Result<String> {
    let mut command = Command::new(program);
    command.current_dir(root).args(args);
    let label = if args.is_empty() {
        program.display().to_string()
    } else {
        format!("{} {}", program.display(), args.join(" "))
    };
    let result = crate::bounded_process::output(command, timeout, &label)?;
    ensure!(
        policy == ExitPolicy::Any || result.status.success(),
        "{} {} failed ({})",
        program.display(),
        args.join(" "),
        result.status
    );
    let stdout = String::from_utf8(result.stdout)
        .with_context(|| format!("{} output is not UTF-8", program.display()))?;
    let stderr = String::from_utf8(result.stderr)
        .with_context(|| format!("{} diagnostic output is not UTF-8", program.display()))?;
    let mut combined = normalize(&stdout);
    let diagnostic = normalize(&stderr);
    if !diagnostic.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&diagnostic);
    }
    Ok(combined)
}

fn normalize(text: &str) -> String {
    text.trim().replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_git(record: &mut Record) {
        let path = std::env::current_dir().unwrap().join("git");
        for (key, value) in [
            ("tool.git.bytes", "1".to_string()),
            ("tool.git.path", path.to_string_lossy().into_owned()),
            ("tool.git.sha256", "f".repeat(64)),
            ("tool.git.version", "git version 2.44.0".to_string()),
        ] {
            record.insert(key, value).unwrap();
        }
    }

    fn complete_record(kind: Kind) -> Record {
        let mut record = Record::default();
        record.insert("kind", kind.slug()).unwrap();
        record.insert("result", PASS).unwrap();
        record.insert("source.commit", "a".repeat(40)).unwrap();
        record.insert("source.tree", "b".repeat(40)).unwrap();
        fixture_git(&mut record);
        match kind {
            Kind::Advisory => {
                fixture_tool(&mut record, "input.cargo-audit");
                record.fields.insert(
                    "input.cargo-audit.version".to_string(),
                    ADVISORY_TOOL_VERSION.to_string(),
                );
                record.insert("input.cargo-lock.bytes", "1").unwrap();
                record
                    .insert("input.cargo-lock.sha256", "c".repeat(64))
                    .unwrap();
                record.insert("database.revision", "d".repeat(40)).unwrap();
                record.insert("database.tree", "e".repeat(40)).unwrap();
                record
                    .insert("database.last-updated", "2026-08-30T12:00:00Z")
                    .unwrap();
                record
                    .insert("database.url", ADVISORY_DATABASE_URL)
                    .unwrap();
                record
                    .insert("execution.command", ADVISORY_COMMAND)
                    .unwrap();
                record.insert("execution.exit-code", "0").unwrap();
                record
                    .insert("execution.refresh-command", ADVISORY_REFRESH_COMMAND)
                    .unwrap();
                record.insert("execution.refresh-exit-code", "0").unwrap();
                record
                    .insert("observation.utc-unix-seconds", "1788048000")
                    .unwrap();
                fixture_artifact(&mut record, "artifact.policy", ADVISORY_POLICY_PATH);
                fixture_artifact(&mut record, "artifact.transcript", ADVISORY_TRANSCRIPT);
                fixture_artifact(
                    &mut record,
                    "artifact.refresh-transcript",
                    ADVISORY_REFRESH_TRANSCRIPT,
                );
            }
            Kind::Mutation => {
                record.insert("input.mutations", "1").unwrap();
                record
                    .insert("input.registry.sha256", "c".repeat(64))
                    .unwrap();
                record
                    .insert("input.source.sha256", "d".repeat(64))
                    .unwrap();
                record.insert("input.index.sha256", "e".repeat(64)).unwrap();
                fixture_artifact(&mut record, "artifact.report", MUTATION_REPORT);
            }
            Kind::CargoMutantsGrade => {
                for (key, value) in [
                    ("execution.authority", "external-operator-v1"),
                    ("grade.adjudicated", "0"),
                    ("grade.caught", "1"),
                    ("grade.missed", "0"),
                    ("grade.timeout", "0"),
                    ("grade.total", "1"),
                    ("grade.unviable", "0"),
                    ("tool.cargo-mutants.bytes", "1"),
                    (
                        "tool.cargo-mutants.sha256",
                        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    ),
                    ("tool.cargo-mutants.version", "27.1.0"),
                    (
                        "tool.cargo-nextest.archive.sha256",
                        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    ),
                    ("tool.cargo-nextest.bytes", "1"),
                    ("tool.cargo-nextest.commit", "e"),
                    (
                        "tool.cargo-nextest.sha256",
                        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    ),
                    ("tool.cargo-nextest.url", "https://example.invalid"),
                    ("tool.cargo-nextest.version", "0.9.145"),
                ] {
                    record.insert(key, value).unwrap();
                }
                for (prefix, name) in [
                    ("artifact.request", "request"),
                    ("artifact.inventory", "inventory.json"),
                    ("artifact.mutants", "mutants.json"),
                    ("artifact.outcomes", "outcomes.json"),
                    ("artifact.adjudications", "adjudications.json"),
                    ("artifact.doctest", "doctest.transcript"),
                ] {
                    fixture_artifact(&mut record, prefix, name);
                }
            }
            Kind::CodexSecurityDeep => {
                for (key, value) in [
                    ("authentication.channel", "codex-home-chatgpt"),
                    (
                        "environment.policy",
                        "xtask-owned-codex-security-hermetic-v9",
                    ),
                    ("execution.authority", "xtask-mcp-wrapper-windows-job-v8"),
                    ("execution.complete-tool", "complete_codex_security_scan"),
                    ("execution.model", "gpt-6.1-sol"),
                    ("execution.protocol", "2025-06-18"),
                    ("execution.reasoning", "xhigh"),
                    ("execution.server-name", "codex-security"),
                    ("execution.server-version", "0.1.160"),
                    ("execution.start-tool", "start_codex_security_deep_scan"),
                    ("execution.tools.bytes", "79316"),
                    ("execution.tools.count", "47"),
                    (
                        "execution.tools.sha256",
                        "514a38fefaaa3f89b1b897755721eca31b0fef5589c506102a696163e4add29a",
                    ),
                    (
                        "execution.worker-permission",
                        "provider-envelope=managed-root-read+explicit-deny;effective-worker=exec-only:backend=mxc,root=deny,minimal+owned-repository+scan+plugin+qualified-runtime=read,repository-metadata+credential=deny,outside-canary=default-deny-probed,network=false",
                    ),
                    (
                        "execution.worker-effective-profile.sha256",
                        "3333333333333333333333333333333333333333333333333333333333333333",
                    ),
                    (
                        "execution.worker-effective-state.sha256",
                        "4444444444444444444444444444444444444444444444444444444444444444",
                    ),
                    (
                        "execution.worker-parent-state.sha256",
                        "1111111111111111111111111111111111111111111111111111111111111111",
                    ),
                    (
                        "execution.worker-provider-profile.sha256",
                        "2222222222222222222222222222222222222222222222222222222222222222",
                    ),
                    ("execution.worker-preflight.allowed-reads", "7"),
                    ("execution.worker-preflight.denied-reads", "4"),
                    ("execution.worker-preflight.denied-write-creates", "2"),
                    ("execution.worker-preflight.denied-write-mutations", "3"),
                    ("execution.worker-preflight.denied-network-listeners", "1"),
                    ("execution.worker-preflight.denied-network-connections", "1"),
                    ("execution.worker-preflight.denied-descendants", "1"),
                    ("execution.worker-transitions.app-server", "1"),
                    ("execution.worker-transitions.exec-initial", "2"),
                    ("execution.worker-transitions.exec-resumed", "3"),
                    ("findings.critical", "0"),
                    ("findings.high", "0"),
                    ("findings.informational", "0"),
                    ("findings.low", "0"),
                    ("findings.medium", "0"),
                    ("findings.total", "0"),
                    ("scan.id", "scan-fixture"),
                    ("scan.producer.name", "codex-security-plugin"),
                    ("scan.producer.version", "0.1.32"),
                    ("scan.surfaces", "0"),
                    ("tool.codex-security.integrity.bytes", "17465"),
                    (
                        "tool.codex-security.integrity.sha256",
                        "6b342df66fb9b5ce7fc8c9de0e974b59d21340cdc06167df46d1a19bd4ea9441",
                    ),
                    ("tool.codex-security.tree.files", "142"),
                    ("tool.codex-security.version", "0.1.32"),
                    ("tool.codex.runtime.bytes", "442152128"),
                    ("tool.codex.runtime.files", "4"),
                    ("tool.codex.runtime.root", "parent-of-tool.codex"),
                    (
                        "tool.codex.runtime.sha256",
                        "72dd0efdd65e34488bc013f88166a8f549cf1f7441d0d23ad7116bc5758de1d5",
                    ),
                    ("tool.python.runtime.bytes", "1"),
                    ("tool.python.runtime.files", "1"),
                    ("tool.python.runtime.root", "parent-of-tool.python"),
                    (
                        "tool.python.runtime.sha256",
                        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    ),
                ] {
                    record.insert(key, value).unwrap();
                }
                for prefix in ["tool.node", "tool.python", "tool.codex"] {
                    fixture_tool(&mut record, prefix);
                }
                for (prefix, name) in [
                    ("artifact.request", "request"),
                    ("artifact.manifest", "scan-manifest.json"),
                    ("artifact.findings", "findings.json"),
                    ("artifact.coverage", "coverage.json"),
                    ("artifact.report", "report.md"),
                    ("artifact.worker-transitions", "worker-transitions"),
                ] {
                    fixture_artifact(&mut record, prefix, name);
                }
            }
            Kind::Model => {
                fixture_tool(&mut record, "input.java");
                record.insert("input.models.bytes", "1").unwrap();
                record.insert("input.models.files", "1").unwrap();
                record
                    .insert("input.models.sha256", "d".repeat(64))
                    .unwrap();
                record.insert("input.tlc-jar.bytes", "1").unwrap();
                record
                    .insert("input.tlc-jar.sha256", "e".repeat(64))
                    .unwrap();
                for artifact in crate::model::TRANSCRIPT_ARTIFACTS {
                    fixture_artifact(&mut record, artifact.prefix, artifact.name);
                }
            }
            Kind::VmwareSelfTest | Kind::VmwareShipped => {
                fixture_tool(&mut record, "input.vmrun");
                record
                    .insert("execution.vmrun.command", "-T ws list")
                    .unwrap();
                record.insert("execution.vmrun.running", "1").unwrap();
                for (prefix, name) in [
                    ("artifact.generation", VMWARE_GENERATION_RECEIPT),
                    ("artifact.image", VMWARE_IMAGE),
                    ("artifact.namespace", VMWARE_NAMESPACE),
                    ("artifact.vmdk", VMWARE_VMDK),
                    ("artifact.namespace-vmdk", VMWARE_NAMESPACE_VMDK),
                    ("artifact.vmx", VMWARE_VMX),
                    ("artifact.handoff", VMWARE_HANDOFF),
                    ("artifact.transcript", VMWARE_TRANSCRIPT),
                    ("artifact.observer", VMWARE_OBSERVER),
                ] {
                    fixture_artifact(&mut record, prefix, name);
                }
            }
        }
        record
    }

    #[test]
    #[ignore]
    fn slow_external_probe_child() {
        std::thread::sleep(Duration::from_secs(2));
    }

    #[test]
    fn an_external_qualification_probe_has_a_direct_child_return_deadline() {
        let executable = std::env::current_exe().expect("current test executable");
        let arguments = [
            "--exact",
            "qualification::tests::slow_external_probe_child",
            "--ignored",
        ];
        let started = Instant::now();
        let error = output_with_timeout(
            &std::env::current_dir().unwrap(),
            &executable,
            &arguments,
            ExitPolicy::Any,
            Duration::from_millis(100),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("timed out after 100 ms"));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    fn fixture_tool(record: &mut Record, prefix: &str) {
        record.insert(format!("{prefix}.bytes"), "1").unwrap();
        record
            .insert(format!("{prefix}.sha256"), "f".repeat(64))
            .unwrap();
        record
            .insert(format!("{prefix}.version"), "fixture 1")
            .unwrap();
    }

    fn fixture_artifact(record: &mut Record, prefix: &str, name: &str) {
        record.insert(format!("{prefix}.bytes"), "1").unwrap();
        record.insert(format!("{prefix}.path"), name).unwrap();
        record
            .insert(format!("{prefix}.sha256"), "0".repeat(64))
            .unwrap();
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .expect("running fixture Git");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn success_status() -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;

        std::process::ExitStatus::from_raw(0)
    }

    #[cfg(windows)]
    fn success_status() -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;

        std::process::ExitStatus::from_raw(0)
    }

    fn advisory_fixture(label: &str) -> (PathBuf, Paths, PathBuf, SourceIdentity) {
        let fixture =
            crate::test_support::reserve_temp_directory(&format!("wizard-advisory-{label}"));
        let repository = fixture.join("repository");
        let root = fixture.join("qualification");
        let directory = root.join(Kind::Advisory.slug());
        std::fs::create_dir_all(directory.join(".cargo")).unwrap();
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::write(repository.join("Cargo.lock"), "locked graph\n").unwrap();
        std::fs::write(directory.join(ADVISORY_POLICY_PATH), ADVISORY_POLICY).unwrap();
        let database_revision = "d".repeat(40);
        let output = Output {
            status: success_status(),
            stdout: clean_advisory_report(&database_revision),
            stderr: b"Fetching advisory database\n".to_vec(),
        };
        std::fs::write(
            directory.join(ADVISORY_REFRESH_TRANSCRIPT),
            render_advisory_transcript(&output, &database_revision, DatabaseBinding::Fetched)
                .unwrap(),
        )
        .unwrap();
        let pinned = Output {
            status: success_status(),
            stdout: pinned_advisory_report(),
            stderr: Vec::new(),
        };
        std::fs::write(
            directory.join(ADVISORY_TRANSCRIPT),
            render_advisory_transcript(&pinned, &database_revision, DatabaseBinding::Pinned)
                .unwrap(),
        )
        .unwrap();
        let mut record = complete_record(Kind::Advisory);
        let lock = digest_file(&repository.join("Cargo.lock")).unwrap();
        record
            .fields
            .insert("input.cargo-lock.bytes".to_string(), lock.bytes.to_string());
        record
            .fields
            .insert("input.cargo-lock.sha256".to_string(), lock.sha256);
        for (prefix, path) in [
            ("artifact.policy", directory.join(ADVISORY_POLICY_PATH)),
            (
                "artifact.refresh-transcript",
                directory.join(ADVISORY_REFRESH_TRANSCRIPT),
            ),
            ("artifact.transcript", directory.join(ADVISORY_TRANSCRIPT)),
        ] {
            let digest = digest_file(&path).unwrap();
            record
                .fields
                .insert(format!("{prefix}.bytes"), digest.bytes.to_string());
            record
                .fields
                .insert(format!("{prefix}.sha256"), digest.sha256);
        }
        std::fs::write(directory.join(RECORD), record.render()).unwrap();
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        (fixture, Paths::new(repository, None), root, source)
    }

    fn clean_advisory_report(database_revision: &str) -> Vec<u8> {
        format!(
            "{{\"database\":{{\"advisory-count\":1,\"last-commit\":\"{database_revision}\",\"last-updated\":\"2026-08-30T12:00:00Z\"}},\"lockfile\":{{\"dependency-count\":1}},\"settings\":{{\"target_arch\":[],\"target_os\":[],\"severity\":null,\"ignore\":[],\"informational_warnings\":[\"unmaintained\",\"unsound\",\"notice\"]}},\"vulnerabilities\":{{\"found\":false,\"count\":0,\"list\":[]}},\"warnings\":{{}}}}\n"
        )
        .into_bytes()
    }

    fn pinned_advisory_report() -> Vec<u8> {
        String::from_utf8(clean_advisory_report(&"d".repeat(40)))
            .unwrap()
            .replace(&format!("\"{}\"", "d".repeat(40)), "null")
            .replace("\"2026-08-30T12:00:00Z\"", "null")
            .into_bytes()
    }

    fn unchecked_advisory_transcript(stdout: &[u8]) -> Vec<u8> {
        let stderr = b"";
        let header = format!(
            "{ADVISORY_TRANSCRIPT_FORMAT}\nexit.code=0\nstderr.bytes={}\nstderr.sha256={}\nstdout.bytes={}\nstdout.sha256={}\n\n",
            stderr.len(),
            sha256::hex(&sha256::digest(stderr)),
            stdout.len(),
            sha256::hex(&sha256::digest(stdout)),
        );
        let mut transcript = header.into_bytes();
        transcript.extend_from_slice(stdout);
        transcript.extend_from_slice(stderr);
        transcript
    }

    fn rebind_advisory_artifact(root: &Path, prefix: &str, path: &Path) {
        let record_path = root.join(Kind::Advisory.slug()).join(RECORD);
        let mut record = read_record(&record_path, Kind::Advisory).unwrap();
        let digest = digest_file(path).unwrap();
        record
            .fields
            .insert(format!("{prefix}.bytes"), digest.bytes.to_string());
        record
            .fields
            .insert(format!("{prefix}.sha256"), digest.sha256);
        std::fs::write(record_path, record.render()).unwrap();
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating directory symlink");
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_dir(target, link).expect("creating directory symlink");
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating file symlink");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("creating file symlink");
    }

    fn write_snapshot_bundle(root: &Path) {
        for spec in BUNDLE_FILES {
            let path = root.join(bundle_tail(spec).unwrap());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut file = File::create(path).unwrap();
            file.write_all(spec.prefix.as_bytes()).unwrap();
            let (maximum, exact) = bundle_file_size_policy(spec).unwrap();
            if exact {
                file.set_len(maximum).unwrap();
            }
        }
    }

    #[test]
    fn a_qualification_record_is_canonical_and_checksummed() {
        for kind in [
            Kind::Advisory,
            Kind::Mutation,
            Kind::CargoMutantsGrade,
            Kind::CodexSecurityDeep,
            Kind::Model,
            Kind::VmwareSelfTest,
            Kind::VmwareShipped,
        ] {
            let record = complete_record(kind);
            validate_record(&record, kind).unwrap();
            let rendered = record.render();
            assert_eq!(parse_record(&rendered).unwrap(), record);

            let tampered = rendered.replace("result=pass", "result=fail");
            assert!(parse_record(&tampered).is_err());
            assert!(parse_record(rendered.trim_end()).is_err());
        }
    }

    #[test]
    fn online_and_offline_release_paths_share_the_owned_deep_verifier() {
        let implementation = include_str!("qualification.rs");
        let tests = implementation.find("\n#[cfg(test)]\nmod tests {").unwrap();
        let production = &implementation[..tests];
        let online = implementation
            .find("pub(crate) fn recorded_source(paths: &Paths)")
            .unwrap();
        let offline = implementation.find("fn verify_records(").unwrap();
        let implementation_end = implementation
            .find("fn verify_advisory_qualification(")
            .unwrap();
        let guard = "    if !campaigns.deferred(Kind::CodexSecurityDeep) {\n        ";
        assert!(implementation[online..offline].contains(&format!(
            "{guard}require_authenticated_deep_qualification(paths, &root, &source)?;\n    }}\n"
        )));
        assert!(
            implementation[offline..implementation_end].contains(&format!(
                "{guard}require_authenticated_deep_qualification(paths, root, source)?;\n    }}\n"
            ))
        );
        assert_eq!(
            production
                .matches("require_authenticated_deep_qualification(")
                .count(),
            3
        );
        assert_eq!(
            production
                .matches("fn require_authenticated_deep_qualification(")
                .count(),
            1
        );
    }

    #[test]
    fn a_deferred_campaign_carrying_a_record_is_refused() {
        let root =
            crate::test_support::reserve_temp_directory("wizard-qualification-deferred-record");
        let paths = Paths::new(root.clone(), None);
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let deferred = CampaignStates::from_labels([DEFERRED, DEFERRED]).unwrap();
        let grade = root.join(Kind::CargoMutantsGrade.slug());
        std::fs::create_dir_all(&grade).unwrap();
        assert_eq!(CampaignStates::observe(&root).unwrap(), deferred);
        require_deferred(&root, Kind::CargoMutantsGrade).unwrap();

        std::fs::write(grade.join(RECORD), "stale grade record\n").unwrap();
        assert_eq!(
            CampaignStates::observe(&root).unwrap(),
            CampaignStates::from_labels([PASS, DEFERRED]).unwrap()
        );
        let error = verify_records_after_authority(
            &paths,
            &root,
            &source,
            &"c".repeat(64),
            1,
            None,
            &deferred,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("deferred cargo-mutants-grade campaign carries"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_a_deferred_campaigns_files_leave_the_bundle() {
        let relatives = |labels| {
            CampaignStates::from_labels(labels)
                .unwrap()
                .bundle_files()
                .iter()
                .map(|spec| spec.relative)
                .collect::<Vec<_>>()
        };
        let without = |slugs: &[&str]| {
            BUNDLE_FILES
                .iter()
                .map(|spec| spec.relative)
                .filter(|relative| {
                    !slugs
                        .iter()
                        .any(|slug| relative.starts_with(&format!("qualification/{slug}/")))
                })
                .collect::<Vec<_>>()
        };
        let grade = "cargo-mutants-grade";
        let deep = "codex-security-deep";
        assert_eq!(relatives([PASS, PASS]), without(&[]));
        assert_eq!(relatives([PASS, PASS]).len(), BUNDLE_FILES.len());
        assert_eq!(relatives([DEFERRED, PASS]), without(&[grade]));
        assert_eq!(relatives([PASS, DEFERRED]), without(&[deep]));
        assert_eq!(relatives([DEFERRED, DEFERRED]), without(&[grade, deep]));
        assert_eq!(
            relatives([DEFERRED, DEFERRED]).len(),
            BUNDLE_FILES.len() - 14
        );
    }

    #[test]
    fn only_the_grade_and_deep_campaigns_are_deferrable() {
        assert_eq!(
            CampaignStates::status_keys(),
            [
                "qualification.cargo-mutants-grade.status".to_string(),
                "qualification.codex-security-deep.status".to_string(),
            ]
        );
        let passed = CampaignStates::from_labels([PASS, PASS]).unwrap();
        let deferred = CampaignStates::from_labels([DEFERRED, DEFERRED]).unwrap();
        for kind in [Kind::CargoMutantsGrade, Kind::CodexSecurityDeep] {
            assert!(deferred.deferred(kind));
            assert!(!passed.deferred(kind));
        }
        for kind in [
            Kind::Mutation,
            Kind::Advisory,
            Kind::Model,
            Kind::VmwareSelfTest,
            Kind::VmwareShipped,
        ] {
            assert!(!deferred.deferred(kind));
        }
    }

    #[test]
    fn campaign_status_is_exactly_pass_or_deferred() {
        for labels in [
            [PASS, PASS],
            [PASS, DEFERRED],
            [DEFERRED, PASS],
            [DEFERRED, DEFERRED],
        ] {
            assert_eq!(
                CampaignStates::from_labels(labels).unwrap().labels(),
                labels
            );
        }
        for invalid in ["", "fail", "Pass", "DEFERRED", "pass ", "skipped"] {
            assert!(
                CampaignStates::from_labels([invalid, PASS]).is_err(),
                "{invalid:?}"
            );
            assert!(
                CampaignStates::from_labels([DEFERRED, invalid]).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn deep_and_mutants_attempts_are_separate_capability_types() {
        let implementation = include_str!("qualification.rs");
        let deep = implementation.find("impl DeepAttempt {").unwrap();
        let owned = implementation.find("impl OwnedDeepAttempt {").unwrap();
        let mutants = implementation.find("impl MutantsAttempt {").unwrap();
        let constants = implementation[mutants..]
            .find("pub(crate) const BUNDLE_FILES")
            .unwrap()
            + mutants;
        assert!(!implementation[deep..owned].contains("pass_observed_deep"));
        assert!(implementation[owned..mutants].contains("pass_observed_deep"));
        assert!(
            implementation[owned..mutants]
                .contains("_authority: crate::security_deep::DeepAuthority")
        );
        assert!(!implementation[deep..mutants].contains("pass_observed_mutants"));
        assert!(implementation[mutants..constants].contains("pass_observed_mutants"));
        let begin_owned = implementation
            .find("pub(crate) fn begin_owned_security_deep(")
            .unwrap();
        let begin_mutants = implementation[begin_owned..]
            .find("pub(crate) fn begin_mutants_grade(")
            .unwrap()
            + begin_owned;
        assert!(
            implementation[begin_owned..begin_mutants]
                .contains("_authority: &crate::security_deep::DeepAuthority")
        );

        let security = include_str!("security_deep.rs");
        let security_tests = security.find("#[cfg(test)]").unwrap();
        assert_eq!(
            security[..security_tests]
                .matches("DeepAuthority::new()")
                .count(),
            1
        );
        let token = security
            .find("#[derive(Debug)]\npub(crate) struct DeepAuthority {")
            .unwrap();
        let token_end = security[token..]
            .find("\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
            .unwrap()
            + token;
        let token_definition = &security[token..token_end];
        assert!(token_definition.contains("\n    _private: (),"));
        assert!(token_definition.contains("\n    fn new() -> Self"));
        assert!(!token_definition.contains("Clone"));

        let run = security.find("fn run_owned(paths: &Paths)").unwrap();
        let after_run = security[run..].find("\nfn begin(").unwrap() + run;
        let run_body = &security[run..after_run];
        assert_eq!(run_body.matches("DeepAuthority::new()").count(), 1);
        assert_eq!(
            run_body
                .matches("qualification::begin_owned_security_deep(paths, &authority)")
                .count(),
            1
        );
        assert_eq!(
            run_body
                .matches(".pass_observed_deep(authority, &fields, artifacts)")
                .count(),
            1
        );
    }

    #[test]
    fn owned_deep_replays_copied_artifacts_before_committing_pass() {
        let implementation = include_str!("qualification.rs");
        let owned = implementation.find("impl OwnedDeepAttempt {").unwrap();
        let end = implementation[owned..]
            .find("impl MutantsAttempt {")
            .unwrap()
            + owned;
        let body = &implementation[owned..end];

        let copy = body.find("write_new_file(&path, bytes)?;").unwrap();
        let replay = body.find("crate::security_deep::verify_sealed(").unwrap();
        let compare = body
            .find("require_external_fields(&record, &derived)?;")
            .unwrap();
        let commit = body
            .find("commit_record(&self.paths, Kind::CodexSecurityDeep, &record)")
            .unwrap();
        assert!(copy < replay && replay < compare && compare < commit);
        assert_eq!(
            body.matches("crate::security_deep::verify_sealed(").count(),
            1
        );
        assert_eq!(
            body.matches("commit_record(&self.paths, Kind::CodexSecurityDeep, &record)")
                .count(),
            1
        );
        assert_eq!(body.matches("\"artifact.worker-transitions\"").count(), 1);
        let support = implementation.find("fn verify_support(").unwrap();
        let support_end = implementation[support..]
            .find("fn require_external_fields")
            .unwrap()
            + support;
        assert!(
            implementation[support..support_end]
                .contains("(\"artifact.worker-transitions\", \"worker-transitions\")")
        );
    }

    #[test]
    fn owned_deep_cleans_scratch_before_minting_pass() {
        let security = include_str!("security_deep.rs");
        let run = security.find("fn run_owned(paths: &Paths)").unwrap();
        let end = security[run..].find("\nfn reserve_scratch(").unwrap() + run;
        let body = &security[run..end];
        let transaction = body.find("finalize_owned_transaction(").unwrap();
        let cleanup = body.find("|| remove_scratch(&scratch)").unwrap();
        let pass = body
            .find("|| attempt.pass_observed_deep(authority, &fields, artifacts)")
            .unwrap();
        assert!(
            transaction < cleanup && cleanup < pass,
            "PASS preceded the fallible scratch-cleanup transaction"
        );
    }

    #[test]
    fn qualification_schema_rejects_omission_and_invention() {
        let mut record = complete_record(Kind::Mutation);
        record.fields.remove("artifact.report.sha256");
        assert!(validate_record(&record, Kind::Mutation).is_err());

        let mut record = complete_record(Kind::Model);
        record.fields.remove("artifact.run.ring-clean.sha256");
        assert!(validate_record(&record, Kind::Model).is_err());

        let mut record = complete_record(Kind::Model);
        record.insert("result.claimed-by-prose", PASS).unwrap();
        assert!(validate_record(&record, Kind::Model).is_err());

        let mut record = complete_record(Kind::CodexSecurityDeep);
        record.fields.remove("tool.python.runtime.sha256");
        assert!(validate_record(&record, Kind::CodexSecurityDeep).is_err());

        for field in [
            "tool.codex.runtime.bytes",
            "tool.codex.runtime.files",
            "tool.codex.runtime.root",
            "tool.codex.runtime.sha256",
        ] {
            let mut record = complete_record(Kind::CodexSecurityDeep);
            record.fields.remove(field);
            assert!(
                validate_record(&record, Kind::CodexSecurityDeep).is_err(),
                "missing Codex runtime field {field} was accepted"
            );
        }

        let complete = complete_record(Kind::CodexSecurityDeep);
        validate_record(&complete, Kind::CodexSecurityDeep).unwrap();
        for field in [
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
        ] {
            let mut record = complete.clone();
            record.fields.remove(field);
            assert!(
                validate_record(&record, Kind::CodexSecurityDeep).is_err(),
                "missing worker authority field {field} was accepted"
            );
        }

        let mut record = complete_record(Kind::CodexSecurityDeep);
        record
            .insert("execution.claimed-by-provider", PASS)
            .unwrap();
        assert!(validate_record(&record, Kind::CodexSecurityDeep).is_err());
    }

    #[test]
    fn every_forged_owned_deep_authority_field_is_rejected() {
        let record = complete_record(Kind::CodexSecurityDeep);
        let expected: Vec<(String, String)> = record
            .fields
            .iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "authentication.channel" | "environment.policy"
                ) || key.starts_with("execution.")
                    || key.starts_with("findings.")
                    || key.starts_with("scan.")
                    || key.starts_with("tool.codex-security.")
                    || key.starts_with("tool.node.")
                    || key.starts_with("tool.python.")
                    || key.starts_with("tool.codex.")
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        assert_eq!(expected.len(), 59);
        require_external_fields(&record, &expected).unwrap();

        for (key, _) in &expected {
            let mut forged = record.clone();
            forged.fields.insert(key.clone(), "forged".to_string());
            assert!(
                require_external_fields(&forged, &expected).is_err(),
                "forged owned Deep field `{key}` was accepted"
            );
        }
    }

    #[test]
    fn qualification_v5_is_rejected_even_with_its_own_valid_checksum() {
        let rendered = complete_record(Kind::Mutation).render();
        let fields_start = rendered.find('\n').unwrap() + 1;
        let checksum_start = rendered.rfind("checksum.sha256=").unwrap();
        let body = format!(
            "wizard-qualification-v5\n{}",
            &rendered[fields_start..checksum_start]
        );
        let old = format!(
            "{body}checksum.sha256={}\n",
            sha256::hex(&sha256::digest(body.as_bytes()))
        );

        let error = parse_record(&old).unwrap_err().to_string();
        assert!(error.contains("no supported header"), "{error}");
    }

    #[test]
    fn advisory_schema_binds_database_time_command_and_tool_identity() {
        for field in ["database", "database-time", "time", "command", "tool"] {
            let mut record = complete_record(Kind::Advisory);
            match field {
                "database" => {
                    record
                        .fields
                        .insert("database.revision".to_string(), "forged".to_string());
                }
                "database-time" => {
                    record.fields.insert(
                        "database.last-updated".to_string(),
                        "not-a-time".to_string(),
                    );
                }
                "time" => {
                    record
                        .fields
                        .insert("observation.utc-unix-seconds".to_string(), "0".to_string());
                }
                "command" => {
                    record.fields.insert(
                        "execution.command".to_string(),
                        "cargo-audit audit --stale".to_string(),
                    );
                }
                "tool" => {
                    record.fields.insert(
                        "input.cargo-audit.version".to_string(),
                        "cargo 1.97.0".to_string(),
                    );
                }
                _ => unreachable!(),
            }
            assert!(
                validate_record(&record, Kind::Advisory).is_err(),
                "forged advisory {field} identity was accepted"
            );
        }
    }

    #[test]
    fn advisory_database_identity_rejects_forged_origin_and_dirty_bytes() {
        let fixture = crate::test_support::reserve_temp_directory("wizard-advisory-database");
        let parent = fixture.join("advisory");
        let database = parent.join(ADVISORY_DATABASE);
        std::fs::create_dir_all(&database).unwrap();
        run_git(&database, &["init", "--quiet"]);
        std::fs::write(database.join("advisory.toml"), "[advisory]\n").unwrap();
        run_git(&database, &["add", "advisory.toml"]);
        run_git(
            &database,
            &[
                "-c",
                "user.name=Wizard Test",
                "-c",
                "user.email=wizard@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
        run_git(
            &database,
            &["remote", "add", "origin", ADVISORY_DATABASE_URL],
        );
        advisory_database_identity(&parent, &database).unwrap();

        run_git(
            &database,
            &[
                "remote",
                "set-url",
                "origin",
                "https://example.invalid/forged.git",
            ],
        );
        assert!(advisory_database_identity(&parent, &database).is_err());
        run_git(
            &database,
            &["remote", "set-url", "origin", ADVISORY_DATABASE_URL],
        );
        std::fs::write(database.join("advisory.toml"), "forged\n").unwrap();
        assert!(advisory_database_identity(&parent, &database).is_err());
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn a_missing_advisory_record_is_rejected() {
        let (fixture, paths, root, source) = advisory_fixture("missing");
        std::fs::remove_file(root.join(Kind::Advisory.slug()).join(RECORD)).unwrap();

        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();

        assert!(error.contains("qualification record"), "{error}");
        let implementation = include_str!("qualification.rs");
        let verification = implementation.find("fn verify_records(").unwrap();
        let advisory = implementation[verification..]
            .find("verify_advisory_qualification(paths, root, source)?;")
            .unwrap();
        let loop_start = implementation[verification..]
            .find("for kind in [")
            .unwrap();
        assert!(advisory < loop_start);
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn a_stale_advisory_record_cannot_describe_another_lockfile() {
        let (fixture, paths, root, source) = advisory_fixture("stale");
        verify_advisory_qualification(&paths, &root, &source).unwrap();
        std::fs::write(paths.root().join("Cargo.lock"), "changed locked graph\n").unwrap();

        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();

        assert!(error.contains("another Cargo.lock"), "{error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn forged_advisory_policy_and_result_bytes_are_rejected_semantically() {
        let (fixture, paths, root, source) = advisory_fixture("forged");
        let directory = root.join(Kind::Advisory.slug());
        let transcript_path = directory.join(ADVISORY_TRANSCRIPT);
        let valid = std::fs::read(&transcript_path).unwrap();
        let mut forged = valid.clone();
        let result = forged
            .windows(b"exit.code=0".len())
            .position(|window| window == b"exit.code=0")
            .unwrap();
        forged[result + b"exit.code=".len()] = b'1';
        std::fs::write(&transcript_path, forged).unwrap();
        rebind_advisory_artifact(&root, "artifact.transcript", &transcript_path);
        assert!(verify_advisory_qualification(&paths, &root, &source).is_err());

        std::fs::write(&transcript_path, valid).unwrap();
        rebind_advisory_artifact(&root, "artifact.transcript", &transcript_path);
        let policy = directory.join(ADVISORY_POLICY_PATH);
        std::fs::write(&policy, "[advisories]\nignore = [\"RUSTSEC-2099-0001\"]\n").unwrap();
        rebind_advisory_artifact(&root, "artifact.policy", &policy);
        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no-ignore"), "{error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn forged_green_advisory_json_is_rejected_semantically() {
        let (fixture, paths, root, source) = advisory_fixture("forged-json");
        let transcript = root.join(Kind::Advisory.slug()).join(ADVISORY_TRANSCRIPT);
        let revision = "d".repeat(40);
        let clean = String::from_utf8(clean_advisory_report(&revision)).unwrap();
        let hostile = [
            ("empty object", "{}\n".to_string()),
            (
                "unknown top-level field",
                clean.replace("\"warnings\":{}", "\"warnings\":{},\"forged\":true"),
            ),
            (
                "unknown database field",
                clean.replace(
                    "\"advisory-count\":1",
                    "\"advisory-count\":1,\"forged\":true",
                ),
            ),
            (
                "empty advisory database",
                clean.replace("\"advisory-count\":1", "\"advisory-count\":0"),
            ),
            (
                "malformed database time",
                clean.replace("2026-08-30T12:00:00Z", "not-a-time"),
            ),
            (
                "unknown lockfile field",
                clean.replace(
                    "\"dependency-count\":1",
                    "\"dependency-count\":1,\"forged\":true",
                ),
            ),
            (
                "empty lockfile",
                clean.replace("\"dependency-count\":1", "\"dependency-count\":0"),
            ),
            (
                "unknown settings field",
                clean.replace("\"target_arch\":[]", "\"target_arch\":[],\"forged\":true"),
            ),
            (
                "vulnerability found",
                clean.replace("\"found\":false", "\"found\":true"),
            ),
            (
                "unknown vulnerability field",
                clean.replace("\"found\":false", "\"found\":false,\"forged\":true"),
            ),
            (
                "inconsistent vulnerability count",
                clean.replace("\"count\":0,\"list\":[]", "\"count\":1,\"list\":[]"),
            ),
            (
                "dependency warning",
                clean.replace("\"warnings\":{}", "\"warnings\":{\"unmaintained\":[{}]}"),
            ),
            (
                "ignored advisory setting",
                clean.replace("\"ignore\":[]", "\"ignore\":[\"RUSTSEC-2099-0001\"]"),
            ),
            (
                "different database revision",
                clean.replace(&revision, &"e".repeat(40)),
            ),
        ];

        for (label, report) in hostile {
            std::fs::write(
                &transcript,
                unchecked_advisory_transcript(report.as_bytes()),
            )
            .unwrap();
            rebind_advisory_artifact(&root, "artifact.transcript", &transcript);
            assert!(
                verify_advisory_qualification(&paths, &root, &source).is_err(),
                "forged {label} report was accepted"
            );
        }
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn an_empty_advisory_database_cannot_author_a_qualification() {
        let revision = "d".repeat(40);
        let report = String::from_utf8(clean_advisory_report(&revision))
            .unwrap()
            .replace("\"advisory-count\":1", "\"advisory-count\":0");

        let error = verify_advisory_report(report.as_bytes(), &revision, DatabaseBinding::Fetched)
            .unwrap_err()
            .to_string();

        assert!(error.contains("empty advisory database"), "{error}");
    }

    #[test]
    fn a_pinned_audit_report_carries_null_database_identity_by_tool_design() {
        let revision = "d".repeat(40);
        let facts = verify_advisory_report(
            &pinned_advisory_report(),
            &revision,
            DatabaseBinding::Pinned,
        )
        .unwrap();
        assert_eq!(facts.advisory_count, 1);
        assert_eq!(facts.last_updated, None);
        let fetched = verify_advisory_report(
            &clean_advisory_report(&revision),
            &revision,
            DatabaseBinding::Fetched,
        )
        .unwrap();
        assert_eq!(
            fetched.last_updated.as_deref(),
            Some("2026-08-30T12:00:00Z")
        );
        let error = verify_advisory_report(
            &pinned_advisory_report(),
            &revision,
            DatabaseBinding::Fetched,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("no usable advisory database identity"),
            "{error}"
        );
    }

    #[test]
    fn a_population_transcript_without_database_identity_is_rejected() {
        let (fixture, paths, root, source) = advisory_fixture("null-population");
        let refresh = root
            .join(Kind::Advisory.slug())
            .join(ADVISORY_REFRESH_TRANSCRIPT);
        std::fs::write(
            &refresh,
            unchecked_advisory_transcript(&pinned_advisory_report()),
        )
        .unwrap();
        rebind_advisory_artifact(&root, "artifact.refresh-transcript", &refresh);
        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no usable advisory database identity"),
            "{error}"
        );
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn the_pinned_audit_must_see_the_population_runs_database() {
        let (fixture, paths, root, source) = advisory_fixture("count-drift");
        let refresh = root
            .join(Kind::Advisory.slug())
            .join(ADVISORY_REFRESH_TRANSCRIPT);
        let drifted = String::from_utf8(clean_advisory_report(&"d".repeat(40)))
            .unwrap()
            .replace("\"advisory-count\":1", "\"advisory-count\":2");
        std::fs::write(&refresh, unchecked_advisory_transcript(drifted.as_bytes())).unwrap();
        rebind_advisory_artifact(&root, "artifact.refresh-transcript", &refresh);
        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();
        assert!(error.contains("different advisory databases"), "{error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn an_advisory_record_must_repeat_the_attested_database_update_time() {
        let (fixture, paths, root, source) = advisory_fixture("update-time");
        let record_path = root.join(Kind::Advisory.slug()).join(RECORD);
        let mut record = read_record(&record_path, Kind::Advisory).unwrap();
        record.fields.insert(
            "database.last-updated".to_string(),
            "2026-08-31T12:00:00Z".to_string(),
        );
        std::fs::write(&record_path, record.render()).unwrap();
        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();
        assert!(error.contains("another database update time"), "{error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn advisory_transcript_rendering_refuses_output_beyond_its_ceiling() {
        let revision = "d".repeat(40);
        let output = Output {
            status: success_status(),
            stdout: clean_advisory_report(&revision),
            stderr: Vec::new(),
        };
        let error =
            render_advisory_transcript_with_limit(&output, &revision, DatabaseBinding::Fetched, 1)
                .unwrap_err()
                .to_string();

        assert!(
            error.contains("cargo-audit transcript exceeds its bounded artifact limit"),
            "{error}"
        );
    }

    #[test]
    fn advisory_transcript_verification_refuses_bytes_beyond_its_ceiling() {
        let revision = "d".repeat(40);
        let output = Output {
            status: success_status(),
            stdout: clean_advisory_report(&revision),
            stderr: Vec::new(),
        };
        let transcript =
            render_advisory_transcript(&output, &revision, DatabaseBinding::Fetched).unwrap();
        let error = verify_advisory_transcript_with_limit(
            &transcript,
            &revision,
            DatabaseBinding::Fetched,
            transcript.len() as u64 - 1,
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("cargo-audit transcript exceeds its bounded artifact limit"),
            "{error}"
        );
    }

    #[test]
    fn an_oversized_advisory_transcript_is_rejected_before_reading() {
        let (fixture, paths, root, source) = advisory_fixture("oversized");
        let transcript = root.join(Kind::Advisory.slug()).join(ADVISORY_TRANSCRIPT);
        OpenOptions::open(OpenOptions::write(&mut File::options(), true), &transcript)
            .unwrap()
            .set_len(MAX_ADVISORY_TRANSCRIPT_BYTES + 1)
            .unwrap();

        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();

        assert!(error.contains("exceeding its"), "{error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn an_advisory_record_bound_to_another_source_is_rejected() {
        let (fixture, paths, root, _) = advisory_fixture("source");
        let source = SourceIdentity {
            commit: "9".repeat(40),
            tree: "8".repeat(40),
        };

        let error = verify_advisory_qualification(&paths, &root, &source)
            .unwrap_err()
            .to_string();

        assert!(error.contains("another commit"), "{error}");
        std::fs::remove_dir_all(fixture).unwrap();
    }

    #[test]
    fn semantic_artifacts_are_bounded_before_and_during_allocation() {
        let root = crate::test_support::reserve_temp_directory("wizard-bounded-artifact");
        let path = root.join("artifact");
        let file = File::create(&path).unwrap();
        file.set_len(33).unwrap();
        drop(file);
        let error = read_bounded_file(&path, 32, "fixture artifact")
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeding the 32-byte allocation limit"));

        std::fs::write(&path, b"x").unwrap();
        let changed = path.clone();
        let error = read_bounded_file_with(
            &path,
            32,
            "fixture artifact",
            || Ok(()),
            move || {
                OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), changed)?
                    .set_len(33)?;
                Ok(())
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("grew beyond the 32-byte allocation limit"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_artifact_swapped_for_a_link_after_its_check_is_never_read() {
        let root = crate::test_support::reserve_temp_directory("wizard-swapped-artifact");
        let path = root.join("report.md");
        let parked = root.join("parked");
        let secret = root.join("secret");
        std::fs::write(&path, b"producer report").unwrap();
        std::fs::write(&secret, b"operator secret").unwrap();
        let (link, target, original, restored) =
            (path.clone(), secret.clone(), path.clone(), parked.clone());
        let result = read_bounded_file_with(
            &path,
            64,
            "fixture report",
            move || {
                std::fs::rename(&link, &parked)?;
                create_file_symlink(&target, &link);
                Ok(())
            },
            move || {
                std::fs::remove_file(&original)?;
                std::fs::rename(&restored, &original)?;
                Ok(())
            },
        );
        let error = result
            .expect_err("a link swapped in after the path check was read")
            .to_string();
        assert!(error.contains("is not a real file"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_qualification_digests_have_a_streaming_ceiling() {
        let root = crate::test_support::reserve_temp_directory("wizard-bounded-digest");
        let path = root.join("artifact");
        File::create(&path).unwrap().set_len(33).unwrap();

        let error = digest_file_bounded(&path, 32).unwrap_err().to_string();

        assert!(
            error.contains("exceeding its 32-byte digest limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_qualification_digest_growth_is_refused_after_measurement() {
        let root = crate::test_support::reserve_temp_directory("wizard-growing-digest");
        let path = root.join("artifact");
        std::fs::write(&path, b"x").unwrap();
        let growing = path.clone();

        let error = digest_file_bounded_with(&path, 32, move || {
            OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), growing)?
                .set_len(33)?;
            Ok(())
        })
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("grew beyond its 32-byte digest limit while hashing"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_qualification_digest_truncation_is_refused_after_measurement() {
        let root = crate::test_support::reserve_temp_directory("wizard-truncated-digest");
        let path = root.join("artifact");
        std::fs::write(&path, b"1234").unwrap();
        let shrinking = path.clone();

        let error = digest_file_bounded_with(&path, 4, move || {
            OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), shrinking)?
                .set_len(3)?;
            Ok(())
        })
        .unwrap_err()
        .to_string();

        assert!(error.contains("changed length while hashing"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_oversized_qualification_record_is_refused_before_parsing() {
        let root =
            crate::test_support::reserve_temp_directory("wizard-oversized-qualification-record");
        let path = root.join("record");
        File::create(&path)
            .unwrap()
            .set_len(MAX_RECORD_BYTES + 1)
            .unwrap();

        let error = read_record(&path, Kind::Mutation).unwrap_err().to_string();

        assert!(
            error.contains("exceeding the 262144-byte allocation limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_oversized_generation_receipt_is_refused_before_parsing() {
        let root =
            crate::test_support::reserve_temp_directory("wizard-oversized-generation-record");
        let path = root.join(VMWARE_GENERATION_RECEIPT);
        File::create(&path)
            .unwrap()
            .set_len(MAX_VMWARE_TEXT_BYTES + 1)
            .unwrap();

        let error = read_generation_record(&path, Kind::VmwareSelfTest)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("exceeding the 1048576-byte allocation limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tracked_model_files_are_size_checked_before_streaming() {
        let root = crate::test_support::reserve_temp_directory("wizard-model-input-bound");
        let models = root.join("models");
        std::fs::create_dir_all(&models).unwrap();
        run_git(&root, &["init", "--quiet"]);
        let oversized = models.join("oversized.tla");
        std::fs::write(&oversized, b"tracked").unwrap();
        run_git(&root, &["add", "models/oversized.tla"]);
        OpenOptions::open(OpenOptions::write(&mut File::options(), true), &oversized)
            .unwrap()
            .set_len(MAX_MODEL_FILE_BYTES + 1)
            .unwrap();

        let error = tracked_tree_digest(&root, "models")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("exceeding its 1048576-byte limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tracked_model_tree_has_an_independent_total_size_bound() {
        let root = crate::test_support::reserve_temp_directory("wizard-model-tree-bound");
        let models = root.join("models");
        std::fs::create_dir_all(&models).unwrap();
        run_git(&root, &["init", "--quiet"]);
        for index in 0..5 {
            let path = models.join(format!("input-{index}.tla"));
            std::fs::write(&path, b"tracked").unwrap();
        }
        run_git(&root, &["add", "models"]);
        for index in 0..5 {
            OpenOptions::open(
                OpenOptions::write(&mut File::options(), true),
                models.join(format!("input-{index}.tla")),
            )
            .unwrap()
            .set_len(MAX_MODEL_FILE_BYTES)
            .unwrap();
        }

        let error = tracked_tree_digest(&root, "models")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("exceeding its 4194304-byte limit"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn streamed_model_tree_digest_preserves_the_length_delimited_schema() {
        let root = crate::test_support::reserve_temp_directory("wizard-model-digest");
        std::fs::create_dir_all(root.join("models/nested")).unwrap();
        run_git(&root, &["init", "--quiet"]);
        for (path, contents) in [
            ("models/a.tla", b"short".as_slice()),
            ("models/nested/longer.cfg", b"different bytes".as_slice()),
        ] {
            std::fs::write(root.join(path), contents).unwrap();
        }
        run_git(&root, &["add", "models"]);

        let observed = tracked_tree_digest(&root, "models").unwrap();
        let listing = git_output_bytes(
            &root,
            &["ls-files", "-z", "--", "models"],
            MAX_MODEL_PATH_LIST_BYTES,
            "model digest compatibility fixture",
        )
        .unwrap();
        let paths: Vec<&str> = listing
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| std::str::from_utf8(path).unwrap())
            .collect();
        let mut legacy = Vec::new();
        let mut bytes = 0u64;
        for relative in &paths {
            let contents = std::fs::read(root.join(relative)).unwrap();
            bytes += contents.len() as u64;
            legacy.extend_from_slice(&(relative.len() as u64).to_le_bytes());
            legacy.extend_from_slice(relative.as_bytes());
            legacy.extend_from_slice(&(contents.len() as u64).to_le_bytes());
            legacy.extend_from_slice(&contents);
        }

        assert_eq!(observed.files, paths.len());
        assert_eq!(observed.bytes, bytes);
        assert_eq!(observed.sha256, sha256::hex(&sha256::digest(&legacy)));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_queries_enforce_the_callers_semantic_capture_limit() {
        let executable = std::env::current_exe().unwrap();
        let error = git_output_bytes_with(
            &std::env::current_dir().unwrap(),
            &["status", "--porcelain=v1"],
            32,
            "fixture Git query",
            |command, timeout, label, limit| {
                assert_eq!(
                    command.get_program(),
                    crate::git::resolved()
                        .unwrap()
                        .command(Path::new("."))
                        .get_program()
                );
                assert_eq!(timeout, PROBE_TIMEOUT);
                assert_eq!(limit, 32);
                let mut noisy = Command::new(&executable);
                noisy.args([
                    "--exact",
                    "bounded_process::tests::noisy_capture_fixture_child",
                    "--ignored",
                    "--nocapture",
                ]);
                crate::bounded_process::output_with_limit(noisy, timeout, label, limit)
            },
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("exceeded 32 bytes"));
    }

    #[test]
    fn deterministic_machine_images_have_exact_sizes_before_verification() {
        let root = crate::test_support::reserve_temp_directory("wizard-exact-artifact");
        let image = root.join(VMWARE_IMAGE);
        File::create(&image)
            .unwrap()
            .set_len(boot_image_bytes() - 1)
            .unwrap();
        assert!(require_artifact_size(Kind::VmwareShipped, VMWARE_IMAGE, &image).is_err());
        OpenOptions::open(OpenOptions::write(&mut File::options(), true), &image)
            .unwrap()
            .set_len(boot_image_bytes())
            .unwrap();
        require_artifact_size(Kind::VmwareShipped, VMWARE_IMAGE, &image).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_qualification_size_policy_has_an_independent_sparse_caller_witness() {
        struct ExpectedPolicy {
            prefix: String,
            relative: String,
            artifact: Option<(Kind, &'static str)>,
            maximum: u64,
            exact: bool,
        }

        let record = |slug: &str| ExpectedPolicy {
            prefix: format!("qualification.{slug}.record"),
            relative: format!("qualification/{slug}/record"),
            artifact: None,
            maximum: 262_144,
            exact: false,
        };
        let artifact =
            |slug: &str, kind: Kind, key: &str, name: &'static str, maximum: u64, exact: bool| {
                ExpectedPolicy {
                    prefix: format!("qualification.{slug}.{key}"),
                    relative: format!("qualification/{slug}/{name}"),
                    artifact: Some((kind, name)),
                    maximum,
                    exact,
                }
            };

        let mut expected = vec![
            record("cargo-mutants-grade"),
            artifact(
                "cargo-mutants-grade",
                Kind::CargoMutantsGrade,
                "request",
                "request",
                1_048_576,
                false,
            ),
            artifact(
                "cargo-mutants-grade",
                Kind::CargoMutantsGrade,
                "inventory",
                "inventory.json",
                67_108_864,
                false,
            ),
            artifact(
                "cargo-mutants-grade",
                Kind::CargoMutantsGrade,
                "mutants",
                "mutants.json",
                67_108_864,
                false,
            ),
            artifact(
                "cargo-mutants-grade",
                Kind::CargoMutantsGrade,
                "outcomes",
                "outcomes.json",
                67_108_864,
                false,
            ),
            artifact(
                "cargo-mutants-grade",
                Kind::CargoMutantsGrade,
                "adjudications",
                "adjudications.json",
                16_777_216,
                false,
            ),
            artifact(
                "cargo-mutants-grade",
                Kind::CargoMutantsGrade,
                "doctest",
                "doctest.transcript",
                1_048_576,
                false,
            ),
            record("codex-security-deep"),
            artifact(
                "codex-security-deep",
                Kind::CodexSecurityDeep,
                "request",
                "request",
                262_144,
                false,
            ),
            artifact(
                "codex-security-deep",
                Kind::CodexSecurityDeep,
                "manifest",
                "scan-manifest.json",
                4_194_304,
                false,
            ),
            artifact(
                "codex-security-deep",
                Kind::CodexSecurityDeep,
                "findings",
                "findings.json",
                33_554_432,
                false,
            ),
            artifact(
                "codex-security-deep",
                Kind::CodexSecurityDeep,
                "coverage",
                "coverage.json",
                4_194_304,
                false,
            ),
            artifact(
                "codex-security-deep",
                Kind::CodexSecurityDeep,
                "report",
                "report.md",
                33_554_432,
                false,
            ),
            artifact(
                "codex-security-deep",
                Kind::CodexSecurityDeep,
                "worker-transitions",
                "worker-transitions",
                1_048_576,
                false,
            ),
            record("advisory"),
            artifact(
                "advisory",
                Kind::Advisory,
                "policy",
                ".cargo/audit.toml",
                4_096,
                false,
            ),
            artifact(
                "advisory",
                Kind::Advisory,
                "transcript",
                "cargo-audit.transcript",
                16_781_312,
                false,
            ),
            artifact(
                "advisory",
                Kind::Advisory,
                "refresh-transcript",
                "cargo-audit-refresh.transcript",
                16_781_312,
                false,
            ),
            record("mutation"),
            artifact(
                "mutation",
                Kind::Mutation,
                "report",
                "report",
                16_777_216,
                false,
            ),
            record("model"),
        ];
        for (key, name) in [
            ("ring-clean", "ring-clean.transcript"),
            ("ring-memory-clean", "ring-memory-clean.transcript"),
            (
                "ring-memory-store-relaxed",
                "ring-memory-store-relaxed.transcript",
            ),
            (
                "ring-memory-load-relaxed",
                "ring-memory-load-relaxed.transcript",
            ),
            (
                "ring-memory-no-reservation",
                "ring-memory-no-reservation.transcript",
            ),
            (
                "ring-memory-duplicate-terminal",
                "ring-memory-duplicate-terminal.transcript",
            ),
        ] {
            expected.push(artifact("model", Kind::Model, key, name, 16_781_312, false));
        }
        for (slug, kind) in [
            ("vmware-self-test", Kind::VmwareSelfTest),
            ("vmware-shipped", Kind::VmwareShipped),
        ] {
            expected.push(record(slug));
            for (key, name, maximum, exact) in [
                ("generation", "generation.receipt", 1_048_576, false),
                ("image", "bedrock-flat.img", 67_108_864, true),
                ("namespace", "wizard-nvm-flat.img", 68_157_440, true),
                ("vmdk", "bedrock.vmdk", 1_048_576, false),
                ("namespace-vmdk", "wizard-nvm.vmdk", 1_048_576, false),
                ("vmx", "bedrock.vmx", 1_048_576, false),
                ("handoff", "dma-handoff.bin", 48, true),
                ("transcript", "serial.log", 16_777_216, false),
                ("observer", "vmrun-list.txt", 1_048_576, false),
            ] {
                expected.push(artifact(slug, kind, key, name, maximum, exact));
            }
        }

        assert_eq!(BUNDLE_FILES.len(), expected.len());
        let root = crate::test_support::reserve_temp_directory("wizard-size-policy");
        let path = root.join("sparse-artifact");
        File::create(&path).unwrap();

        for (spec, expected) in BUNDLE_FILES.iter().zip(&expected) {
            assert_eq!(spec.prefix, expected.prefix);
            assert_eq!(spec.relative, expected.relative);
            assert_eq!(
                bundle_file_size_policy(spec).unwrap(),
                (expected.maximum, expected.exact),
                "{} has the wrong bundle policy",
                expected.relative
            );
            if let Some((kind, name)) = expected.artifact {
                assert_eq!(
                    artifact_size_policy(kind, name).unwrap(),
                    (expected.maximum, expected.exact),
                    "{} has the wrong artifact policy",
                    expected.relative
                );
            }

            let refused = if expected.exact {
                expected.maximum - 1
            } else {
                expected.maximum + 1
            };
            OpenOptions::open(OpenOptions::write(&mut File::options(), true), &path)
                .unwrap()
                .set_len(refused)
                .unwrap();
            assert!(
                require_bundle_file_size(spec, &path).is_err(),
                "{} admitted a sparse file outside its bundle policy",
                expected.relative
            );
            if let Some((kind, name)) = expected.artifact {
                assert!(
                    require_artifact_size(kind, name, &path).is_err(),
                    "{} admitted a sparse file outside its artifact policy",
                    expected.relative
                );
            }

            OpenOptions::open(OpenOptions::write(&mut File::options(), true), &path)
                .unwrap()
                .set_len(expected.maximum)
                .unwrap();
            require_bundle_file_size(spec, &path).unwrap();
            if let Some((kind, name)) = expected.artifact {
                require_artifact_size(kind, name, &path).unwrap();
            }
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vmware_capture_preflights_image_size_before_streaming() {
        let root = crate::test_support::reserve_temp_directory("wizard-streamed-artifact");
        let source = root.join("source.img");
        let captured = root.join("captured.img");
        File::create(&source).unwrap().set_len(4097).unwrap();

        assert!(
            capture_vmware_file(&source, &captured, VMWARE_IMAGE, 4096, 1).is_err(),
            "an oversized image reached the capture stream"
        );
        assert!(
            !captured.exists(),
            "an oversized image created partial qualification bytes"
        );

        OpenOptions::open(OpenOptions::write(&mut File::options(), true), &source)
            .unwrap()
            .set_len(4096)
            .unwrap();
        let digest = capture_vmware_file(&source, &captured, VMWARE_IMAGE, 4096, 1).unwrap();
        assert_eq!(digest.bytes, 4096);
        assert_eq!(std::fs::metadata(captured).unwrap().len(), 4096);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vmware_capture_refuses_growth_after_its_preflight_measurement() {
        let root = crate::test_support::reserve_temp_directory("wizard-growing-vmware-capture");
        let source = root.join("source.img");
        let captured = root.join("captured.img");
        File::create(&source).unwrap().set_len(4096).unwrap();
        let growing = source.clone();

        let error =
            capture_vmware_file_with(&source, &captured, VMWARE_IMAGE, 4096, 1, move || {
                OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), growing)?
                    .set_len(4097)?;
                Ok(())
            })
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("grew beyond its 4096-byte capture limit while being copied"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vmware_capture_refuses_truncation_after_exact_preflight() {
        let root = crate::test_support::reserve_temp_directory("wizard-truncated-vmware-capture");
        let source = root.join("source.img");
        let captured = root.join("captured.img");
        File::create(&source).unwrap().set_len(4096).unwrap();
        let shrinking = source.clone();

        let error =
            capture_vmware_file_with(&source, &captured, VMWARE_IMAGE, 4096, 1, move || {
                OpenOptions::open(OpenOptions::write(&mut OpenOptions::new(), true), shrinking)?
                    .set_len(4095)?;
                Ok(())
            })
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("changed outside its exact 4096-byte capture extent"),
            "{error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_source_identity_is_rejected_field_by_field() {
        let record = complete_record(Kind::Mutation);
        let current = SourceIdentity {
            commit: "9".repeat(40),
            tree: "b".repeat(40),
        };
        let error = require_source(&record, &current).unwrap_err().to_string();
        assert!(error.contains("another commit"), "{error}");

        let current = SourceIdentity {
            commit: "a".repeat(40),
            tree: "8".repeat(40),
        };
        let error = require_source(&record, &current).unwrap_err().to_string();
        assert!(error.contains("another tree"), "{error}");
    }

    #[test]
    fn source_capture_requires_two_clean_views_of_one_identity() {
        let current = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        require_stable_source_capture("", &current, "", &current).unwrap();

        let changed = SourceIdentity {
            commit: "9".repeat(40),
            tree: current.tree.clone(),
        };
        assert!(require_stable_source_capture("", &current, "", &changed).is_err());
        assert!(
            require_stable_source_capture(" M kernel/src/main.rs\n", &current, "", &current,)
                .is_err()
        );
        assert!(
            require_stable_source_capture("", &current, "?? transient-input\n", &current,).is_err()
        );
    }

    #[test]
    fn copied_support_bytes_must_match_their_inner_record() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-support");
        let paths = Paths::new(root.join("repository"), None);
        let directory = root.join(Kind::Mutation.slug());
        std::fs::create_dir_all(&directory).unwrap();
        let report = directory.join(MUTATION_REPORT);
        std::fs::write(&report, "accepted").unwrap();
        let digest = digest_file(&report).unwrap();
        let mut record = complete_record(Kind::Mutation);
        record.fields.insert(
            "artifact.report.bytes".to_string(),
            digest.bytes.to_string(),
        );
        record
            .fields
            .insert("artifact.report.sha256".to_string(), digest.sha256);
        verify_support(&paths, &record, Kind::Mutation, &root).unwrap();

        std::fs::write(&report, "tampered").unwrap();
        assert!(verify_support(&paths, &record, Kind::Mutation, &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_support_rejects_omitted_substituted_and_reordered_runs() {
        let root = crate::test_support::reserve_temp_directory("wizard-model-support");
        let paths = Paths::new(root.join("repository"), None);
        let directory = root.join(Kind::Model.slug());
        std::fs::create_dir_all(&directory).unwrap();
        let transcripts = crate::model::fixture_transcripts();
        let mut record = complete_record(Kind::Model);
        for transcript in &transcripts {
            let artifact = transcript.artifact();
            std::fs::write(directory.join(artifact.name), transcript.bytes()).unwrap();
            let digest = digest_bytes(transcript.bytes());
            record.fields.insert(
                format!("{}.bytes", artifact.prefix),
                digest.bytes.to_string(),
            );
            record
                .fields
                .insert(format!("{}.sha256", artifact.prefix), digest.sha256);
        }
        verify_support(&paths, &record, Kind::Model, &root).unwrap();

        let first = transcripts[0].artifact();
        let second = transcripts[1].artifact();
        std::fs::remove_file(directory.join(first.name)).unwrap();
        assert!(verify_support(&paths, &record, Kind::Model, &root).is_err());

        std::fs::write(directory.join(first.name), transcripts[1].bytes()).unwrap();
        let donated = digest_bytes(transcripts[1].bytes());
        record
            .fields
            .insert(format!("{}.bytes", first.prefix), donated.bytes.to_string());
        record
            .fields
            .insert(format!("{}.sha256", first.prefix), donated.sha256);
        assert!(
            verify_support(&paths, &record, Kind::Model, &root).is_err(),
            "a valid clean transcript donated another run's identity"
        );

        std::fs::write(directory.join(first.name), transcripts[0].bytes()).unwrap();
        let restored = digest_bytes(transcripts[0].bytes());
        record.fields.insert(
            format!("{}.bytes", first.prefix),
            restored.bytes.to_string(),
        );
        record
            .fields
            .insert(format!("{}.sha256", first.prefix), restored.sha256);
        verify_support(&paths, &record, Kind::Model, &root).unwrap();
        let first_path = format!("{}.path", first.prefix);
        let second_path = format!("{}.path", second.prefix);
        record.fields.insert(first_path, second.name.to_string());
        record.fields.insert(second_path, first.name.to_string());
        assert!(verify_support(&paths, &record, Kind::Model, &root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_snapshot_parent_link_cannot_substitute_qualification_bytes() {
        let root =
            crate::test_support::reserve_temp_directory("wizard-qualification-snapshot-link");
        write_snapshot_bundle(&root);
        let digests = BUNDLE_FILES
            .iter()
            .map(|spec| digest_file(&root.join(bundle_tail(spec).unwrap())).unwrap())
            .collect();
        let snapshot = Snapshot {
            root: root.clone(),
            digests,
            campaigns: CampaignStates::from_labels([PASS, PASS]).unwrap(),
            _mutation: RetainedMutationPass::new(
                CurrentPass::fixture(&root.join("mutation.lock")).unwrap(),
            ),
        };
        snapshot.verify_unchanged().unwrap();
        let model = root.join(Kind::Model.slug());
        let alternate = root.join("model-alternate");
        std::fs::rename(&model, &alternate).unwrap();
        create_directory_symlink(&alternate, &model);

        assert!(snapshot.verify_unchanged().is_err());
        drop(snapshot);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_directories_are_validated_before_any_redirected_creation() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-install-link");
        let source = root.join("source");
        let evidence = root.join("evidence");
        let outside = root.join("outside");
        write_snapshot_bundle(&source);
        std::fs::create_dir_all(&evidence).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        create_directory_symlink(&outside, &evidence.join("qualification"));
        let digests = BUNDLE_FILES
            .iter()
            .map(|spec| digest_file(&source.join(bundle_tail(spec).unwrap())).unwrap())
            .collect();
        let mutation_lock = source.join("mutation.lock");
        let snapshot = Snapshot {
            root: source.clone(),
            digests,
            campaigns: CampaignStates::from_labels([PASS, PASS]).unwrap(),
            _mutation: RetainedMutationPass::new(CurrentPass::fixture(&mutation_lock).unwrap()),
        };
        assert!(CurrentPass::fixture(&mutation_lock).is_err());

        assert!(snapshot.install(&evidence).is_err());
        assert!(CurrentPass::fixture(&mutation_lock).is_err());
        assert!(
            crate::paths::directory_entries(&outside)
                .unwrap()
                .is_empty(),
            "bundle preparation created a directory through a redirected parent"
        );

        drop(snapshot);
        drop(CurrentPass::fixture(&mutation_lock).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_retains_mutation_lock_through_install_refusal() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-install-lock");
        let source = root.join("source");
        let evidence = root.join("not-a-directory");
        write_snapshot_bundle(&source);
        std::fs::write(&evidence, "occupied").unwrap();
        let digests = BUNDLE_FILES
            .iter()
            .map(|spec| digest_file(&source.join(bundle_tail(spec).unwrap())).unwrap())
            .collect();
        let mutation_lock = source.join("mutation.lock");
        let snapshot = Snapshot {
            root: source,
            digests,
            campaigns: CampaignStates::from_labels([PASS, PASS]).unwrap(),
            _mutation: RetainedMutationPass::new(CurrentPass::fixture(&mutation_lock).unwrap()),
        };

        assert!(CurrentPass::fixture(&mutation_lock).is_err());
        assert!(snapshot.install(&evidence).is_err());
        assert!(CurrentPass::fixture(&mutation_lock).is_err());
        drop(snapshot);
        drop(CurrentPass::fixture(&mutation_lock).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn newest_mutation_and_tool_inputs_are_compared_field_by_field() {
        let mutation = complete_record(Kind::Mutation);
        let current = PassEvidence {
            commit: "a".repeat(40),
            registry: "c".repeat(64),
            source_digest: "d".repeat(64),
            index_digest: "e".repeat(64),
            mutation_count: 1,
            report_digest: "0".repeat(64),
            report_path: PathBuf::from("unused.report"),
        };
        require_mutation_inputs(&mutation, &current.registry, 1, Some(&current)).unwrap();
        assert!(require_mutation_inputs(&mutation, &"9".repeat(64), 1, None).is_err());
        assert!(require_mutation_inputs(&mutation, &current.registry, 2, None).is_err());
        for field in ["commit", "registry", "source", "index", "report", "count"] {
            let mut stale = current.clone();
            match field {
                "commit" => stale.commit = "9".repeat(40),
                "registry" => stale.registry = "9".repeat(64),
                "source" => stale.source_digest = "9".repeat(64),
                "index" => stale.index_digest = "9".repeat(64),
                "report" => stale.report_digest = "9".repeat(64),
                "count" => stale.mutation_count = 2,
                _ => unreachable!(),
            }
            assert!(
                require_mutation_inputs(&mutation, &current.registry, 1, Some(&stale)).is_err(),
                "stale mutation {field} was accepted"
            );
        }

        for (kind, keys) in [
            (
                Kind::Model,
                &[
                    "input.java.sha256",
                    "input.java.version",
                    "input.models.sha256",
                    "input.tlc-jar.sha256",
                ][..],
            ),
            (Kind::VmwareSelfTest, &["input.vmrun.sha256"][..]),
        ] {
            let recorded = complete_record(kind);
            for key in keys {
                let mut current = recorded.clone();
                current.fields.insert((*key).to_string(), "9".repeat(64));
                assert!(
                    require_record_inputs(&recorded, &current).is_err(),
                    "stale exceptional input {key} was accepted"
                );
            }
        }
    }

    #[test]
    fn generation_receipt_binds_reset_source_selector_and_machine_bytes() {
        let directory = crate::test_support::reserve_temp_directory("wizard-vmware-receipt");
        for name in [
            VMWARE_IMAGE,
            VMWARE_NAMESPACE,
            VMWARE_VMDK,
            VMWARE_NAMESPACE_VMDK,
            VMWARE_VMX,
        ] {
            std::fs::write(directory.join(name), name).unwrap();
        }
        std::fs::write(
            directory.join(VMWARE_HANDOFF),
            crate::platform_acpi::dma_handoff_table(),
        )
        .unwrap();
        let generation = "1".repeat(64);
        std::fs::write(
            directory.join(VMWARE_VMX),
            crate::vmware::generation_vmx(
                Kernel::SelfTest,
                &generation_transcript_name(&generation).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let transcript = directory.join(generation_transcript_name(&generation).unwrap());
        std::fs::write(&transcript, []).unwrap();
        require_empty_reset(&digest_file(&transcript).unwrap()).unwrap();
        require_generation_inventory(&directory, &transcript, false).unwrap();
        std::fs::write(directory.join("nvram"), "stale firmware state").unwrap();
        assert!(require_generation_inventory(&directory, &transcript, false).is_err());
        std::fs::remove_file(directory.join("nvram")).unwrap();
        std::fs::remove_file(directory.join(VMWARE_IMAGE)).unwrap();
        std::fs::create_dir(directory.join(VMWARE_IMAGE)).unwrap();
        assert!(require_generation_inventory(&directory, &transcript, false).is_err());
        std::fs::remove_dir(directory.join(VMWARE_IMAGE)).unwrap();
        std::fs::write(directory.join(VMWARE_IMAGE), VMWARE_IMAGE).unwrap();
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let record =
            generation_record(&source, Kind::VmwareSelfTest, &directory, &generation).unwrap();
        std::fs::write(directory.join(VMWARE_GENERATION_RECEIPT), record.render()).unwrap();
        assert!(
            verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory, &transcript)
                .is_err(),
            "an untouched reset transcript counted as execution"
        );

        std::fs::write(&transcript, "fresh machine output\n").unwrap();
        assert!(require_empty_reset(&digest_file(&transcript).unwrap()).is_err());
        verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory, &transcript)
            .unwrap();
        let stale = SourceIdentity {
            commit: "9".repeat(40),
            tree: source.tree.clone(),
        };
        assert!(
            verify_generation_with_source(&stale, Kind::VmwareSelfTest, &directory, &transcript)
                .is_err()
        );
        assert!(
            verify_generation_with_source(&source, Kind::VmwareShipped, &directory, &transcript)
                .is_err()
        );
        std::fs::write(directory.join(VMWARE_VMX), "changed").unwrap();
        assert!(
            verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory, &transcript)
                .is_err()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn vmware_accepts_only_the_source_defined_dma_handoff_authority() {
        let root = crate::test_support::reserve_temp_directory("wizard-vmware-handoff");
        let path = root.join("handoff.bin");
        let mut foreign = crate::platform_acpi::dma_handoff_table();
        foreign[10] ^= 1;
        foreign[9] = 0;
        foreign[9] = 0u8.wrapping_sub(
            foreign
                .iter()
                .fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
        );
        std::fs::write(&path, foreign).unwrap();
        assert!(
            acpi_tables::DmaHandoff::new(&foreign).is_ok(),
            "the near miss must remain structurally valid ACPI"
        );
        assert!(verified_dma_handoff_digest(&path).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_immutable_vmware_generation_is_captured_before_publication() {
        fn write_generation(
            directory: &Path,
            source: &SourceIdentity,
            generation: &str,
            tag: &str,
        ) -> PathBuf {
            std::fs::create_dir_all(directory).unwrap();
            for name in [
                VMWARE_IMAGE,
                VMWARE_NAMESPACE,
                VMWARE_VMDK,
                VMWARE_NAMESPACE_VMDK,
                VMWARE_VMX,
            ] {
                std::fs::write(directory.join(name), format!("{tag}:{name}")).unwrap();
            }
            std::fs::write(
                directory.join(VMWARE_HANDOFF),
                crate::platform_acpi::dma_handoff_table(),
            )
            .unwrap();
            std::fs::write(
                directory.join(VMWARE_VMX),
                crate::vmware::generation_vmx(
                    Kernel::SelfTest,
                    &generation_transcript_name(generation).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
            let transcript = directory.join(generation_transcript_name(generation).unwrap());
            std::fs::write(&transcript, format!("{tag}:transcript\n")).unwrap();
            let receipt =
                generation_record(source, Kind::VmwareSelfTest, directory, generation).unwrap();
            std::fs::write(directory.join(VMWARE_GENERATION_RECEIPT), receipt.render()).unwrap();
            transcript
        }

        fn capture(
            directory: &Path,
            transcript: &Path,
            captured: &Path,
        ) -> CapturedVmwareGeneration {
            let image = directory.join(VMWARE_IMAGE);
            let namespace = directory.join(VMWARE_NAMESPACE);
            std::fs::create_dir(captured).unwrap();
            CapturedVmwareGeneration::read_with_sizes(
                &VmwareArtifacts {
                    image: &directory.join(VMWARE_IMAGE),
                    namespace: &directory.join(VMWARE_NAMESPACE),
                    vmdk: &directory.join(VMWARE_VMDK),
                    namespace_vmdk: &directory.join(VMWARE_NAMESPACE_VMDK),
                    vmx: &directory.join(VMWARE_VMX),
                    handoff: &directory.join(VMWARE_HANDOFF),
                    transcript,
                    receipt: &directory.join(VMWARE_GENERATION_RECEIPT),
                },
                captured,
                std::fs::metadata(image).unwrap().len(),
                std::fs::metadata(namespace).unwrap().len(),
            )
            .unwrap()
        }

        let root = crate::test_support::reserve_temp_directory("wizard-vmware-capture");
        let source = SourceIdentity {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let generation_a = "1".repeat(64);
        let generation_b = "2".repeat(64);
        let directory_a = root.join("generation-a");
        let directory_b = root.join("generation-b");
        let transcript_a = write_generation(&directory_a, &source, &generation_a, "generation-a");
        let transcript_b = write_generation(&directory_b, &source, &generation_b, "generation-b");

        let generated_vmx = std::fs::read(directory_a.join(VMWARE_VMX)).unwrap();
        let image_a = std::fs::read(directory_a.join(VMWARE_IMAGE)).unwrap();
        let image_b = std::fs::read(directory_b.join(VMWARE_IMAGE)).unwrap();
        let mut runtime_vmx = generated_vmx.clone();
        runtime_vmx.extend_from_slice(b"cleanShutdown = \"FALSE\"\n");
        std::fs::write(directory_a.join(VMWARE_VMX), &runtime_vmx).unwrap();

        let valid = capture(&directory_a, &transcript_a, &root.join("captured-valid"));
        valid
            .validate_generation(&source, Kind::VmwareSelfTest)
            .unwrap();
        assert_ne!(
            valid.file(VMWARE_VMX).unwrap().digest,
            digest_bytes(&generated_vmx)
        );
        verify_generation_with_source(&source, Kind::VmwareSelfTest, &directory_a, &transcript_a)
            .unwrap();
        verify_generation_with_source(
            &source,
            Kind::VmwareSelfTest,
            &root.join("captured-valid"),
            &root.join("captured-valid").join(VMWARE_TRANSCRIPT),
        )
        .unwrap();

        std::fs::copy(directory_b.join(VMWARE_VMX), directory_a.join(VMWARE_VMX)).unwrap();
        let mixed = capture(&directory_a, &transcript_a, &root.join("captured-mixed"));
        assert!(
            mixed
                .validate_generation(&source, Kind::VmwareSelfTest)
                .is_err(),
            "machine bytes from two valid generations were combined"
        );

        std::fs::write(directory_a.join(VMWARE_VMX), &runtime_vmx).unwrap();
        let wrong_transcript = capture(
            &directory_a,
            &transcript_b,
            &root.join("captured-wrong-transcript"),
        );
        assert!(
            wrong_transcript
                .validate_generation(&source, Kind::VmwareSelfTest)
                .is_err(),
            "a valid transcript path from another generation was accepted"
        );

        std::fs::write(directory_a.join(VMWARE_IMAGE), &image_b).unwrap();
        let mixed_receipt = capture(
            &directory_a,
            &transcript_a,
            &root.join("captured-mixed-receipt"),
        );
        assert!(
            mixed_receipt
                .validate_generation(&source, Kind::VmwareSelfTest)
                .is_err(),
            "machine bytes from two valid generations shared one receipt"
        );
        std::fs::write(directory_a.join(VMWARE_IMAGE), image_a).unwrap();

        let published = root.join("published");
        let captured = capture(&directory_a, &transcript_a, &published);
        captured
            .validate_generation(&source, Kind::VmwareSelfTest)
            .unwrap();
        let captured_vmx = std::fs::read(&captured.file(VMWARE_VMX).unwrap().captured).unwrap();
        std::fs::write(directory_a.join(VMWARE_VMX), b"changed after capture").unwrap();
        let mut record = Record::default();
        captured.publish(&published, &mut record).unwrap();
        assert_eq!(
            std::fs::read(published.join(VMWARE_VMX)).unwrap(),
            captured_vmx
        );
        assert!(captured.verify_sources().is_err());
        assert!(
            !published.join(RECORD).exists(),
            "a changed capture published an authoritative qualification record"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn each_vmware_generation_owns_a_distinct_serial_file() {
        let first = fresh_vmware_transcript_name().unwrap();
        let second = fresh_vmware_transcript_name().unwrap();
        assert_ne!(first, second);
        for name in [first, second] {
            assert!(name.starts_with(SERIAL_PREFIX));
            assert!(name.ends_with(SERIAL_SUFFIX));
            assert!(is_digest(
                name.strip_prefix(SERIAL_PREFIX)
                    .and_then(|name| name.strip_suffix(SERIAL_SUFFIX))
                    .unwrap()
            ));
        }
    }

    #[test]
    fn a_vmware_selector_directory_cannot_be_substituted() {
        let root = crate::test_support::reserve_temp_directory("wizard-vmware-selector");
        let repository = root.join("repository");
        let vm_root = repository.join("vm");
        let selector = vm_root.join("self-test");
        let alternate = vm_root.join("alternate");
        let outside = root.join("outside");
        std::fs::create_dir_all(&selector).unwrap();
        std::fs::create_dir(&alternate).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let paths = Paths::new(repository, None);
        require_exact_vmware_directory(&paths, Kind::VmwareSelfTest, &selector).unwrap();

        std::fs::remove_dir(&selector).unwrap();
        create_directory_symlink(&alternate, &selector);
        assert!(
            require_exact_vmware_directory(&paths, Kind::VmwareSelfTest, &selector).is_err(),
            "an authoritative selector name accepted another in-root directory"
        );
        std::fs::remove_dir(&selector).unwrap();
        create_directory_symlink(&outside, &selector);
        assert!(
            require_exact_vmware_directory(&paths, Kind::VmwareSelfTest, &selector).is_err(),
            "an authoritative selector name escaped the VMware root"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vmrun_list_requires_one_exact_running_machine() {
        let root = crate::test_support::reserve_temp_directory("wizard-vmrun-list");
        let expected = root.join("expected.vmx");
        let foreign = root.join("foreign.vmx");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&expected, "vmx").unwrap();
        std::fs::write(&foreign, "vmx").unwrap();
        let valid = format!("{VMRUN_LIST_HEADER}1\n{}", expected.display());
        let observed = parse_vmrun_list(&valid).unwrap();
        require_observed_vmx(&observed, &expected).unwrap();

        for invalid in [
            format!("{VMRUN_LIST_HEADER}0"),
            format!(
                "{VMRUN_LIST_HEADER}2\n{}\n{}",
                expected.display(),
                foreign.display()
            ),
            format!("{VMRUN_LIST_HEADER}1"),
            format!("{VMRUN_LIST_HEADER}01\n{}", expected.display()),
            format!("Running VMs: 1\n{}", expected.display()),
            format!("{VMRUN_LIST_HEADER}1\n{}\nextra", expected.display()),
            format!("{VMRUN_LIST_HEADER}1\r\n{}", expected.display()),
        ] {
            assert!(parse_vmrun_list(&invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(require_observed_vmx(&foreign, &expected).is_err());
        assert!(require_observed_vmx(Path::new("expected.vmx"), &expected).is_err());
        assert_eq!(VMRUN_LIST_ARGS, ["-T", "ws", "list"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offline_vmware_observer_is_bound_to_its_exact_selector() {
        let root = crate::test_support::reserve_temp_directory("wizard-vmrun-replay");
        let repository = root.join("repository");
        let qualification = root.join("qualification");
        let paths = Paths::new(repository.clone(), Some(&root.join("target")));
        let self_test_vmx = repository.join("vm/self-test").join(VMWARE_VMX);
        let shipped_vmx = repository.join("vm/shipped").join(VMWARE_VMX);
        let foreign_vmx = root.join("foreign.vmx");
        for path in [&self_test_vmx, &shipped_vmx, &foreign_vmx] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "vmx").unwrap();
        }
        for kind in [Kind::VmwareSelfTest, Kind::VmwareShipped] {
            std::fs::create_dir_all(qualification.join(kind.slug())).unwrap();
        }

        let write_observer = |kind: Kind, vmx: &Path| {
            let observer = qualification.join(kind.slug()).join(VMWARE_OBSERVER);
            std::fs::write(
                &observer,
                format!("{VMRUN_LIST_HEADER}1\n{}\n", vmx.display()),
            )
            .unwrap();
            let mut record = Record::default();
            add_artifact(&mut record, "artifact.observer", VMWARE_OBSERVER, &observer).unwrap();
            record
        };

        let self_test = write_observer(Kind::VmwareSelfTest, &self_test_vmx);
        verify_vmware_observer(&paths, &self_test, Kind::VmwareSelfTest, &qualification).unwrap();
        let shipped = write_observer(Kind::VmwareShipped, &shipped_vmx);
        verify_vmware_observer(&paths, &shipped, Kind::VmwareShipped, &qualification).unwrap();

        let swapped = write_observer(Kind::VmwareSelfTest, &shipped_vmx);
        assert!(
            verify_vmware_observer(&paths, &swapped, Kind::VmwareSelfTest, &qualification,)
                .is_err(),
            "a coherently checksummed shipped observer qualified the self-test selector"
        );
        let foreign = write_observer(Kind::VmwareSelfTest, &foreign_vmx);
        assert!(
            verify_vmware_observer(&paths, &foreign, Kind::VmwareSelfTest, &qualification,)
                .is_err(),
            "a coherently checksummed foreign absolute VMX qualified"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vmrun_observation_executes_list_between_exact_tool_probes() {
        let root = crate::test_support::reserve_temp_directory("wizard-vmrun-order");
        let expected = root.join("expected.vmx");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&expected, "vmx").unwrap();
        let tool = Tool {
            path: root.join("vmrun"),
            version: "vmrun version test build-1".to_string(),
        };
        let observer = format!("{VMRUN_LIST_HEADER}1\n{}", expected.display());
        let mut calls = Vec::new();

        let actual = observe_running_vm_with(&tool, &expected, |program, args, policy| {
            calls.push((
                program.to_path_buf(),
                args.iter()
                    .map(|arg| (*arg).to_string())
                    .collect::<Vec<_>>(),
                policy,
            ));
            match args {
                [] => Ok(tool.version.clone()),
                ["-T", "ws", "list"] => Ok(observer.clone()),
                _ => bail!("unexpected vmrun arguments: {args:?}"),
            }
        })
        .unwrap();

        assert_eq!(actual, observer);
        assert_eq!(
            calls,
            [
                (tool.path.clone(), Vec::new(), ExitPolicy::Any),
                (
                    tool.path.clone(),
                    vec!["-T".to_string(), "ws".to_string(), "list".to_string()],
                    ExitPolicy::Success,
                ),
                (tool.path.clone(), Vec::new(), ExitPolicy::Any),
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn starting_an_attempt_invalidates_an_older_pass() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-invalidation");
        let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
        let directory = qualification_root(&paths).join(Kind::VmwareSelfTest.slug());
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(RECORD),
            complete_record(Kind::VmwareSelfTest).render(),
        )
        .unwrap();

        invalidate(&paths, Kind::VmwareSelfTest).unwrap();

        assert!(!directory.join(RECORD).exists());
        assert!(directory.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn advisory_invalidation_precedes_even_failed_tool_resolution() {
        let root = crate::test_support::reserve_temp_directory("wizard-advisory-invalidation");
        let paths = Paths::new(root.join("repo"), Some(&root.join("target")));
        let directory = qualification_root(&paths).join(Kind::Advisory.slug());
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(RECORD), "stale advisory pass").unwrap();

        let error = begin_advisory_with(&paths, || bail!("injected tool resolution failure"))
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("injected tool resolution failure"),
            "{error}"
        );
        assert!(!directory.join(RECORD).exists());
        assert!(directory.is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_qualification_kind_directory_cannot_be_substituted() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-parent");
        let paths = Paths::new(root.join("repository"), Some(&root.join("target")));
        let qualification = qualification_root(&paths);
        let model = qualification.join(Kind::Model.slug());
        let alternate = qualification.join("alternate");
        std::fs::create_dir_all(&model).unwrap();
        std::fs::create_dir(&alternate).unwrap();
        kind_directory(&paths, Kind::Model).unwrap();

        std::fs::remove_dir(&model).unwrap();
        create_directory_symlink(&alternate, &model);
        assert!(kind_directory(&paths, Kind::Model).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_qualification_root_cannot_be_substituted() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-root");
        let target = root.join("target");
        let qualification = target.join("qualification");
        let alternate = target.join("alternate");
        std::fs::create_dir_all(alternate.join(Kind::Model.slug())).unwrap();
        create_directory_symlink(&alternate, &qualification);
        let paths = Paths::new(root.join("repository"), Some(&target));

        assert!(kind_directory(&paths, Kind::Model).is_err());

        let ancestor_target = target.join("ancestor-target");
        let ancestor_link = target.join("ancestor-link");
        std::fs::create_dir_all(ancestor_target.join("qualification")).unwrap();
        create_directory_symlink(&ancestor_target, &ancestor_link);
        assert!(require_real_root(&ancestor_link.join("qualification"), "fixture").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_redirected_target_is_refused_before_qualification_root_creation() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-target-link");
        let outside = root.join("outside");
        let target = root.join("target-link");
        std::fs::create_dir_all(&outside).unwrap();
        create_directory_symlink(&outside, &target);
        let paths = Paths::new(root.join("repository"), Some(&target));

        assert!(invalidate(&paths, Kind::Model).is_err());
        assert!(!outside.join("qualification").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_is_rechecked_immediately_before_a_record_commits() {
        let root = crate::test_support::reserve_temp_directory("wizard-qualification-source");
        let repository = root.join("repository");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init", "--quiet"]);
        std::fs::write(repository.join("tracked"), "initial\n").unwrap();
        run_git(&repository, &["add", "tracked"]);
        run_git(
            &repository,
            &[
                "-c",
                "user.name=Wizard Test",
                "-c",
                "user.email=wizard@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
        let paths = Paths::new(repository.clone(), Some(&root.join("target")));
        let mut record = base_record(&paths, Kind::Model).unwrap();
        fixture_tool(&mut record, "input.java");
        record.insert("input.models.bytes", "1").unwrap();
        record.insert("input.models.files", "1").unwrap();
        record
            .insert("input.models.sha256", "d".repeat(64))
            .unwrap();
        record.insert("input.tlc-jar.bytes", "1").unwrap();
        record
            .insert("input.tlc-jar.sha256", "e".repeat(64))
            .unwrap();
        for artifact in crate::model::TRANSCRIPT_ARTIFACTS {
            fixture_artifact(&mut record, artifact.prefix, artifact.name);
        }
        record.insert("result", PASS).unwrap();
        invalidate(&paths, Kind::Model).unwrap();
        std::fs::write(repository.join("tracked"), "changed\n").unwrap();

        let error = commit_record(&paths, Kind::Model, &record).unwrap_err();
        assert!(
            format!("{error:#}").contains("qualification requires a stable clean checkout"),
            "{error:#}"
        );
        assert!(
            !kind_directory(&paths, Kind::Model)
                .unwrap()
                .join(RECORD)
                .exists()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generation_start_invalidates_both_receipt_names() {
        let directory =
            crate::test_support::reserve_temp_directory("wizard-generation-invalidation");
        let receipt = directory.join(VMWARE_GENERATION_RECEIPT);
        std::fs::write(&receipt, "stale pass").unwrap();
        std::fs::write(directory.join(VMWARE_GENERATION_TEMP), "partial pass").unwrap();

        invalidate_vmware_generation(&receipt).unwrap();

        assert!(!receipt.exists());
        assert!(!directory.join(VMWARE_GENERATION_TEMP).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn exceptional_tasks_commit_only_after_their_terminal_verdict() {
        let dispatcher = include_str!("lib.rs");
        assert!(
            dispatcher
                .contains("Task::QualifyAdvisory => qualification::qualify_advisory(&paths),")
        );

        let qualification = include_str!("qualification.rs");
        let advisory_start = qualification.find("impl AdvisoryAttempt").unwrap();
        let advisory_end = qualification[advisory_start..]
            .find("fn advisory_command(")
            .unwrap()
            + advisory_start;
        let advisory = &qualification[advisory_start..advisory_end];
        let refresh = advisory
            .find("fresh RustSec advisory database population")
            .unwrap();
        let database = advisory
            .find("let database_identity = advisory_database_identity")
            .unwrap();
        let review = advisory
            .find("revision-pinned RustSec cargo-audit qualification")
            .unwrap();
        let unchanged: Vec<usize> = advisory
            .match_indices(
                "advisory_database_identity(&directory, &database)? == database_identity",
            )
            .map(|(offset, _)| offset)
            .collect();
        assert_eq!(unchanged.len(), 2);
        let transcript = advisory
            .find("let transcript = render_advisory_transcript")
            .unwrap();
        let advisory_result = advisory
            .find("self.record.insert(\"result\", PASS)?;")
            .unwrap();
        let advisory_commit = advisory.rfind("commit_record").unwrap();
        assert!(
            refresh < database
                && database < review
                && review < unchanged[0]
                && unchanged[0] < transcript
                && transcript < unchanged[1]
                && unchanged[1] < advisory_result
                && advisory_result < advisory_commit
        );

        let model = include_str!("model.rs");
        let begin = model
            .find("let qualification_attempt = qualification::begin_model(paths)?;")
            .unwrap();
        let preflight = model.find("verify_jar(&jar)?;").unwrap();
        let runs = model.find("for run in RUNS {").unwrap();
        let capture_contract = "        transcripts.push(transcript(\n            run,\n            success,\n            report.as_bytes(),\n            error.as_bytes(),\n        )?);";
        let capture = model
            .find(capture_contract)
            .expect("model transcripts capture the classified stdout and stderr bytes");
        assert_eq!(
            model.rfind(capture_contract),
            Some(capture),
            "the model transcript capture contract is unique"
        );
        let pass = model
            .find("qualification_attempt.pass(&transcripts)?;")
            .unwrap();
        assert!(begin < preflight && preflight < runs && runs < capture && capture < pass);

        let mutation_source = include_str!("mutate.rs");
        let mutation_start = mutation_source
            .rfind("\npub fn run(paths: &Paths) -> Result<()> {")
            .unwrap();
        let mutation_end = mutation_source[mutation_start..]
            .find("\npub(crate) fn registry_fingerprint")
            .unwrap()
            + mutation_start;
        let mutation = &mutation_source[mutation_start..mutation_end];
        let unique = |marker| {
            let occurrences: Vec<_> = mutation.match_indices(marker).collect();
            assert_eq!(
                occurrences.len(),
                1,
                "mutation run marker `{marker}` is not unique"
            );
            occurrences[0].0
        };
        let begin = unique("let qualification_attempt = qualification::begin_mutation");
        let open = unique("let mut session = Session::open");
        let complete = unique("let outcomes = session.complete");
        let current = unique("let pass = crate::mutation_workspace::current_pass");
        let pass = unique("qualification_attempt.pass(&pass)?;");
        assert!(begin < open && open < complete && complete < current && current < pass);

        let start = qualification.find("impl VmwareAttempt").unwrap();
        let end = qualification[start..]
            .find("pub(crate) fn invalidate_vmware_generation")
            .unwrap()
            + start;
        let body = &qualification[start..end];
        let observe = body.find("observe_running_vm").unwrap();
        let result = body.find("self.record.insert(\"result\", PASS)?;").unwrap();
        let commit = body.find("commit_record").unwrap();
        assert!(observe < result && result < commit);
    }

    #[test]
    fn vmrun_version_is_separated_from_its_usage_text() {
        let report = "\r\nvmrun version 1.17.0 build-24583834\r\n\r\nUsage: vmrun COMMAND\r\n";
        assert_eq!(
            vmrun_version(report).unwrap(),
            "vmrun version 1.17.0 build-24583834"
        );
        assert!(vmrun_version("Usage: vmrun COMMAND").is_err());
    }

    #[test]
    fn cargo_audit_version_is_one_exact_resolved_identity() {
        assert_eq!(
            cargo_audit_version(ADVISORY_TOOL_VERSION).unwrap(),
            ADVISORY_TOOL_VERSION
        );
        assert!(cargo_audit_version("cargo-audit-audit 0.22.2\nforged").is_err());
        assert!(cargo_audit_version("cargo-audit-audit 0.22.3").is_err());
        assert!(cargo_audit_version("cargo 1.97.0").is_err());
    }

    #[test]
    fn advisory_refresh_and_review_commands_are_direct_exact_and_bounded() {
        let tool = Tool {
            path: PathBuf::from("resolved-cargo-audit"),
            version: ADVISORY_TOOL_VERSION.to_string(),
        };
        let directory = PathBuf::from("qualification/advisory");
        let database = directory.join(ADVISORY_DATABASE);
        let lock = PathBuf::from("repository/Cargo.lock");
        for fetch in [true, false] {
            let command = advisory_command(&tool, &directory, &database, &lock, fetch);
            let mut expected: Vec<std::ffi::OsString> = [
                "audit",
                "--color",
                "never",
                "--db",
                database.to_str().unwrap(),
                "--deny",
                "warnings",
                "--file",
                lock.to_str().unwrap(),
                "--json",
            ]
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect();
            if !fetch {
                expected.push("--no-fetch".into());
            }
            expected.extend([
                std::ffi::OsString::from("--url"),
                std::ffi::OsString::from(ADVISORY_DATABASE_URL),
            ]);

            assert_eq!(command.get_program(), tool.path.as_os_str());
            assert_eq!(command.get_current_dir(), Some(directory.as_path()));
            assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
            assert!(command.get_envs().any(|(name, value)| {
                name == std::ffi::OsStr::new("CARGO_NET_OFFLINE") && value.is_none()
            }));
        }
    }
}
