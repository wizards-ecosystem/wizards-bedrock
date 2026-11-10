//! Exceptional qualification records and their resolved tools.

mod advisory;
mod attempts;
mod record;
mod size_policy;
mod verify;
mod vmware;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

use crate::files::require_real_directory;
use crate::git::is_git_id;
use crate::image::Kernel;
use crate::json::{
    JsonValue, json_array, json_field, json_object, json_string_array, json_u64, parse_json,
    require_json_fields,
};
use crate::mutate::workspace::{CurrentPass, PassEvidence};
use crate::paths::Paths;
use crate::sha256;

pub(crate) use self::advisory::qualify_advisory;
use self::advisory::*;
pub(crate) use self::attempts::{
    ObservedDeepArtifacts, ObservedMutantsArtifacts, begin_model, begin_mutants_grade,
    begin_mutation, begin_owned_security_deep, begin_security_deep,
};
use self::record::*;
pub(crate) use self::record::{SourceIdentity, source_identity, validate_field};
pub(crate) use self::size_policy::bundle_file_size_policy;
use self::size_policy::*;
use self::verify::*;
pub(crate) use self::verify::{recorded_source, snapshot, verify_bundle};
use self::vmware::*;
pub(crate) use self::vmware::{
    VmwareArtifacts, begin_vmware, commit_vmware_generation, fresh_vmware_transcript_name,
    generated_transcript_path, invalidate_vmware, invalidate_vmware_generation, vmrun,
};

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
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Tool {
    pub(crate) path: PathBuf,
    pub(crate) version: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

#[derive(Clone, PartialEq, Eq, Debug)]
struct Digest {
    bytes: u64,
    sha256: String,
}

#[derive(Clone, PartialEq, Eq, Default, Debug)]
struct Record {
    fields: BTreeMap<String, String>,
}

impl Record {
    fn insert(&mut self, key: impl Into<String>, value: impl Into<String>) -> Result<()> {
        let key = key.into();
        let value = value.into();
        validate_field("qualification", &key, &value)?;
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct BundleFile {
    pub(crate) prefix: &'static str,
    pub(crate) relative: &'static str,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    crate::mutate::workspace::require_direct_file_if_present(
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

fn require_regular_file(path: &Path, label: &str) -> Result<()> {
    let parent = path.parent().context("regular file has no parent")?;
    crate::mutate::workspace::canonical_existing_real_directory(parent, label)?;
    crate::mutate::workspace::require_direct_file(parent, path, label)
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
    crate::mutate::workspace::canonical_existing_real_directory(parent, label)?;
    crate::mutate::workspace::require_direct_file(parent, path, label)?;
    before_open()?;
    let mut file = crate::mutate::workspace::open_real_file(path, label)?;
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
    crate::mutate::workspace::require_direct_file(parent, path, label)?;
    Ok(bytes)
}

pub(crate) fn read_bounded_text(path: &Path, maximum: u64, label: &str) -> Result<String> {
    String::from_utf8(read_bounded_file(path, maximum, label)?)
        .with_context(|| format!("{label} {} is not UTF-8", path.display()))
}

pub(crate) fn boot_image_bytes() -> u64 {
    crate::paths::DISK_MIB * 1024 * 1024
}

pub(crate) fn namespace_bytes() -> u64 {
    crate::qemu::NVME_VOLUME_SECTOR * crate::qemu::NVME_BLOCK_BYTES as u64
        + crate::image::corpus::VOLUME_BYTES as u64
}

fn copy_exact(source: &Path, destination: &Path, expected: &Digest) -> Result<()> {
    require_regular_file(source, "qualification source artifact")?;
    let mut input =
        crate::mutate::workspace::open_real_file(source, "qualification source artifact")?;
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

#[derive(Clone, PartialEq, Eq, Debug)]
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
        let mut file = crate::mutate::workspace::open_real_file(&path, "tracked model input")?;
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

fn find_on_path(stem: &str) -> Option<PathBuf> {
    let name = executable_name(stem);
    let path = std::env::var_os("PATH").unwrap_or_default();
    crate::qemu::which_in(std::env::split_paths(&path), &name)
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
mod tests;
