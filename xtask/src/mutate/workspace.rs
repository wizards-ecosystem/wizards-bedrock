//! Crash-safe isolation and progress for mutation sweeps.
//!
//! Source the harness lays down is written, never copied. `fs::copy` carries
//! the snapshot timestamp with it, and a source older than the object built
//! from the mutation is exactly what Cargo declines to rebuild (D-237).

mod journal;
mod source;
mod tree;

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::paths::Paths;
use crate::sha256;

use journal::{Journal, JournalStore, Report, ReportVerdict, is_digest, is_hex};
use source::{
    Baseline, copy_tracked, digest_tracked, require_confined_file, require_mutation_layout,
    require_real_tree, restore_workspace, validate_relative, write_source,
};
pub(crate) use source::{
    copy_clean_tracked_to, finish_staged_operation, publish_staged_file, with_clean_source_copy,
};
use tree::{SweepLock, acquire_lock, require_real_child};
pub(crate) use tree::{
    canonical_existing_real_directory, metadata_is_real_directory, open_real_directory,
    open_real_file, prepare_real_child, prepare_real_directory_tree, require_direct_file,
    require_direct_file_if_present,
};

const LOCK_NAME: &str = "LOCK";
const STATE_ROOT_NAME: &str = "mutate";
const MAX_STATE_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// One completed mutation's exact outcome.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Its named check caught it.
    Caught,
    /// Its named check stayed green.
    Survived,
    /// The mutation did not reach a conclusive verdict.
    Inconclusive(String),
}

/// Identity of a current successful mutation sweep.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct PassEvidence {
    pub(crate) commit: String,
    pub(crate) registry: String,
    pub(crate) source_digest: String,
    pub(crate) index_digest: String,
    pub(crate) mutation_count: usize,
    pub(crate) report_digest: String,
    pub(crate) report_path: PathBuf,
}

#[derive(Debug)]
pub(crate) struct CurrentPass {
    evidence: PassEvidence,
    _lock: SweepLock,
}

impl CurrentPass {
    pub(crate) fn evidence(&self) -> &PassEvidence {
        &self.evidence
    }

    #[cfg(test)]
    pub(crate) fn fixture(lock_path: &Path) -> Result<Self> {
        Ok(Self {
            evidence: PassEvidence {
                commit: String::new(),
                registry: String::new(),
                source_digest: String::new(),
                index_digest: String::new(),
                mutation_count: 0,
                report_digest: String::new(),
                report_path: PathBuf::new(),
            },
            _lock: acquire_lock(lock_path)?,
        })
    }
}

/// One isolated, resumable sweep.
pub struct Session {
    _lock: SweepLock,
    baseline: Baseline,
    snapshot: PathBuf,
    workspace: Paths,
    store: JournalStore,
    journal: Journal,
}

impl Session {
    /// Open a fresh sweep or resume the interrupted sweep for this commit and
    /// registry.
    ///
    /// # Errors
    ///
    /// If another sweep is live, the checkout is not clean, its source/index
    /// digest changed, or the durable state is malformed.
    pub fn open(paths: &Paths, registry: &str, mutation_count: usize) -> Result<Self> {
        ensure!(is_digest(registry), "mutation registry digest is malformed");
        let state_root = prepare_state_root(paths.target_dir())?;
        let lock = acquire_lock(&state_root.join(LOCK_NAME))?;
        let baseline = Baseline::discover(paths.root())?;
        let key = format!("{}-{}", &baseline.commit[..12], &registry[..12]);
        let state = prepare_real_child(&state_root, &state_root.join(key), "mutation state")?;
        let snapshot = state.join("source");
        let workspace_root = state.join("workspace");
        let build = state.join("build");
        let store = JournalStore::new(state)?;

        let journal = match store.load()? {
            Some(journal) => {
                journal.matches(&baseline, registry, mutation_count)?;
                require_mutation_layout(&snapshot, &workspace_root)?;
                require_real_tree(&snapshot, "mutation source snapshot")?;
                ensure!(
                    digest_tracked(&snapshot, &baseline.tracked)? == baseline.source_digest,
                    "the resumable source snapshot changed"
                );
                journal
            }
            None => {
                prepare_real_child(&store.directory, &snapshot, "mutation source snapshot")?;
                prepare_real_child(&store.directory, &workspace_root, "mutation workspace")?;
                copy_tracked(paths.root(), &snapshot, &baseline.tracked)?;
                copy_tracked(&snapshot, &workspace_root, &baseline.tracked)?;
                ensure!(
                    digest_tracked(&snapshot, &baseline.tracked)? == baseline.source_digest,
                    "the copied source snapshot differs from the checkout"
                );
                ensure!(
                    digest_tracked(&workspace_root, &baseline.tracked)? == baseline.source_digest,
                    "the mutation workspace differs from its source snapshot"
                );
                let mut journal = Journal::new(&baseline, registry);
                store.save(&mut journal)?;
                journal
            }
        };
        prepare_real_child(&store.directory, &build, "mutation build directory")?;

        let session = Self {
            _lock: lock,
            baseline,
            snapshot,
            workspace: Paths::new(workspace_root, Some(&build)),
            store,
            journal,
        };
        restore_workspace(
            &session.snapshot,
            session.workspace.root(),
            &session.baseline.tracked,
        )?;
        Ok(session)
    }

    /// Paths every build and boot in the sweep must use.
    pub fn paths(&self) -> &Paths {
        &self.workspace
    }

    /// Outcomes already made durable by an earlier process.
    pub fn completed(&self) -> &[Outcome] {
        &self.journal.completed
    }

    /// The mutation which may have been left applied when the runner died.
    pub fn active(&self) -> Option<usize> {
        self.journal.active
    }

    /// Restore an interrupted mutation from the immutable snapshot so it can
    /// be rerun.
    ///
    /// # Errors
    ///
    /// If the journal and registry disagree or exact restoration fails.
    pub fn recover(&mut self, index: usize, relative: &str) -> Result<()> {
        validate_relative(relative)?;
        ensure!(
            self.journal.active == Some(index),
            "another mutation is active"
        );
        ensure!(
            index == self.journal.completed.len(),
            "active progress is not contiguous"
        );
        restore_workspace(
            &self.snapshot,
            self.workspace.root(),
            &self.baseline.tracked,
        )?;
        self.journal.active = None;
        self.store.save(&mut self.journal)
    }

    /// Read one pristine tracked file from the commit-pinned snapshot.
    pub fn original(&self, relative: &str) -> Result<String> {
        require_mutation_layout(&self.snapshot, self.workspace.root())?;
        let path = require_confined_file(&self.snapshot, relative, "mutation source file")?;
        read_state_text(&path, "mutation source file")
    }

    /// Make `index` durable as the one which may be left applied.
    pub fn begin(&mut self, index: usize) -> Result<()> {
        ensure!(
            self.journal.active.is_none(),
            "a mutation is already active"
        );
        ensure!(
            index == self.journal.completed.len(),
            "mutation progress is not contiguous"
        );
        self.journal.active = Some(index);
        self.store.save(&mut self.journal)
    }

    /// Write a mutation only into the isolated workspace.
    pub fn write(&self, relative: &str, mutated: &str) -> Result<()> {
        require_mutation_layout(&self.snapshot, self.workspace.root())?;
        let path =
            require_confined_file(self.workspace.root(), relative, "mutation workspace file")?;
        write_source(&path, mutated.as_bytes())
            .with_context(|| format!("writing isolated mutation {}", path.display()))
    }

    /// Restore the active file exactly, then make its outcome durable.
    pub fn finish(&mut self, index: usize, relative: &str, outcome: Outcome) -> Result<()> {
        validate_relative(relative)?;
        ensure!(
            self.journal.active == Some(index),
            "another mutation is active"
        );
        restore_workspace(
            &self.snapshot,
            self.workspace.root(),
            &self.baseline.tracked,
        )?;
        self.journal.completed.push(outcome);
        self.journal.active = None;
        self.store.save(&mut self.journal)
    }

    /// Verify the source and index, write the final summary and retire the
    /// resumable journal.
    ///
    /// `ids` are the registry's permanent identities in sweep order, so the
    /// durable report names each outcome's mutation rather than its position.
    pub fn complete(mut self, ids: &[u16]) -> Result<Vec<Outcome>> {
        ensure!(self.journal.active.is_none(), "a mutation is still active");
        ensure!(
            self.journal.completed.len() == ids.len(),
            "only {} of {} mutations completed",
            self.journal.completed.len(),
            ids.len()
        );
        restore_workspace(
            &self.snapshot,
            self.workspace.root(),
            &self.baseline.tracked,
        )?;
        ensure!(
            digest_tracked(&self.snapshot, &self.baseline.tracked)? == self.baseline.source_digest,
            "the immutable mutation source snapshot changed during the sweep"
        );
        self.baseline.verify()?;

        let report = self.store.save_report(Report::from(&self.journal, ids))?;
        self.store.write_last_run(&report)?;
        self.store.clear()?;
        Ok(std::mem::take(&mut self.journal.completed))
    }
}

/// Require the newest completed sweep for this clean checkout to be green.
pub(crate) fn current_pass(
    paths: &Paths,
    registry: &str,
    mutation_count: usize,
) -> Result<CurrentPass> {
    ensure!(is_digest(registry), "mutation registry digest is malformed");
    let state_root = prepare_state_root(paths.target_dir())?;
    let pass_lock = acquire_lock(&state_root.join(LOCK_NAME))?;
    let baseline = Baseline::discover(paths.root())?;
    let key = format!("{}-{}", &baseline.commit[..12], &registry[..12]);
    let state = state_root.join(key);
    require_real_child(&state_root, &state, "mutation state")?;
    let store = JournalStore::new(state)?;
    ensure!(
        store.load()?.is_none(),
        "the current mutation sweep is unfinished"
    );
    let (report_path, report) = store
        .latest_report()?
        .context("the current commit and mutation registry have no completed sweep")?;
    report.require_pass(&baseline, registry, mutation_count)?;
    let report_text = read_state_text(&report_path, "mutation report")?;
    let report_digest = bind_current_report(&report, report_text.as_bytes())?;
    Ok(CurrentPass {
        evidence: PassEvidence {
            commit: report.commit,
            registry: report.registry,
            source_digest: report.source_digest,
            index_digest: report.index_digest,
            mutation_count: report.outcomes.len(),
            report_digest,
            report_path,
        },
        _lock: pass_lock,
    })
}

pub(crate) fn recorded_pass(
    paths: &Paths,
    commit: &str,
    registry: &str,
    source_digest: &str,
    index_digest: &str,
    mutation_count: usize,
    expected_report_digest: &str,
) -> Result<CurrentPass> {
    ensure!(is_hex(commit, 40), "recorded mutation commit is malformed");
    ensure!(
        is_digest(registry),
        "recorded mutation registry is malformed"
    );
    let state_root = prepare_state_root(paths.target_dir())?;
    let pass_lock = acquire_lock(&state_root.join(LOCK_NAME))?;
    let key = format!("{}-{}", &commit[..12], &registry[..12]);
    let state = state_root.join(key);
    require_real_child(&state_root, &state, "recorded mutation state")?;
    let store = JournalStore::new(state)?;
    ensure!(
        store.load()?.is_none(),
        "the recorded mutation sweep is unfinished"
    );
    let (report_path, report) = store
        .latest_report()?
        .context("the recorded commit and registry have no completed sweep")?;
    let report_text = read_state_text(&report_path, "recorded mutation report")?;
    let report_digest = bind_current_report(&report, report_text.as_bytes())?;
    ensure!(
        report.commit == commit
            && report.registry == registry
            && report.source_digest == source_digest
            && report.index_digest == index_digest
            && report.outcomes.len() == mutation_count
            && report_digest == expected_report_digest,
        "newest recorded mutation report differs from the qualification record"
    );
    ensure!(
        report.verdict == ReportVerdict::Pass
            && report.caught == mutation_count
            && report.survived == 0
            && report.inconclusive == 0,
        "the newest recorded mutation report did not pass"
    );
    Ok(CurrentPass {
        evidence: PassEvidence {
            commit: report.commit,
            registry: report.registry,
            source_digest: report.source_digest,
            index_digest: report.index_digest,
            mutation_count: report.outcomes.len(),
            report_digest,
            report_path,
        },
        _lock: pass_lock,
    })
}

pub(crate) fn verify_recorded_pass(
    path: &Path,
    commit: &str,
    registry: &str,
    source_digest: &str,
    index_digest: &str,
    mutation_count: usize,
) -> Result<()> {
    let report = Report::parse(&read_state_text(path, "recorded mutation report")?)?;
    ensure!(
        report.commit == commit
            && report.registry == registry
            && report.source_digest == source_digest
            && report.index_digest == index_digest
            && report.outcomes.len() == mutation_count,
        "recorded mutation report belongs to different qualification inputs"
    );
    ensure!(
        report.verdict == ReportVerdict::Pass
            && report.caught == mutation_count
            && report.survived == 0
            && report.inconclusive == 0,
        "recorded mutation report did not pass"
    );
    Ok(())
}

fn bind_current_report(selected: &Report, bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).context("mutation report is not UTF-8")?;
    ensure!(
        Report::parse(text)? == *selected,
        "mutation report changed while selecting the current pass"
    );
    Ok(sha256::hex(&sha256::digest(bytes)))
}

fn read_state_text(path: &Path, label: &str) -> Result<String> {
    let bytes = read_bounded_bytes(path, MAX_STATE_FILE_BYTES, label)?;
    String::from_utf8(bytes).with_context(|| format!("{label} {} is not UTF-8", path.display()))
}

fn read_bounded_bytes(path: &Path, maximum: u64, label: &str) -> Result<Vec<u8>> {
    read_bounded_bytes_with(path, maximum, label, || Ok(()))
}

fn read_bounded_bytes_with<F>(
    path: &Path,
    maximum: u64,
    label: &str,
    after_measurement: F,
) -> Result<Vec<u8>>
where
    F: FnOnce() -> Result<()>,
{
    let mut file = open_real_file(path, label)?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding the {maximum}-byte allocation limit"
    );
    after_measurement()?;
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
        bytes.len() as u64 == initial
            && file
                .metadata()
                .with_context(|| format!("remeasuring {label} {}", path.display()))?
                .len()
                == initial,
        "{label} changed length while being read"
    );
    Ok(bytes)
}

fn prepare_state_root(target: &Path) -> Result<PathBuf> {
    let target = prepare_real_directory_tree(target, "mutation target directory")?;
    prepare_real_child(
        &target,
        &target.join(STATE_ROOT_NAME),
        "mutation state root",
    )
}

#[cfg(test)]
mod tests;
