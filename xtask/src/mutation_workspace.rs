//! Crash-safe isolation and progress for mutation sweeps.
//!
//! Source the harness lays down is written, never copied. `fs::copy` carries
//! the snapshot timestamp with it, and a source older than the object built
//! from the mutation is exactly what Cargo declines to rebuild (D-237).

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;
use crate::sha256;

const JOURNAL_MAGIC: &str = "wizard-mutation-journal-v1";
const REPORT_MAGIC: &str = "wizard-mutation-report-v1";
const LOCK_NAME: &str = "LOCK";
const JOURNAL_SLOTS: [&str; 2] = ["journal.0", "journal.1"];
const REPORT_PREFIX: &str = "run.";
const REPORT_SUFFIX: &str = ".report";
const LAST_RUN: &str = "last-run";
const STATE_ROOT_NAME: &str = "mutate";
const MAX_STATE_FILE_BYTES: u64 = 16 * 1024 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_GIT_IDENTITY_BYTES: usize = 4096;
const MAX_GIT_STATUS_BYTES: usize = 1024 * 1024;
const MAX_GIT_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRACKED_PATH_BYTES: usize = 4 * 1024 * 1024;
const MAX_TRACKED_FILES: usize = 4096;
const MAX_TRACKED_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRACKED_TREE_BYTES: u64 = 64 * 1024 * 1024;
static NEXT_STORE_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NEXT_SOURCE_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One completed mutation's exact outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Its named check caught it.
    Caught,
    /// Its named check stayed green.
    Survived,
    /// The mutation did not reach a conclusive verdict.
    Inconclusive(String),
}

/// Identity of a current successful mutation sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub fn complete(mut self, expected: usize) -> Result<Vec<Outcome>> {
        ensure!(self.journal.active.is_none(), "a mutation is still active");
        ensure!(
            self.journal.completed.len() == expected,
            "only {} of {expected} mutations completed",
            self.journal.completed.len()
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

        let report = self.store.save_report(Report::from(&self.journal))?;
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
    let mut file =
        File::open(path).with_context(|| format!("opening {label} {}", path.display()))?;
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

pub(crate) fn prepare_real_directory_tree(path: &Path, label: &str) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("reading the current directory")?
            .join(path)
    };
    let mut missing = Vec::new();
    let mut cursor = absolute.as_path();
    loop {
        match std::fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                require_real_directory_metadata(cursor, &metadata, label)?;
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor
                    .parent()
                    .with_context(|| format!("{label} has no existing ancestor"))?;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", cursor.display()));
            }
        }
    }
    for ancestor in cursor.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(ancestor)
            .with_context(|| format!("inspecting ancestor {}", ancestor.display()))?;
        require_real_directory_metadata(ancestor, &metadata, label)?;
    }
    for directory in missing.into_iter().rev() {
        match std::fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("creating {}", directory.display()));
            }
        }
        let metadata = std::fs::symlink_metadata(&directory)
            .with_context(|| format!("inspecting {}", directory.display()))?;
        require_real_directory_metadata(&directory, &metadata, label)?;
        let parent = directory
            .parent()
            .with_context(|| format!("{label} has no parent"))?;
        require_canonical_child(parent, &directory, label)?;
    }
    let canonical = std::fs::canonicalize(&absolute)
        .with_context(|| format!("canonicalizing {}", absolute.display()))?;
    let metadata = std::fs::symlink_metadata(&canonical)
        .with_context(|| format!("inspecting {}", canonical.display()))?;
    require_real_directory_metadata(&canonical, &metadata, label)?;
    Ok(canonical)
}

pub(crate) fn canonical_existing_real_directory(path: &Path, label: &str) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("reading the current directory")?
            .join(path)
    };
    for ancestor in absolute.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(ancestor)
            .with_context(|| format!("inspecting ancestor {}", ancestor.display()))?;
        require_real_directory_metadata(ancestor, &metadata, label)?;
    }
    std::fs::canonicalize(&absolute)
        .with_context(|| format!("canonicalizing {}", absolute.display()))
}

pub(crate) fn prepare_real_child(parent: &Path, child: &Path, label: &str) -> Result<PathBuf> {
    ensure!(
        child.parent() == Some(parent),
        "{label} is not an exact lexical child of {}",
        parent.display()
    );
    match std::fs::symlink_metadata(child) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(child)
                .with_context(|| format!("creating {label} {}", child.display()))?;
        }
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", child.display()));
        }
    }
    require_real_child(parent, child, label)?;
    std::fs::canonicalize(child).with_context(|| format!("canonicalizing {}", child.display()))
}

fn require_real_child(parent: &Path, child: &Path, label: &str) -> Result<()> {
    ensure!(
        child.parent() == Some(parent),
        "{label} is not an exact lexical child of {}",
        parent.display()
    );
    let metadata = std::fs::symlink_metadata(child)
        .with_context(|| format!("inspecting {}", child.display()))?;
    require_real_directory_metadata(child, &metadata, label)?;
    require_canonical_child(parent, child, label)
}

fn require_canonical_child(parent: &Path, child: &Path, label: &str) -> Result<()> {
    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalizing {}", parent.display()))?;
    let canonical_child = std::fs::canonicalize(child)
        .with_context(|| format!("canonicalizing {}", child.display()))?;
    ensure!(
        canonical_child.parent() == Some(canonical_parent.as_path()),
        "{label} escaped its canonical parent"
    );
    Ok(())
}

fn require_real_directory_metadata(
    path: &Path,
    metadata: &std::fs::Metadata,
    label: &str,
) -> Result<()> {
    ensure!(
        metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && !is_reparse_point(metadata),
        "{label} is not a real directory: {}",
        path.display()
    );
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
const fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[derive(Debug)]
struct SweepLock {
    _file: File,
}

fn acquire_lock(path: &Path) -> Result<SweepLock> {
    let parent = path.parent().context("mutation lock has no parent")?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .with_context(|| format!("inspecting {}", parent.display()))?;
    require_real_directory_metadata(parent, &parent_metadata, "mutation lock parent")?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => require_real_file_metadata(path, &metadata, "mutation lock")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", path.display()));
        }
    }
    let mut options = OpenOptions::new();
    OpenOptions::create(&mut options, true);
    OpenOptions::truncate(&mut options, false);
    OpenOptions::read(&mut options, true);
    OpenOptions::write(&mut options, true);
    let file = OpenOptions::open(&options, path)
        .with_context(|| format!("opening mutation lock {}", path.display()))?;
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspecting {}", path.display()))?;
    require_real_file_metadata(path, &metadata, "mutation lock")?;
    require_canonical_child(parent, path, "mutation lock")?;
    file.try_lock().with_context(|| {
        format!(
            "another mutation sweep holds {}; wait for it or stop it before resuming",
            path.display()
        )
    })?;
    Ok(SweepLock { _file: file })
}

fn require_real_file_metadata(
    path: &Path,
    metadata: &std::fs::Metadata,
    label: &str,
) -> Result<()> {
    ensure!(
        metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && !is_reparse_point(metadata),
        "{label} is not a real file: {}",
        path.display()
    );
    Ok(())
}

#[derive(Debug)]
struct Baseline {
    root: PathBuf,
    commit: String,
    tracked: Vec<PathBuf>,
    source_digest: String,
    index_path: PathBuf,
    index_digest: String,
}

fn parse_tracked_paths(listed: &[u8]) -> Result<Vec<PathBuf>> {
    let mut tracked = Vec::new();
    let mut previous = None;
    for raw in listed
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        if let Some(previous) = previous {
            ensure!(
                previous < raw,
                "Git tracked paths are not unique and sorted"
            );
        }
        previous = Some(raw);
        let text = std::str::from_utf8(raw).context("a tracked path is not UTF-8")?;
        validate_relative(text)?;
        tracked.push(PathBuf::from(text));
    }
    ensure!(!tracked.is_empty(), "Git reports no tracked files");
    ensure!(
        tracked.len() <= MAX_TRACKED_FILES,
        "Git reports {} tracked files, exceeding the {MAX_TRACKED_FILES}-file mutation limit",
        tracked.len()
    );
    Ok(tracked)
}

impl Baseline {
    fn discover(root: &Path) -> Result<Self> {
        let status = git(
            root,
            ["status", "--porcelain=v1", "--untracked-files=all"],
            MAX_GIT_STATUS_BYTES,
            "mutation source status",
        )?;
        ensure!(
            status.is_empty(),
            "mutation sweeps require a clean checkout; git reports:\n{}",
            String::from_utf8_lossy(&status)
        );
        let commit = git_text(
            root,
            ["rev-parse", "--verify", "HEAD"],
            MAX_GIT_IDENTITY_BYTES,
            "mutation source commit",
        )?;
        ensure!(is_hex(&commit, 40), "HEAD is not a full Git object id");
        let listed = git(
            root,
            ["ls-files", "-z"],
            MAX_TRACKED_PATH_BYTES,
            "mutation tracked-path inventory",
        )?;
        let tracked = parse_tracked_paths(&listed)?;

        let index_name = git_text(
            root,
            ["rev-parse", "--git-path", "index"],
            MAX_GIT_IDENTITY_BYTES,
            "mutation Git index path",
        )?;
        let index_path = require_confined_file(root, &index_name, "mutation Git index")?;
        let index_digest = digest_file(&index_path, MAX_GIT_INDEX_BYTES, "Git index")?;
        let source_digest = digest_tracked(root, &tracked)?;
        let settled_status = git(
            root,
            ["status", "--porcelain=v1", "--untracked-files=all"],
            MAX_GIT_STATUS_BYTES,
            "settled mutation source status",
        )?;
        ensure!(
            settled_status.is_empty(),
            "mutation source changed while its baseline was captured; git reports:\n{}",
            String::from_utf8_lossy(&settled_status)
        );
        ensure!(
            git_text(
                root,
                ["rev-parse", "--verify", "HEAD"],
                MAX_GIT_IDENTITY_BYTES,
                "settled mutation source commit",
            )? == commit,
            "mutation source commit changed while its baseline was captured"
        );
        ensure!(
            digest_file(&index_path, MAX_GIT_INDEX_BYTES, "settled Git index")? == index_digest,
            "Git index changed while the mutation baseline was captured"
        );
        Ok(Self {
            root: root.to_path_buf(),
            commit,
            tracked,
            source_digest,
            index_path,
            index_digest,
        })
    }

    fn verify(&self) -> Result<()> {
        ensure!(
            digest_tracked(&self.root, &self.tracked)? == self.source_digest,
            "the source checkout changed during the mutation sweep"
        );
        ensure!(
            digest_file(&self.index_path, MAX_GIT_INDEX_BYTES, "Git index")? == self.index_digest,
            "the Git index changed during the mutation sweep"
        );
        Ok(())
    }
}

fn git<const N: usize>(
    root: &Path,
    args: [&str; N],
    maximum: usize,
    label: &str,
) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    crate::bounded_process::isolate_git_environment(&mut command);
    command.current_dir(root).args(args);
    let output = crate::bounded_process::output_with_limit(command, GIT_TIMEOUT, label, maximum)
        .with_context(|| format!("running {label}"))?;
    if !output.status.success() {
        bail!(
            "Git query failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    ensure!(
        output.stderr.iter().all(u8::is_ascii_whitespace),
        "{label} printed unexpected diagnostics"
    );
    Ok(output.stdout)
}

fn git_text<const N: usize>(
    root: &Path,
    args: [&str; N],
    maximum: usize,
    label: &str,
) -> Result<String> {
    let bytes = git(root, args, maximum, label)?;
    Ok(std::str::from_utf8(&bytes)
        .context("Git returned non-UTF-8 text")?
        .trim()
        .to_string())
}

fn copy_tracked(from: &Path, to: &Path, tracked: &[PathBuf]) -> Result<()> {
    let root = canonical_existing_real_directory(to, "mutation copy root")?;
    for relative in tracked {
        let relative = relative
            .to_str()
            .context("a tracked mutation path is not UTF-8")?;
        let source = require_confined_file(from, relative, "mutation copy source")?;
        let (_, destination) =
            prepare_confined_destination(&root, relative, "mutation copy destination")?;
        let bytes = read_bounded_bytes(&source, MAX_TRACKED_FILE_BYTES, "tracked mutation file")?;
        write_source(&destination, &bytes)?;
    }
    Ok(())
}

/// Write a source file the way an editor would, so the build system sees it
/// change.
///
/// `std::fs::copy` carries the source's timestamp onto the destination, which
/// moves a restored file *backwards* in time — behind the object built from
/// the mutation. Cargo calls that fresh and keeps the defect, in the package
/// and in everything that inlined it (D-237).
fn write_source(destination: &Path, bytes: &[u8]) -> Result<()> {
    write_source_with(destination, bytes, |from, to| std::fs::rename(from, to))
}

fn write_source_with(
    destination: &Path,
    bytes: &[u8],
    publish: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    let parent = destination
        .parent()
        .context("mutation source destination has no parent")?;
    ensure!(
        destination.parent() == Some(parent),
        "mutation source destination escaped its parent"
    );

    let mut reserved = None;
    for _ in 0..64 {
        let nonce = NEXT_SOURCE_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = parent.join(format!(
            ".mutation-source-{}-{nonce:016x}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::write(&mut options, true);
        match OpenOptions::open(&options, &path) {
            Ok(file) => {
                require_direct_file(parent, &path, "temporary mutation source")?;
                reserved = Some((path, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("creating temporary source {}", path.display()));
            }
        }
    }
    let (temporary, mut file) =
        reserved.context("could not reserve a temporary mutation source")?;
    let result = (|| {
        file.write_all(bytes)
            .with_context(|| format!("writing {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", temporary.display()))?;
        drop(file);

        publish_staged_file_with(
            parent,
            &temporary,
            destination,
            "mutation source",
            publish,
            |published| {
                ensure!(
                    read_bounded_bytes(
                        published,
                        MAX_TRACKED_FILE_BYTES,
                        "published mutation source",
                    )? == bytes,
                    "published mutation source differs from its requested bytes"
                );
                Ok(())
            },
        )?;
        Ok(())
    })();
    finish_staged_operation(parent, &temporary, "temporary mutation source", result)
}

pub(crate) fn finish_staged_operation<T>(
    parent: &Path,
    staging: &Path,
    label: &str,
    result: Result<T>,
) -> Result<T> {
    finish_staged_operation_with(parent, staging, label, result, |path| {
        std::fs::remove_file(path)
    })
}

fn finish_staged_operation_with<T>(
    parent: &Path,
    staging: &Path,
    label: &str,
    result: Result<T>,
    remove: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(operation_error) => {
            if require_direct_file_if_present(parent, staging, label).with_context(|| {
                format!(
                    "after {operation_error:#}; validating failed {label} {}",
                    staging.display()
                )
            })? {
                remove(staging).with_context(|| {
                    format!(
                        "after {operation_error:#}; removing failed {label} {}",
                        staging.display()
                    )
                })?;
            }
            Err(operation_error)
        }
    }
}

pub(crate) fn publish_staged_file(
    parent: &Path,
    staging: &Path,
    destination: &Path,
    label: &str,
    validate: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    publish_staged_file_with(
        parent,
        staging,
        destination,
        label,
        |from, to| std::fs::rename(from, to),
        validate,
    )
}

fn publish_staged_file_with(
    parent: &Path,
    staging: &Path,
    destination: &Path,
    label: &str,
    publish: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    validate: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    ensure!(
        staging.parent() == Some(parent) && destination.parent() == Some(parent),
        "{label} publication paths are not exact siblings"
    );
    let canonical_parent = canonical_existing_real_directory(parent, label)?;
    require_direct_file(parent, staging, &format!("staged {label}"))?;
    let previous = if require_direct_file_if_present(parent, destination, label)? {
        let backup = unused_source_path(parent, "backup")?;
        std::fs::rename(destination, &backup).with_context(|| {
            format!(
                "retaining old {label} {} as {}",
                destination.display(),
                backup.display()
            )
        })?;
        require_direct_file(parent, &backup, &format!("retained {label}"))?;
        Some(backup)
    } else {
        None
    };
    let result = (|| {
        ensure!(
            canonical_existing_real_directory(parent, label)? == canonical_parent,
            "{label} parent changed before publication"
        );
        ensure!(
            !require_direct_file_if_present(parent, destination, label)?,
            "{label} destination reappeared before publication"
        );
        publish(staging, destination).with_context(|| {
            format!(
                "publishing staged {label} {} as {}",
                staging.display(),
                destination.display()
            )
        })?;
        require_direct_file(parent, destination, &format!("published {label}"))?;
        validate(destination)
    })();
    if let Err(error) = result {
        restore_previous_file(parent, destination, previous.as_deref(), label)
            .with_context(|| format!("{error:#}; rolling back failed {label} publication"))?;
        return Err(error);
    }
    if let Some(backup) = previous
        && let Err(error) = std::fs::remove_file(&backup)
    {
        restore_previous_file(parent, destination, Some(&backup), label)
            .with_context(|| format!("{error}; rolling back uncommitted {label} publication"))?;
        return Err(error)
            .with_context(|| format!("removing retained {label} {}", backup.display()));
    }
    Ok(())
}

fn unused_source_path(parent: &Path, kind: &str) -> Result<PathBuf> {
    for _ in 0..64 {
        let nonce = NEXT_SOURCE_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = parent.join(format!(
            ".mutation-{kind}-{}-{nonce:016x}",
            std::process::id()
        ));
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspecting reserved path {}", path.display()));
            }
        }
    }
    bail!("could not reserve a mutation source {kind} path")
}

fn restore_previous_file(
    parent: &Path,
    destination: &Path,
    previous: Option<&Path>,
    label: &str,
) -> Result<()> {
    if require_direct_file_if_present(parent, destination, &format!("failed {label} publication"))?
    {
        std::fs::remove_file(destination)
            .with_context(|| format!("removing failed publication {}", destination.display()))?;
    }
    let Some(backup) = previous else {
        return Ok(());
    };
    std::fs::rename(backup, destination).with_context(|| {
        format!(
            "restoring retained source {} as {}",
            backup.display(),
            destination.display()
        )
    })?;
    require_direct_file(parent, destination, &format!("restored {label}"))?;
    Ok(())
}

fn restore_file(snapshot: &Path, workspace: &Path, relative: &str) -> Result<()> {
    let source = require_confined_file(snapshot, relative, "mutation snapshot source")?;
    let bytes = read_bounded_bytes(&source, MAX_TRACKED_FILE_BYTES, "mutation snapshot")?;
    let workspace = canonical_existing_real_directory(workspace, "mutation workspace")?;
    let (parent, destination) =
        prepare_confined_destination(&workspace, relative, "mutation restore destination")?;
    if require_direct_file_if_present(&parent, &destination, "mutation restore destination")?
        && read_direct_file(&parent, &destination, "mutation restore destination")? == bytes
    {
        return Ok(());
    }
    write_source(&destination, &bytes)?;
    ensure!(
        read_bounded_bytes(
            &destination,
            MAX_TRACKED_FILE_BYTES,
            "restored mutation source",
        )? == bytes,
        "restored mutation file differs from its snapshot"
    );
    Ok(())
}

fn prepare_confined_destination(
    root: &Path,
    relative: &str,
    label: &str,
) -> Result<(PathBuf, PathBuf)> {
    validate_relative(relative)?;
    let canonical_root = canonical_existing_real_directory(root, label)?;
    let mut parent = canonical_root;
    let mut components = Path::new(relative).components().peekable();
    while let Some(component) = core::iter::Iterator::next(&mut components) {
        let Component::Normal(name) = component else {
            unreachable!("validate_relative accepted a non-normal component");
        };
        let child = parent.join(name);
        if components.peek().is_some() {
            parent = prepare_real_child(&parent, &child, label)?;
        } else {
            return Ok((parent, child));
        }
    }
    unreachable!("validate_relative accepted an empty path")
}

fn read_direct_file(parent: &Path, path: &Path, label: &str) -> Result<Vec<u8>> {
    require_direct_file(parent, path, label)?;
    let bytes = read_bounded_bytes(path, MAX_TRACKED_FILE_BYTES, label)?;
    require_direct_file(parent, path, label)?;
    Ok(bytes)
}

fn restore_workspace(snapshot: &Path, workspace: &Path, tracked: &[PathBuf]) -> Result<()> {
    require_mutation_layout(snapshot, workspace)?;
    require_real_tree(snapshot, "mutation source snapshot")?;

    clean_workspace(workspace, Path::new(""), tracked)?;
    for relative in tracked {
        let relative = relative
            .to_str()
            .context("a tracked workspace path is not UTF-8")?;
        restore_file(snapshot, workspace, relative)?;
    }
    ensure!(
        digest_tracked(workspace, tracked)? == digest_tracked(snapshot, tracked)?,
        "the restored mutation workspace differs from its snapshot"
    );
    verify_workspace_layout(workspace, Path::new(""), tracked)
}

fn require_mutation_layout(snapshot: &Path, workspace: &Path) -> Result<()> {
    ensure!(
        snapshot.file_name().and_then(|name| name.to_str()) == Some("source")
            && workspace.file_name().and_then(|name| name.to_str()) == Some("workspace")
            && snapshot.parent() == workspace.parent()
            && snapshot.parent().and_then(Path::parent).is_some(),
        "refusing to sanitize paths outside one mutation state directory"
    );
    let state = snapshot
        .parent()
        .context("mutation snapshot has no state directory")?;
    let state_root = state.parent().context("mutation state has no state root")?;
    let target = state_root
        .parent()
        .context("mutation state root has no target directory")?;
    ensure!(
        state_root.file_name().and_then(|name| name.to_str()) == Some(STATE_ROOT_NAME),
        "mutation state root is not the exact `{STATE_ROOT_NAME}` directory"
    );
    let key = state
        .file_name()
        .and_then(|name| name.to_str())
        .context("mutation state key is not UTF-8")?;
    ensure!(valid_state_key(key), "mutation state key is malformed");

    let target_metadata = std::fs::symlink_metadata(target)
        .with_context(|| format!("inspecting {}", target.display()))?;
    require_real_directory_metadata(target, &target_metadata, "mutation target directory")?;
    for ancestor in target.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(ancestor)
            .with_context(|| format!("inspecting ancestor {}", ancestor.display()))?;
        require_real_directory_metadata(ancestor, &metadata, "mutation target ancestry")?;
    }
    require_real_child(target, state_root, "mutation state root")?;
    require_real_child(state_root, state, "mutation state")?;
    require_real_child(state, snapshot, "mutation source snapshot")?;
    require_real_child(state, workspace, "mutation workspace")
}

fn valid_state_key(key: &str) -> bool {
    let Some((commit, registry)) = key.split_once('-') else {
        return false;
    };
    commit.len() == 12
        && registry.len() == 12
        && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        && registry.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn require_real_tree(root: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)
        .with_context(|| format!("inspecting {}", root.display()))?;
    require_real_directory_metadata(root, &metadata, label)?;
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry.with_context(|| format!("reading an entry in {}", root.display()))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("inspecting {}", path.display()))?;
        ensure!(
            !metadata.file_type().is_symlink() && !is_reparse_point(&metadata),
            "{label} contains a redirected entry: {}",
            path.display()
        );
        if metadata.file_type().is_dir() {
            require_real_tree(&path, label)?;
        } else {
            ensure!(
                metadata.file_type().is_file(),
                "{label} contains a non-file entry: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn clean_workspace(root: &Path, relative: &Path, tracked: &[PathBuf]) -> Result<()> {
    let directory = root.join(relative);
    for entry in
        std::fs::read_dir(&directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let child = relative.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(entry.path())
            .with_context(|| format!("inspecting {}", entry.path().display()))?;
        let expected_file = tracked.iter().any(|path| path == &child);
        let expected_directory = tracked
            .iter()
            .any(|path| path != &child && path.starts_with(&child));

        if expected_directory
            && metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && !is_reparse_point(&metadata)
        {
            clean_workspace(root, &child, tracked)?;
        } else if !(expected_file
            && metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && !is_reparse_point(&metadata))
        {
            remove_workspace_entry(&entry.path(), &metadata)?;
        }
    }
    Ok(())
}

fn remove_workspace_entry(path: &Path, metadata: &std::fs::Metadata) -> Result<()> {
    if metadata.file_type().is_dir()
        && !metadata.file_type().is_symlink()
        && !is_reparse_point(metadata)
    {
        for entry in
            std::fs::read_dir(path).with_context(|| format!("reading {}", path.display()))?
        {
            let entry = entry.with_context(|| format!("reading an entry in {}", path.display()))?;
            let child_metadata = std::fs::symlink_metadata(entry.path())
                .with_context(|| format!("inspecting {}", entry.path().display()))?;
            remove_workspace_entry(&entry.path(), &child_metadata)?;
        }
        std::fs::remove_dir(path)
            .with_context(|| format!("removing unexpected directory {}", path.display()))
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(file_error) => std::fs::remove_dir(path).with_context(|| {
                format!(
                    "removing unexpected entry {} after remove_file failed: {file_error}",
                    path.display()
                )
            }),
        }
    }
}

fn verify_workspace_layout(root: &Path, relative: &Path, tracked: &[PathBuf]) -> Result<()> {
    let directory = root.join(relative);
    for entry in
        std::fs::read_dir(&directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let child = relative.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(entry.path())
            .with_context(|| format!("inspecting {}", entry.path().display()))?;
        let expected_file = tracked.iter().any(|path| path == &child);
        let expected_directory = tracked
            .iter()
            .any(|path| path != &child && path.starts_with(&child));
        ensure!(
            !metadata.file_type().is_symlink()
                && !is_reparse_point(&metadata)
                && ((expected_file && metadata.file_type().is_file())
                    || (expected_directory && metadata.file_type().is_dir())),
            "unexpected entry remained in the mutation workspace: {}",
            child.display()
        );
        if expected_directory {
            verify_workspace_layout(root, &child, tracked)?;
        }
    }
    Ok(())
}

fn require_confined_file(root: &Path, relative: &str, label: &str) -> Result<PathBuf> {
    validate_relative(relative)?;
    let root_metadata = std::fs::symlink_metadata(root)
        .with_context(|| format!("inspecting {}", root.display()))?;
    require_real_directory_metadata(root, &root_metadata, label)?;

    let path = Path::new(relative);
    let mut parent = root.to_path_buf();
    let mut components = path.components().peekable();
    while let Some(component) = core::iter::Iterator::next(&mut components) {
        let Component::Normal(name) = component else {
            unreachable!("validate_relative accepted a non-normal component");
        };
        let child = parent.join(name);
        if components.peek().is_some() {
            require_real_child(&parent, &child, label)?;
            parent = child;
        } else {
            require_direct_file(&parent, &child, label)?;
            return Ok(child);
        }
    }
    unreachable!("validate_relative accepted an empty path")
}

pub(crate) fn require_direct_file(parent: &Path, path: &Path, label: &str) -> Result<()> {
    ensure!(
        require_direct_file_if_present(parent, path, label)?,
        "{label} is missing: {}",
        path.display()
    );
    Ok(())
}

pub(crate) fn require_direct_file_if_present(
    parent: &Path,
    path: &Path,
    label: &str,
) -> Result<bool> {
    ensure!(
        path.parent() == Some(parent),
        "{label} is not an exact lexical child of {}",
        parent.display()
    );
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", path.display()));
        }
    };
    require_real_file_metadata(path, &metadata, label)?;
    require_canonical_child(parent, path, label)?;
    Ok(true)
}

fn validate_relative(relative: &str) -> Result<()> {
    let path = Path::new(relative);
    ensure!(
        !path.as_os_str().is_empty(),
        "an empty tracked path is invalid"
    );
    ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "tracked path is not confined to the workspace: {relative}"
    );
    Ok(())
}

fn digest_tracked(root: &Path, tracked: &[PathBuf]) -> Result<String> {
    ensure!(
        tracked.len() <= MAX_TRACKED_FILES,
        "tracked source contains {} files, exceeding its {MAX_TRACKED_FILES}-file limit",
        tracked.len()
    );
    let mut measured = Vec::with_capacity(tracked.len());
    let mut total = 0u64;
    for relative in tracked {
        let name = relative
            .to_str()
            .context("a tracked mutation path is not UTF-8")?;
        let path = require_confined_file(root, name, "tracked mutation source")?;
        let length = path
            .metadata()
            .with_context(|| format!("measuring tracked file {}", relative.display()))?
            .len();
        ensure!(
            length <= MAX_TRACKED_FILE_BYTES,
            "tracked file {name} is {length} bytes, exceeding its {MAX_TRACKED_FILE_BYTES}-byte limit"
        );
        total = total
            .checked_add(length)
            .context("tracked source length overflowed")?;
        ensure!(
            total <= MAX_TRACKED_TREE_BYTES,
            "tracked source is {total} bytes, exceeding its {MAX_TRACKED_TREE_BYTES}-byte limit"
        );
        measured.push((name, path, length));
    }

    let mut hasher = sha256::Hasher::new();
    for (name, path, length) in measured {
        require_confined_file(root, name, "tracked mutation source")?;
        let mut file = File::open(&path).with_context(|| format!("opening tracked file {name}"))?;
        ensure!(
            file.metadata()
                .with_context(|| format!("remeasuring tracked file {name}"))?
                .len()
                == length,
            "tracked file {name} changed length before hashing"
        );
        hasher
            .update(&(name.len() as u64).to_le_bytes())
            .context("hashing a tracked path length")?;
        hasher
            .update(name.as_bytes())
            .context("hashing a tracked path")?;
        hasher
            .update(&length.to_le_bytes())
            .context("hashing a tracked file length")?;

        let mut observed = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        let mut limited = (&mut file).take(length + 1);
        loop {
            let read = std::io::Read::read(&mut limited, &mut buffer)
                .with_context(|| format!("reading tracked file {name}"))?;
            if read == 0 {
                break;
            }
            observed += read as u64;
            hasher
                .update(&buffer[..read])
                .with_context(|| format!("hashing tracked file {name}"))?;
        }
        ensure!(
            observed == length
                && file
                    .metadata()
                    .with_context(|| format!("remeasuring tracked file {name}"))?
                    .len()
                    == length,
            "tracked file {name} changed length while being hashed"
        );
        require_confined_file(root, name, "tracked mutation source")?;
    }
    Ok(sha256::hex(&hasher.finish()))
}

fn digest_file(path: &Path, maximum: u64, label: &str) -> Result<String> {
    digest_file_with(path, maximum, label, || Ok(()))
}

fn digest_file_with<F>(
    path: &Path,
    maximum: u64,
    label: &str,
    after_measurement: F,
) -> Result<String>
where
    F: FnOnce() -> Result<()>,
{
    let mut file =
        File::open(path).with_context(|| format!("opening {label} {}", path.display()))?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding its {maximum}-byte limit"
    );
    after_measurement()?;
    let (observed, digest) = sha256::digest_reader((&mut file).take(maximum + 1))
        .with_context(|| format!("hashing {label} {}", path.display()))?;
    ensure!(
        observed <= maximum,
        "{label} grew beyond its {maximum}-byte limit while being hashed"
    );
    ensure!(
        observed == initial
            && file
                .metadata()
                .with_context(|| format!("remeasuring {label} {}", path.display()))?
                .len()
                == initial,
        "{label} changed length while being hashed"
    );
    Ok(sha256::hex(&digest))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Journal {
    generation: u64,
    commit: String,
    registry: String,
    source_digest: String,
    index_digest: String,
    active: Option<usize>,
    completed: Vec<Outcome>,
}

impl Journal {
    fn new(baseline: &Baseline, registry: &str) -> Self {
        Self {
            generation: 0,
            commit: baseline.commit.clone(),
            registry: registry.to_string(),
            source_digest: baseline.source_digest.clone(),
            index_digest: baseline.index_digest.clone(),
            active: None,
            completed: Vec::new(),
        }
    }

    fn matches(&self, baseline: &Baseline, registry: &str, mutation_count: usize) -> Result<()> {
        ensure!(
            self.commit == baseline.commit,
            "journal belongs to another commit"
        );
        ensure!(
            self.registry == registry,
            "journal belongs to another mutation registry"
        );
        ensure!(
            self.source_digest == baseline.source_digest,
            "source differs from the interrupted sweep"
        );
        ensure!(
            self.index_digest == baseline.index_digest,
            "index differs from the interrupted sweep"
        );
        ensure!(
            self.completed.len() <= mutation_count,
            "journal contains more outcomes than the registry"
        );
        if let Some(active) = self.active {
            ensure!(
                active == self.completed.len() && active < mutation_count,
                "journal's active mutation is not the next mutation"
            );
        }
        Ok(())
    }

    fn body(&self) -> String {
        let mut body = format!(
            "{JOURNAL_MAGIC}\ngeneration {}\ncommit {}\nregistry {}\nsource {}\nindex {}\nactive {}\ncompleted {}\n",
            self.generation,
            self.commit,
            self.registry,
            self.source_digest,
            self.index_digest,
            self.active
                .map_or_else(|| "none".to_string(), |index| index.to_string()),
            self.completed.len()
        );
        for outcome in &self.completed {
            match outcome {
                Outcome::Caught => body.push_str("result caught\n"),
                Outcome::Survived => body.push_str("result survived\n"),
                Outcome::Inconclusive(error) => {
                    body.push_str("result inconclusive ");
                    body.push_str(&hex_encode(error.as_bytes()));
                    body.push('\n');
                }
            }
        }
        body
    }

    fn encode_with_checksum(&self) -> String {
        let body = self.body();
        let digest = sha256::hex(&sha256::digest(body.as_bytes()));
        format!("{body}checksum {digest}\n")
    }

    fn parse(text: &str) -> Result<Self> {
        let mut lines: Vec<&str> = text.lines().collect();
        let checksum = lines.pop().context("journal has no checksum")?;
        let expected = checksum
            .strip_prefix("checksum ")
            .context("journal checksum line is malformed")?;
        ensure!(is_digest(expected), "journal checksum is malformed");
        let body = format!("{}\n", lines.join("\n"));
        let actual = sha256::hex(&sha256::digest(body.as_bytes()));
        ensure!(actual == expected, "journal checksum does not match");

        let mut lines = lines.into_iter();
        ensure!(
            core::iter::Iterator::next(&mut lines) == Some(JOURNAL_MAGIC),
            "wrong journal version"
        );
        let generation = <u64 as core::str::FromStr>::from_str(field(&mut lines, "generation")?)
            .context("parsing journal generation")?;
        let commit = field(&mut lines, "commit")?.to_string();
        let registry = field(&mut lines, "registry")?.to_string();
        let source_digest = field(&mut lines, "source")?.to_string();
        let index_digest = field(&mut lines, "index")?.to_string();
        let active = match field(&mut lines, "active")? {
            "none" => None,
            index => Some(
                <usize as core::str::FromStr>::from_str(index)
                    .context("parsing active mutation")?,
            ),
        };
        let count = <usize as core::str::FromStr>::from_str(field(&mut lines, "completed")?)
            .context("parsing completed mutation count")?;
        ensure!(
            is_hex(&commit, 40)
                && is_digest(&registry)
                && is_digest(&source_digest)
                && is_digest(&index_digest),
            "journal identity field is malformed"
        );

        let mut completed = Vec::with_capacity(count);
        for _ in 0..count {
            let line =
                core::iter::Iterator::next(&mut lines).context("journal outcome is missing")?;
            let outcome = match line {
                "result caught" => Outcome::Caught,
                "result survived" => Outcome::Survived,
                _ => {
                    let encoded = line
                        .strip_prefix("result inconclusive ")
                        .context("journal outcome is malformed")?;
                    let bytes = hex_decode(encoded)?;
                    Outcome::Inconclusive(
                        String::from_utf8(bytes).context("journal error is not UTF-8")?,
                    )
                }
            };
            completed.push(outcome);
        }
        ensure!(
            core::iter::Iterator::next(&mut lines).is_none(),
            "journal has trailing records"
        );
        Ok(Self {
            generation,
            commit,
            registry,
            source_digest,
            index_digest,
            active,
            completed,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportVerdict {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Report {
    sequence: u64,
    commit: String,
    registry: String,
    source_digest: String,
    index_digest: String,
    caught: usize,
    survived: usize,
    inconclusive: usize,
    verdict: ReportVerdict,
    outcomes: Vec<Outcome>,
}

impl Report {
    fn from(journal: &Journal) -> Self {
        let (caught, survived, inconclusive) = outcome_totals(&journal.completed);
        Self {
            sequence: 0,
            commit: journal.commit.clone(),
            registry: journal.registry.clone(),
            source_digest: journal.source_digest.clone(),
            index_digest: journal.index_digest.clone(),
            caught,
            survived,
            inconclusive,
            verdict: if survived == 0 && inconclusive == 0 {
                ReportVerdict::Pass
            } else {
                ReportVerdict::Fail
            },
            outcomes: journal.completed.clone(),
        }
    }

    fn body(&self) -> String {
        let verdict = match self.verdict {
            ReportVerdict::Pass => "pass",
            ReportVerdict::Fail => "fail",
        };
        let mut body = format!(
            "{REPORT_MAGIC}\nsequence {}\ncommit {}\nregistry {}\nsource {}\nindex {}\nmutations {}\ncaught {}\nsurvived {}\ninconclusive {}\nverdict {verdict}\n",
            self.sequence,
            self.commit,
            self.registry,
            self.source_digest,
            self.index_digest,
            self.outcomes.len(),
            self.caught,
            self.survived,
            self.inconclusive,
        );
        for outcome in &self.outcomes {
            match outcome {
                Outcome::Caught => body.push_str("result caught\n"),
                Outcome::Survived => body.push_str("result survived\n"),
                Outcome::Inconclusive(error) => {
                    body.push_str("result inconclusive ");
                    body.push_str(&hex_encode(error.as_bytes()));
                    body.push('\n');
                }
            }
        }
        body
    }

    fn encode_with_checksum(&self) -> String {
        let body = self.body();
        let digest = sha256::hex(&sha256::digest(body.as_bytes()));
        format!("{body}checksum {digest}\n")
    }

    fn parse(text: &str) -> Result<Self> {
        ensure!(
            text.ends_with('\n') && !text.contains('\r'),
            "mutation report is not canonical LF text"
        );
        let mut lines: Vec<&str> = text.lines().collect();
        let checksum = lines.pop().context("mutation report has no checksum")?;
        let expected = checksum
            .strip_prefix("checksum ")
            .context("mutation report checksum line is malformed")?;
        ensure!(is_digest(expected), "mutation report checksum is malformed");
        let body = format!("{}\n", lines.join("\n"));
        ensure!(
            sha256::hex(&sha256::digest(body.as_bytes())) == expected,
            "mutation report checksum does not match"
        );

        let mut lines = lines.into_iter();
        ensure!(
            core::iter::Iterator::next(&mut lines) == Some(REPORT_MAGIC),
            "wrong mutation report version"
        );
        let sequence = <u64 as core::str::FromStr>::from_str(field(&mut lines, "sequence")?)
            .context("parsing mutation report sequence")?;
        ensure!(sequence != 0, "mutation report sequence is zero");
        let commit = field(&mut lines, "commit")?.to_string();
        let registry = field(&mut lines, "registry")?.to_string();
        let source_digest = field(&mut lines, "source")?.to_string();
        let index_digest = field(&mut lines, "index")?.to_string();
        let mutation_count =
            <usize as core::str::FromStr>::from_str(field(&mut lines, "mutations")?)
                .context("parsing mutation count")?;
        let caught = <usize as core::str::FromStr>::from_str(field(&mut lines, "caught")?)
            .context("parsing caught count")?;
        let survived = <usize as core::str::FromStr>::from_str(field(&mut lines, "survived")?)
            .context("parsing survived count")?;
        let inconclusive =
            <usize as core::str::FromStr>::from_str(field(&mut lines, "inconclusive")?)
                .context("parsing inconclusive count")?;
        let verdict = match field(&mut lines, "verdict")? {
            "pass" => ReportVerdict::Pass,
            "fail" => ReportVerdict::Fail,
            _ => bail!("mutation report verdict is malformed"),
        };
        ensure!(
            is_hex(&commit, 40)
                && is_digest(&registry)
                && is_digest(&source_digest)
                && is_digest(&index_digest),
            "mutation report identity field is malformed"
        );

        let mut outcomes = Vec::with_capacity(mutation_count);
        for _ in 0..mutation_count {
            let line = core::iter::Iterator::next(&mut lines)
                .context("mutation report outcome is missing")?;
            outcomes.push(match line {
                "result caught" => Outcome::Caught,
                "result survived" => Outcome::Survived,
                _ => {
                    let encoded = line
                        .strip_prefix("result inconclusive ")
                        .context("mutation report outcome is malformed")?;
                    Outcome::Inconclusive(
                        String::from_utf8(hex_decode(encoded)?)
                            .context("mutation report error is not UTF-8")?,
                    )
                }
            });
        }
        ensure!(
            core::iter::Iterator::next(&mut lines).is_none(),
            "mutation report has trailing records"
        );
        let actual = outcome_totals(&outcomes);
        ensure!(
            actual == (caught, survived, inconclusive),
            "mutation report totals do not match its outcomes"
        );
        let expected_verdict = if survived == 0 && inconclusive == 0 {
            ReportVerdict::Pass
        } else {
            ReportVerdict::Fail
        };
        ensure!(
            verdict == expected_verdict,
            "mutation report verdict contradicts its totals"
        );
        let report = Self {
            sequence,
            commit,
            registry,
            source_digest,
            index_digest,
            caught,
            survived,
            inconclusive,
            verdict,
            outcomes,
        };
        ensure!(
            report.encode_with_checksum() == text,
            "mutation report is not in its canonical encoding"
        );
        Ok(report)
    }

    fn require_pass(
        &self,
        baseline: &Baseline,
        registry: &str,
        mutation_count: usize,
    ) -> Result<()> {
        ensure!(
            self.commit == baseline.commit
                && self.registry == registry
                && self.source_digest == baseline.source_digest
                && self.index_digest == baseline.index_digest
                && self.outcomes.len() == mutation_count,
            "mutation report is stale or belongs to different inputs"
        );
        ensure!(
            self.verdict == ReportVerdict::Pass
                && self.caught == mutation_count
                && self.survived == 0
                && self.inconclusive == 0,
            "the newest mutation report did not pass"
        );
        Ok(())
    }
}

fn outcome_totals(outcomes: &[Outcome]) -> (usize, usize, usize) {
    let mut caught = 0;
    let mut survived = 0;
    let mut inconclusive = 0;
    for outcome in outcomes {
        match outcome {
            Outcome::Caught => caught += 1,
            Outcome::Survived => survived += 1,
            Outcome::Inconclusive(_) => inconclusive += 1,
        }
    }
    (caught, survived, inconclusive)
}

fn field<'a>(lines: &mut impl Iterator<Item = &'a str>, name: &str) -> Result<&'a str> {
    let line = core::iter::Iterator::next(lines)
        .with_context(|| format!("journal field {name} is missing"))?;
    line.strip_prefix(&format!("{name} "))
        .with_context(|| format!("journal field {name} is malformed"))
}

struct JournalStore {
    directory: PathBuf,
}

impl JournalStore {
    fn new(directory: PathBuf) -> Result<Self> {
        Ok(Self {
            directory: canonical_existing_real_directory(&directory, "mutation journal directory")?,
        })
    }

    fn require_directory(&self) -> Result<()> {
        ensure!(
            canonical_existing_real_directory(&self.directory, "mutation journal directory")?
                == self.directory,
            "mutation journal directory changed after it was opened"
        );
        Ok(())
    }

    fn slot(&self, generation: u64) -> PathBuf {
        self.directory
            .join(JOURNAL_SLOTS[generation as usize % JOURNAL_SLOTS.len()])
    }

    fn load(&self) -> Result<Option<Journal>> {
        self.require_directory()?;
        let mut present = 0;
        let mut valid = Vec::new();
        let mut errors = Vec::new();
        for name in JOURNAL_SLOTS {
            let path = self.directory.join(name);
            if !require_direct_file_if_present(&self.directory, &path, "mutation journal slot")? {
                continue;
            }
            present += 1;
            match read_state_text(&path, "mutation journal slot")
                .and_then(|text| Journal::parse(&text))
            {
                Ok(journal) => valid.push(journal),
                Err(error) => errors.push(format!("{}: {error:#}", path.display())),
            }
        }
        if let Some(journal) = valid.into_iter().max_by_key(|journal| journal.generation) {
            return Ok(Some(journal));
        }
        if present == 0 {
            Ok(None)
        } else {
            bail!(
                "no complete mutation journal remains:\n{}",
                errors.join("\n")
            )
        }
    }

    fn save(&self, journal: &mut Journal) -> Result<()> {
        self.require_directory()?;
        journal.generation = journal
            .generation
            .checked_add(1)
            .context("mutation journal generation exhausted")?;
        let path = self.slot(journal.generation);
        let encoded = journal.encode_with_checksum();
        self.replace_file(&path, encoded.as_bytes(), "mutation journal slot")
    }

    fn save_report(&self, mut report: Report) -> Result<Report> {
        self.require_directory()?;
        let next = self.latest_report()?.map_or(Ok(1), |(_, report)| {
            report
                .sequence
                .checked_add(1)
                .context("mutation report sequence exhausted")
        })?;
        report.sequence = next;
        let path = self.directory.join(report_name(next));
        let encoded = report.encode_with_checksum();
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::write(&mut options, true);
        let mut file = OpenOptions::open(&options, &path)
            .with_context(|| format!("creating immutable mutation report {}", path.display()))?;
        file.write_all(encoded.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", path.display()))?;
        require_direct_file(&self.directory, &path, "immutable mutation report")?;
        ensure!(
            Report::parse(&read_state_text(&path, "immutable mutation report")?)? == report,
            "durable mutation report differs from the completed sweep"
        );
        Ok(report)
    }

    fn latest_report(&self) -> Result<Option<(PathBuf, Report)>> {
        self.require_directory()?;
        let mut reports = Vec::new();
        for entry in std::fs::read_dir(&self.directory)
            .with_context(|| format!("reading {}", self.directory.display()))?
        {
            let entry = entry
                .with_context(|| format!("reading an entry in {}", self.directory.display()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(sequence) = report_sequence(name)? else {
                continue;
            };
            let path = entry.path();
            require_direct_file(&self.directory, &path, "immutable mutation report")?;
            let report = Report::parse(&read_state_text(&path, "immutable mutation report")?)?;
            ensure!(
                report.sequence == sequence,
                "mutation report sequence disagrees with its file name"
            );
            reports.push((path, report));
        }
        Ok(reports
            .into_iter()
            .max_by_key(|(_, report)| report.sequence))
    }

    fn write_last_run(&self, report: &Report) -> Result<()> {
        self.require_directory()?;
        let path = self.directory.join(LAST_RUN);
        let encoded = report.encode_with_checksum();
        self.replace_file(&path, encoded.as_bytes(), "last mutation report")
    }

    fn clear(&self) -> Result<()> {
        self.require_directory()?;
        for name in JOURNAL_SLOTS {
            let path = self.directory.join(name);
            if require_direct_file_if_present(&self.directory, &path, "mutation journal slot")? {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing completed journal {}", path.display()))?;
            }
        }
        Ok(())
    }

    fn replace_file(&self, path: &Path, bytes: &[u8], label: &str) -> Result<()> {
        ensure!(
            path.parent() == Some(self.directory.as_path()),
            "{label} is not an exact mutation-state child"
        );
        let (temporary, mut file) = self.create_temporary_file(label)?;
        let result = (|| {
            file.write_all(bytes)
                .with_context(|| format!("writing {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("syncing {}", temporary.display()))?;
            drop(file);

            self.require_directory()?;
            if require_direct_file_if_present(&self.directory, path, label)? {
                std::fs::remove_file(path)
                    .with_context(|| format!("removing old {label} {}", path.display()))?;
            }
            std::fs::rename(&temporary, path).with_context(|| {
                format!(
                    "publishing {label} {} as {}",
                    temporary.display(),
                    path.display()
                )
            })?;
            require_direct_file(&self.directory, path, label)
        })();
        finish_staged_operation(&self.directory, &temporary, label, result)
    }

    fn create_temporary_file(&self, label: &str) -> Result<(PathBuf, File)> {
        self.require_directory()?;
        for _ in 0..64 {
            let nonce = NEXT_STORE_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = self.directory.join(format!(
                ".mutation-write-{}-{nonce:016x}",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            OpenOptions::create_new(&mut options, true);
            OpenOptions::write(&mut options, true);
            match OpenOptions::open(&options, &path) {
                Ok(file) => {
                    require_direct_file(&self.directory, &path, label)?;
                    return Ok((path, file));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("creating temporary {label} {}", path.display()));
                }
            }
        }
        bail!("could not allocate a unique temporary {label}")
    }
}

fn report_name(sequence: u64) -> String {
    format!("{REPORT_PREFIX}{sequence:020}{REPORT_SUFFIX}")
}

fn report_sequence(name: &str) -> Result<Option<u64>> {
    let Some(sequence) = name
        .strip_prefix(REPORT_PREFIX)
        .and_then(|name| name.strip_suffix(REPORT_SUFFIX))
    else {
        return Ok(None);
    };
    ensure!(
        sequence.len() == 20 && sequence.bytes().all(|byte| byte.is_ascii_digit()),
        "malformed mutation report file name `{name}`"
    );
    Ok(Some(
        <u64 as core::str::FromStr>::from_str(sequence)
            .context("parsing mutation report file sequence")?,
    ))
}

fn is_digest(text: &str) -> bool {
    is_hex(text, 64)
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut encoded, byte| {
            encoded.push_str(&format!("{byte:02x}"));
            encoded
        },
    )
}

fn hex_decode(encoded: &str) -> Result<Vec<u8>> {
    ensure!(encoded.len().is_multiple_of(2), "hex value has half a byte");
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0]).context("hex value has a non-hexadecimal byte")?;
            let low = hex_nibble(pair[1]).context("hex value has a non-hexadecimal byte")?;
            Ok(high << 4 | low)
        })
        .collect()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    const CHILD_STATE: &str = "WIZARD_MUTATION_CHILD_STATE";
    const CHILD_MARKER: &str = "WIZARD_MUTATION_CHILD_MARKER";

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str) -> Self {
            let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "wizard-mutation-{name}-{}-{unique}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("creating unique mutation test directory");
            Self(path)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("removing mutation test directory");
        }
    }

    fn fixture_journal() -> Journal {
        Journal {
            generation: 0,
            commit: "1".repeat(40),
            registry: "2".repeat(64),
            source_digest: "3".repeat(64),
            index_digest: "4".repeat(64),
            active: None,
            completed: Vec::new(),
        }
    }

    fn fixture_baseline(journal: &Journal) -> Baseline {
        Baseline {
            root: PathBuf::from("fixture-root"),
            commit: journal.commit.clone(),
            tracked: Vec::new(),
            source_digest: journal.source_digest.clone(),
            index_path: PathBuf::from("fixture-index"),
            index_digest: journal.index_digest.clone(),
        }
    }

    #[test]
    fn git_path_order_is_not_component_order() {
        let listed = b"boot/limine.conf\0boot/limine/BOOTX64.EFI\0";
        assert_eq!(
            parse_tracked_paths(listed).unwrap(),
            [
                PathBuf::from("boot/limine.conf"),
                PathBuf::from("boot/limine/BOOTX64.EFI")
            ]
        );
    }

    #[test]
    fn tracked_path_order_refuses_duplicates_and_reversals() {
        assert!(parse_tracked_paths(b"alpha\0alpha\0").is_err());
        assert!(parse_tracked_paths(b"bravo\0alpha\0").is_err());
    }

    #[test]
    fn a_completed_report_binds_totals_and_verdict_to_every_outcome() {
        let mut pass_journal = fixture_journal();
        pass_journal.completed = vec![Outcome::Caught, Outcome::Caught];
        let mut pass = Report::from(&pass_journal);
        pass.sequence = 1;
        assert_eq!(Report::parse(&pass.encode_with_checksum()).unwrap(), pass);
        assert_eq!((pass.caught, pass.survived, pass.inconclusive), (2, 0, 0));
        assert_eq!(pass.verdict, ReportVerdict::Pass);
        pass.require_pass(&fixture_baseline(&pass_journal), &pass_journal.registry, 2)
            .expect("current all-caught report");

        let mut failed_journal = fixture_journal();
        failed_journal.completed = vec![
            Outcome::Caught,
            Outcome::Survived,
            Outcome::Inconclusive("wrong machine verdict".to_string()),
        ];
        let mut failed = Report::from(&failed_journal);
        failed.sequence = 2;
        assert_eq!(
            Report::parse(&failed.encode_with_checksum()).unwrap(),
            failed
        );
        assert_eq!(
            (failed.caught, failed.survived, failed.inconclusive),
            (1, 1, 1)
        );
        assert_eq!(failed.verdict, ReportVerdict::Fail);
        assert!(
            failed
                .require_pass(
                    &fixture_baseline(&failed_journal),
                    &failed_journal.registry,
                    3
                )
                .is_err()
        );

        let mut contradictory = failed.clone();
        contradictory.verdict = ReportVerdict::Pass;
        assert!(Report::parse(&contradictory.encode_with_checksum()).is_err());
        let noncanonical_body = pass.body().replacen("sequence 1\n", "sequence 01\n", 1);
        let noncanonical = format!(
            "{noncanonical_body}checksum {}\n",
            sha256::hex(&sha256::digest(noncanonical_body.as_bytes()))
        );
        assert!(Report::parse(&noncanonical).is_err());
        let mut stale = pass.clone();
        stale.commit = "9".repeat(40);
        assert!(
            stale
                .require_pass(&fixture_baseline(&pass_journal), &pass_journal.registry, 2)
                .is_err()
        );
    }

    #[test]
    fn a_new_failed_sweep_supersedes_but_does_not_erase_an_older_pass() {
        let temp = TempTree::new("report-history");
        let store = JournalStore::new(temp.0.clone()).unwrap();
        let mut journal = fixture_journal();
        journal.completed = vec![Outcome::Caught];
        let pass = store.save_report(Report::from(&journal)).unwrap();
        journal.completed = vec![Outcome::Survived];
        let failed = store.save_report(Report::from(&journal)).unwrap();
        store.write_last_run(&failed).unwrap();

        assert_eq!(pass.sequence, 1);
        assert_eq!(failed.sequence, 2);
        assert!(temp.0.join(report_name(1)).is_file());
        assert!(temp.0.join(report_name(2)).is_file());
        assert_eq!(
            store.latest_report().unwrap().unwrap().1.verdict,
            ReportVerdict::Fail
        );
        let last = std::fs::read_to_string(temp.0.join(LAST_RUN)).unwrap();
        assert_eq!(Report::parse(&last).unwrap(), failed);
    }

    #[test]
    fn a_current_pass_cannot_bind_bytes_from_a_substituted_report() {
        let mut journal = fixture_journal();
        journal.completed = vec![Outcome::Caught];
        let mut selected = Report::from(&journal);
        selected.sequence = 1;
        let selected_bytes = selected.encode_with_checksum();

        let mut substituted = selected.clone();
        substituted.sequence = 2;
        let substituted_bytes = substituted.encode_with_checksum();

        assert_eq!(
            bind_current_report(&selected, selected_bytes.as_bytes()).unwrap(),
            sha256::hex(&sha256::digest(selected_bytes.as_bytes()))
        );
        let error = bind_current_report(&selected, substituted_bytes.as_bytes()).unwrap_err();
        assert!(
            format!("{error:#}").contains("changed while selecting the current pass"),
            "{error:#}"
        );
    }

    #[test]
    fn oversized_mutation_state_is_refused_before_payload_allocation() {
        let temp = TempTree::new("oversized-state");
        let path = temp.0.join("oversized.report");
        File::create(&path)
            .unwrap()
            .set_len(MAX_STATE_FILE_BYTES + 1)
            .unwrap();
        let error = read_state_text(&path, "mutation report")
            .unwrap_err()
            .to_string();
        assert!(error.contains("exceeding the 16777216-byte allocation limit"));
    }

    #[test]
    fn mutation_state_growth_is_refused_after_measurement() {
        let temp = TempTree::new("growing-state");
        let path = temp.0.join("growing.report");
        std::fs::write(&path, b"x").unwrap();
        let growing = path.clone();

        let error = read_bounded_bytes_with(&path, 4, "mutation report", move || {
            let mut options = OpenOptions::new();
            OpenOptions::write(&mut options, true);
            OpenOptions::open(&options, growing)?.set_len(5)?;
            Ok(())
        })
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("grew beyond the 4-byte allocation limit while being read"),
            "{error}"
        );
    }

    #[test]
    fn mutation_digest_refuses_an_initial_file_beyond_its_ceiling() {
        let temp = TempTree::new("oversized-digest");
        let path = temp.0.join("oversized.index");
        File::create(&path).unwrap().set_len(5).unwrap();

        let error = digest_file(&path, 4, "Git index").unwrap_err().to_string();

        assert!(error.contains("exceeding its 4-byte limit"), "{error}");
    }

    #[test]
    fn mutation_digest_refuses_growth_after_measurement() {
        let temp = TempTree::new("growing-digest");
        let path = temp.0.join("growing.index");
        std::fs::write(&path, b"x").unwrap();
        let growing = path.clone();

        let error = digest_file_with(&path, 4, "Git index", move || {
            let mut options = OpenOptions::new();
            OpenOptions::write(&mut options, true);
            OpenOptions::open(&options, growing)?.set_len(5)?;
            Ok(())
        })
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("grew beyond its 4-byte limit while being hashed"),
            "{error}"
        );
    }

    #[test]
    fn mutation_digest_refuses_truncation_after_measurement() {
        let temp = TempTree::new("truncated-digest");
        let path = temp.0.join("truncated.index");
        std::fs::write(&path, b"1234").unwrap();
        let shrinking = path.clone();

        let error = digest_file_with(&path, 4, "Git index", move || {
            let mut options = OpenOptions::new();
            OpenOptions::write(&mut options, true);
            OpenOptions::open(&options, shrinking)?.set_len(3)?;
            Ok(())
        })
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("changed length while being hashed"),
            "{error}"
        );
    }

    #[test]
    fn oversized_tracked_sources_are_refused_before_streaming() {
        let temp = TempTree::new("oversized-tracked-source");
        let path = temp.0.join("oversized");
        File::create(&path)
            .unwrap()
            .set_len(MAX_TRACKED_FILE_BYTES + 1)
            .unwrap();

        let error = digest_tracked(&temp.0, &[PathBuf::from("oversized")])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("exceeding its 16777216-byte limit"),
            "{error}"
        );
    }

    #[test]
    fn tracked_source_trees_have_an_independent_total_size_bound() {
        let temp = TempTree::new("oversized-tracked-tree");
        let mut tracked = Vec::new();
        for index in 0..5 {
            let name = format!("input-{index}");
            File::create(temp.0.join(&name))
                .unwrap()
                .set_len(MAX_TRACKED_FILE_BYTES)
                .unwrap();
            tracked.push(PathBuf::from(name));
        }

        let error = digest_tracked(&temp.0, &tracked).unwrap_err().to_string();
        assert!(
            error.contains("exceeding its 67108864-byte limit"),
            "{error}"
        );
    }

    #[test]
    fn streamed_tracked_source_digest_preserves_the_framed_schema() {
        let temp = TempTree::new("tracked-source-schema");
        std::fs::create_dir(temp.0.join("nested")).unwrap();
        std::fs::write(temp.0.join("alpha"), b"first").unwrap();
        std::fs::write(temp.0.join("nested/beta"), b"second").unwrap();
        let tracked = [PathBuf::from("alpha"), PathBuf::from("nested/beta")];
        let observed = digest_tracked(&temp.0, &tracked).unwrap();

        let mut legacy = Vec::new();
        for relative in &tracked {
            let name = relative.to_str().unwrap();
            let bytes = std::fs::read(temp.0.join(relative)).unwrap();
            legacy.extend_from_slice(&(name.len() as u64).to_le_bytes());
            legacy.extend_from_slice(name.as_bytes());
            legacy.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            legacy.extend_from_slice(&bytes);
        }
        assert_eq!(observed, sha256::hex(&sha256::digest(&legacy)));
    }

    fn git_test(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args(args)
            .output()
            .expect("spawning fixture Git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    const MUTATION_TARGET: &str = "libs/probe/src/lib.rs";
    const NON_TARGET: &str = "libs/probe/src/other.rs";
    const REGRESSION_ARTIFACT: &str = "libs/probe/tests/properties.proptest-regressions";

    fn mutation_session(name: &str) -> (TempTree, Paths) {
        let temp = TempTree::new(name);
        let repository = temp.0.join("repository");
        let target = temp.0.join("target");
        std::fs::create_dir_all(repository.join("libs/probe/src"))
            .expect("repository source directory");
        git_test(&repository, &["init", "--quiet"]);
        std::fs::write(repository.join(MUTATION_TARGET), b"target source").expect("target source");
        std::fs::write(repository.join(NON_TARGET), b"other source").expect("non-target source");
        git_test(&repository, &["add", "."]);
        git_test(
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
        (temp, Paths::new(repository, Some(&target)))
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

    fn remove_directory_symlink(path: &Path) {
        if let Err(file_error) = std::fs::remove_file(path) {
            std::fs::remove_dir(path).unwrap_or_else(|directory_error| {
                panic!(
                    "removing directory redirect {}: {file_error}; {directory_error}",
                    path.display()
                )
            });
        }
    }

    #[test]
    fn initial_copy_refuses_a_nested_redirect_before_creating_outside_paths() {
        let tree = TempTree::new("nested-copy-redirect");
        let source = tree.0.join("source");
        let destination = tree.0.join("destination");
        let outside = tree.0.join("outside");
        std::fs::create_dir_all(source.join("nested/deeper")).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(source.join("nested/deeper/tracked"), b"tracked source").unwrap();
        std::fs::write(outside.join("sentinel"), b"outside authority").unwrap();
        let redirect = destination.join("nested");
        create_directory_symlink(&outside, &redirect);

        assert!(
            copy_tracked(
                &source,
                &destination,
                &[PathBuf::from("nested/deeper/tracked")],
            )
            .is_err()
        );
        assert!(
            !outside.join("deeper").exists(),
            "recursive parent creation escaped the mutation copy root"
        );
        assert_eq!(
            std::fs::read(outside.join("sentinel")).unwrap(),
            b"outside authority"
        );
        remove_directory_symlink(&redirect);
    }

    #[test]
    fn restore_refuses_same_byte_external_state_before_its_fast_path() {
        let tree = TempTree::new("nested-restore-redirect");
        let snapshot = tree.0.join("snapshot");
        let workspace = tree.0.join("workspace");
        let outside = tree.0.join("outside");
        std::fs::create_dir_all(snapshot.join("nested/deeper")).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir_all(outside.join("deeper")).unwrap();
        std::fs::write(snapshot.join("nested/deeper/tracked"), b"same bytes").unwrap();
        std::fs::write(outside.join("deeper/tracked"), b"same bytes").unwrap();
        let redirect = workspace.join("nested");
        create_directory_symlink(&outside, &redirect);

        assert!(restore_file(&snapshot, &workspace, "nested/deeper/tracked").is_err());
        assert_eq!(
            std::fs::read(outside.join("deeper/tracked")).unwrap(),
            b"same bytes"
        );
        remove_directory_symlink(&redirect);
    }

    fn contaminate(workspace: &Path) {
        std::fs::write(workspace.join(MUTATION_TARGET), b"mutated target")
            .expect("mutating target");
        std::fs::write(workspace.join(NON_TARGET), b"mutated non-target")
            .expect("mutating non-target");
        let regression = workspace.join(REGRESSION_ARTIFACT);
        std::fs::create_dir_all(regression.parent().expect("regression parent"))
            .expect("regression directory");
        std::fs::write(regression, b"cc generated regression seed").expect("regression artifact");
    }

    fn assert_pristine(workspace: &Path) {
        assert_eq!(
            std::fs::read(workspace.join(MUTATION_TARGET)).unwrap(),
            b"target source"
        );
        assert_eq!(
            std::fs::read(workspace.join(NON_TARGET)).unwrap(),
            b"other source"
        );
        assert!(!workspace.join(REGRESSION_ARTIFACT).exists());
    }

    #[test]
    fn a_finished_mutation_restores_the_whole_sandbox() {
        let (_temp, paths) = mutation_session("whole-sandbox");
        let mut session = Session::open(&paths, &"a".repeat(64), 1).expect("isolated session");
        let workspace = session.paths().root().to_path_buf();
        session.begin(0).expect("starting mutation");
        contaminate(&workspace);

        session
            .finish(0, MUTATION_TARGET, Outcome::Caught)
            .expect("finishing mutation");
        assert_pristine(&workspace);

        contaminate(&workspace);
        assert_eq!(session.complete(1).unwrap(), [Outcome::Caught]);
        assert_pristine(&workspace);
        let evidence = current_pass(&paths, &"a".repeat(64), 1).expect("current pass evidence");
        assert_eq!(evidence.evidence().mutation_count, 1);
        assert!(evidence.evidence().report_path.is_file());
    }

    #[test]
    fn a_current_pass_blocks_a_newer_sweep_until_consumed() {
        let (_temp, paths) = mutation_session("current-pass-lock");
        let registry = "d".repeat(64);
        Session::open(&paths, &registry, 0)
            .expect("isolated session")
            .complete(0)
            .expect("completed report");
        let pass = current_pass(&paths, &registry, 0).expect("current pass");

        let error = match Session::open(&paths, &registry, 0) {
            Ok(_) => panic!("a newer sweep acquired the current pass's lock"),
            Err(error) => error,
        };
        assert!(
            format!("{error:#}").contains("another mutation sweep holds"),
            "{error:#}"
        );

        drop(pass);
        Session::open(&paths, &registry, 0).expect("lock released after pass consumer");
    }

    #[test]
    fn a_redirected_mutation_state_ancestor_is_refused_before_cleanup() {
        let (temp, paths) = mutation_session("redirected-state");
        let registry = "e".repeat(64);
        let commit = git_text(
            paths.root(),
            ["rev-parse", "--verify", "HEAD"],
            MAX_GIT_IDENTITY_BYTES,
            "fixture commit",
        )
        .expect("fixture commit");
        let key = format!("{}-{}", &commit[..12], &registry[..12]);
        let state_root = paths.target_dir().join(STATE_ROOT_NAME);
        let outside = temp.0.join("outside-state");
        let outside_workspace = outside.join("workspace");
        std::fs::create_dir_all(&state_root).expect("mutation state root");
        std::fs::create_dir_all(&outside_workspace).expect("outside workspace");
        let sentinel = outside_workspace.join("keep");
        std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
        create_directory_symlink(&outside, &state_root.join(key));

        let error = match Session::open(&paths, &registry, 0) {
            Ok(_) => panic!("redirected mutation state was accepted"),
            Err(error) => error,
        };
        assert!(
            format!("{error:#}").contains("mutation state is not a real directory"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&sentinel).expect("outside sentinel survives"),
            b"outside authority"
        );
    }

    #[test]
    fn recursive_cleanup_revalidates_the_whole_mutation_state_ancestry() {
        let temp = TempTree::new("redirected-cleanup");
        let target = temp.0.join("target");
        let state_root = target.join(STATE_ROOT_NAME);
        let state = state_root.join("111111111111-222222222222");
        let outside = temp.0.join("outside-cleanup");
        let source = outside.join("source");
        let workspace = outside.join("workspace");
        std::fs::create_dir_all(&state_root).expect("mutation state root");
        std::fs::create_dir_all(&source).expect("outside source");
        std::fs::create_dir_all(&workspace).expect("outside workspace");
        let sentinel = workspace.join("keep");
        std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
        create_directory_symlink(&outside, &state);

        let error = restore_workspace(&state.join("source"), &state.join("workspace"), &[])
            .expect_err("redirected cleanup was accepted");
        assert!(
            format!("{error:#}").contains("mutation state is not a real directory"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&sentinel).expect("outside sentinel survives"),
            b"outside authority"
        );
    }

    #[test]
    fn tracked_snapshot_and_workspace_redirects_are_refused_at_use() {
        let (temp, paths) = mutation_session("redirected-tracked-paths");
        let registry = "f".repeat(64);
        let session = Session::open(&paths, &registry, 0).expect("isolated session");
        let outside_snapshot = temp.0.join("outside-snapshot");
        std::fs::create_dir(&outside_snapshot).expect("outside snapshot directory");
        let outside_snapshot_file = outside_snapshot.join("lib.rs");
        std::fs::write(&outside_snapshot_file, b"outside snapshot authority")
            .expect("outside snapshot file");
        let snapshot_parent = session.snapshot.join("libs/probe/src");
        std::fs::remove_dir_all(&snapshot_parent).expect("removing copied snapshot parent");
        create_directory_symlink(&outside_snapshot, &snapshot_parent);

        let error = session
            .original(MUTATION_TARGET)
            .expect_err("redirected snapshot read was accepted");
        assert!(
            format!("{error:#}").contains("mutation source file is not a real directory"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&outside_snapshot_file).expect("outside snapshot survives"),
            b"outside snapshot authority"
        );
        drop(session);

        let (_temp, paths) = mutation_session("redirected-workspace-parent");
        let session = Session::open(&paths, &registry, 0).expect("isolated session");
        let outside = paths.root().parent().unwrap().join("outside-workspace");
        std::fs::create_dir(&outside).expect("outside workspace directory");
        let sentinel = outside.join("lib.rs");
        std::fs::write(&sentinel, b"outside workspace authority").expect("outside sentinel");
        let workspace_parent = session.paths().root().join("libs/probe/src");
        std::fs::remove_dir_all(&workspace_parent).expect("removing copied workspace parent");
        create_directory_symlink(&outside, &workspace_parent);

        let error = Session::write(&session, MUTATION_TARGET, "redirected mutation")
            .expect_err("redirected workspace write was accepted");
        assert!(
            format!("{error:#}").contains("mutation workspace file is not a real directory"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&sentinel).expect("outside workspace survives"),
            b"outside workspace authority"
        );

        remove_directory_symlink(&workspace_parent);
        std::fs::create_dir(&workspace_parent).expect("recreating workspace parent");
        let outside_file = outside.join("direct-file");
        std::fs::write(&outside_file, b"outside file authority").expect("outside file");
        create_file_symlink(&outside_file, &workspace_parent.join("lib.rs"));
        let error = Session::write(&session, MUTATION_TARGET, "redirected mutation")
            .expect_err("redirected workspace file was accepted");
        assert!(
            format!("{error:#}").contains("mutation workspace file is not a real file"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read(&outside_file).expect("outside file survives"),
            b"outside file authority"
        );
    }

    #[test]
    fn workspace_replacement_never_writes_through_an_external_hardlink() {
        let temp = TempTree::new("workspace-hardlink");
        let workspace = temp.0.join("workspace");
        let outside = temp.0.join("outside");
        std::fs::create_dir(&workspace).expect("workspace directory");
        std::fs::create_dir(&outside).expect("outside directory");
        let sentinel = outside.join("authority");
        let destination = workspace.join("tracked");
        std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
        std::fs::hard_link(&sentinel, &destination).expect("workspace hard link");

        write_source(&destination, b"isolated mutation").expect("replacing workspace source");

        assert_eq!(
            std::fs::read(&sentinel).expect("outside sentinel survives"),
            b"outside authority"
        );
        assert_eq!(
            std::fs::read(&destination).expect("replacement source"),
            b"isolated mutation"
        );

        let (temp, paths) = mutation_session("session-hardlink");
        let session = Session::open(&paths, &"7".repeat(64), 0).expect("isolated session");
        let destination = session.paths().root().join(MUTATION_TARGET);
        let sentinel = temp.0.join("session-authority");
        std::fs::write(&sentinel, b"target source").expect("session sentinel");
        std::fs::remove_file(&destination).expect("unlinking workspace source");
        std::fs::hard_link(&sentinel, &destination).expect("session workspace hard link");

        Session::write(&session, MUTATION_TARGET, "isolated session mutation")
            .expect("session replacement");

        assert_eq!(
            std::fs::read(&sentinel).expect("session sentinel survives"),
            b"target source"
        );
        assert_eq!(
            std::fs::read(&destination).expect("session replacement source"),
            b"isolated session mutation"
        );
    }

    #[test]
    fn a_failed_source_publication_restores_the_previous_authority() {
        let temp = TempTree::new("source-publication-rollback");
        let destination = temp.0.join("tracked.rs");
        std::fs::write(&destination, b"complete source").expect("complete source");

        let error = write_source_with(&destination, b"partial mutation", |_, _| {
            Err(std::io::Error::other("injected publication refusal"))
        })
        .expect_err("failed publication was accepted");

        assert!(format!("{error:#}").contains("injected publication refusal"));
        assert_eq!(
            std::fs::read(&destination).expect("restored source"),
            b"complete source"
        );
        assert_eq!(crate::paths::directory_entries(&temp.0).unwrap().len(), 1);
    }

    #[test]
    fn failed_stage_cleanup_reports_both_operation_and_removal_failures() {
        let temp = TempTree::new("stage-cleanup-removal");
        let staging = temp.0.join("stage");
        std::fs::write(&staging, b"partial bytes").expect("staging file");

        let error = finish_staged_operation_with(
            &temp.0,
            &staging,
            "fixture stage",
            Err::<(), _>(anyhow::anyhow!("injected operation failure")),
            |_| Err(std::io::Error::other("injected cleanup refusal")),
        )
        .expect_err("cleanup failure was discarded");
        let error = format!("{error:#}");

        assert!(error.contains("injected operation failure"), "{error}");
        assert!(error.contains("injected cleanup refusal"), "{error}");
        assert!(staging.exists(), "the injected remover unexpectedly ran");
    }

    #[test]
    fn failed_stage_cleanup_rejects_a_hostile_stage_replacement() {
        let temp = TempTree::new("stage-cleanup-replacement");
        let staging = temp.0.join("stage");
        std::fs::create_dir(&staging).expect("hostile staging directory");

        let error = finish_staged_operation_with(
            &temp.0,
            &staging,
            "fixture stage",
            Err::<(), _>(anyhow::anyhow!("injected operation failure")),
            |_| panic!("a hostile stage must not reach removal"),
        )
        .expect_err("hostile stage replacement was ignored");
        let error = format!("{error:#}");

        assert!(error.contains("injected operation failure"), "{error}");
        assert!(
            error.contains("fixture stage is not a real file"),
            "{error}"
        );
    }

    #[test]
    fn a_mutation_lock_never_writes_through_an_external_hardlink() {
        let temp = TempTree::new("lock-hardlink");
        let state = temp.0.join("state");
        let outside = temp.0.join("outside");
        std::fs::create_dir(&state).expect("mutation state directory");
        std::fs::create_dir(&outside).expect("outside directory");
        let sentinel = outside.join("authority");
        std::fs::write(&sentinel, b"outside authority").expect("outside sentinel");
        std::fs::hard_link(&sentinel, state.join(LOCK_NAME)).expect("lock hard link");

        let lock = acquire_lock(&state.join(LOCK_NAME)).expect("hard-linked lock is byte-safe");
        drop(lock);

        assert_eq!(
            std::fs::read(&sentinel).expect("outside sentinel survives"),
            b"outside authority"
        );
    }

    #[test]
    fn journal_paths_cannot_be_substituted_after_the_store_opens() {
        let temp = TempTree::new("redirected-journal");
        let state = temp.0.join("state");
        std::fs::create_dir(&state).expect("journal state directory");
        let store = JournalStore::new(state.clone()).expect("journal store");
        let outside = temp.0.join("outside-journal");
        std::fs::create_dir(&outside).expect("outside journal directory");
        let outside_slot = outside.join("journal");
        let journal = fixture_journal();
        std::fs::write(&outside_slot, journal.encode_with_checksum()).expect("outside journal");
        create_file_symlink(&outside_slot, &store.slot(1));

        let error = store
            .load()
            .expect_err("redirected journal read was accepted");
        assert!(
            format!("{error:#}").contains("mutation journal slot is not a real file"),
            "{error:#}"
        );
        assert_eq!(
            std::fs::read_to_string(&outside_slot).expect("outside journal survives"),
            journal.encode_with_checksum()
        );

        std::fs::remove_file(store.slot(1)).expect("removing journal redirect");
        let outside_last_run = outside.join("last-run");
        std::fs::write(&outside_last_run, b"outside report authority").expect("outside report");
        std::fs::hard_link(&outside_last_run, state.join(LAST_RUN)).expect("hard-linking report");
        let mut report = Report::from(&journal);
        report.sequence = 1;
        store
            .write_last_run(&report)
            .expect("publishing last report");
        assert_eq!(
            std::fs::read(&outside_last_run).expect("outside report survives"),
            b"outside report authority"
        );
        assert_eq!(
            Report::parse(&std::fs::read_to_string(state.join(LAST_RUN)).unwrap()).unwrap(),
            report
        );

        let redirected = temp.0.join("redirected-state");
        std::fs::rename(&state, &redirected).expect("moving journal state");
        create_directory_symlink(&redirected, &state);
        let error = store
            .save(&mut fixture_journal())
            .expect_err("redirected journal directory was accepted");
        assert!(
            format!("{error:#}").contains("mutation journal directory is not a real directory"),
            "{error:#}"
        );
        assert!(!redirected.join("journal.1").exists());
    }

    #[test]
    fn an_interrupted_session_is_sanitized_before_recovery() {
        let (_temp, paths) = mutation_session("recovery-sandbox");
        let mut session = Session::open(&paths, &"b".repeat(64), 1).expect("isolated session");
        let workspace = session.paths().root().to_path_buf();
        session.begin(0).expect("starting mutation");
        drop(session);
        contaminate(&workspace);

        let mut resumed = Session::open(&paths, &"b".repeat(64), 1).expect("resuming session");
        assert_eq!(resumed.active(), Some(0));
        assert_pristine(&workspace);

        contaminate(&workspace);
        resumed
            .recover(0, MUTATION_TARGET)
            .expect("recovering mutation");
        assert_eq!(resumed.active(), None);
        assert_pristine(&workspace);

        resumed.begin(0).expect("restarting mutation");
        Session::write(&resumed, MUTATION_TARGET, "mutated target").expect("applying mutation");
        resumed
            .finish(0, MUTATION_TARGET, Outcome::Caught)
            .expect("finishing mutation");
        assert_eq!(resumed.complete(1).unwrap(), [Outcome::Caught]);
    }

    /// The real Git boundary supplies only tracked bytes to a distinct build
    /// root and leaves both the checkout and its index byte-exact.
    #[test]
    fn a_session_builds_only_from_its_tracked_copy() {
        let temp = TempTree::new("session");
        let repository = temp.0.join("repository");
        let target = temp.0.join("target");
        std::fs::create_dir(&repository).expect("repository directory");
        git_test(&repository, &["init", "--quiet"]);
        std::fs::write(repository.join(".gitignore"), "ignored\n").expect("ignore file");
        std::fs::write(repository.join("tracked"), b"source bytes").expect("tracked file");
        std::fs::write(repository.join("ignored"), b"not an input").expect("ignored file");
        git_test(&repository, &["add", ".gitignore", "tracked"]);
        git_test(
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
        let source_before = std::fs::read(repository.join("tracked")).expect("source before");
        let index_before = std::fs::read(repository.join(".git/index")).expect("index before");

        let paths = Paths::new(repository.clone(), Some(&target));
        let session = Session::open(&paths, &"a".repeat(64), 0).expect("isolated session");
        let workspace = session.paths().root().to_path_buf();
        assert_ne!(workspace, repository);
        assert_eq!(
            std::fs::read(workspace.join("tracked")).unwrap(),
            source_before
        );
        assert!(!workspace.join("ignored").exists());
        assert!(!workspace.join(".git").exists());
        assert_eq!(session.complete(0).unwrap(), []);

        assert_eq!(
            std::fs::read(repository.join("tracked")).unwrap(),
            source_before
        );
        assert_eq!(
            std::fs::read(repository.join(".git/index")).unwrap(),
            index_before
        );
        let status = Command::new("git")
            .current_dir(&repository)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args(["status", "--porcelain=v1", "--untracked-files=all"])
            .output()
            .expect("final Git status");
        assert!(status.status.success());
        assert!(status.stdout.is_empty());
    }

    #[test]
    fn a_changed_snapshot_cannot_authorize_a_report_for_pristine_source() {
        let (_temp, paths) = mutation_session("changed-snapshot");
        let session = Session::open(&paths, &"c".repeat(64), 0).expect("isolated session");
        std::fs::write(
            session.snapshot.join(MUTATION_TARGET),
            b"substituted source",
        )
        .expect("changing copied snapshot");

        let error = session.complete(0).unwrap_err().to_string();
        assert!(error.contains("source snapshot changed"), "{error}");
        assert!(current_pass(&paths, &"c".repeat(64), 0).is_err());
    }

    fn chain_source(value: u8) -> String {
        format!("#[inline(always)]\npub fn value() -> u8 {{ {value} }}\n")
    }

    /// A three-crate inlining chain whose old snapshot timestamp must not hide
    /// a restored source change from Cargo.
    fn chain(name: &str) -> (TempTree, PathBuf, Paths) {
        let temp = TempTree::new(name);
        let snapshot = temp.0.join("snapshot");
        let workspace = temp.0.join("workspace");
        let build = temp.0.join("build");
        std::fs::create_dir_all(snapshot.join("libs/probe/src")).expect("snapshot source");
        for directory in ["libs/probe/src", "middle/src", "app/src"] {
            std::fs::create_dir_all(workspace.join(directory)).expect("workspace source");
        }
        for (path, contents) in [
            (
                "Cargo.toml",
                "[workspace]\nresolver = \"3\"\nmembers = [\"libs/probe\", \"middle\", \"app\"]\n",
            ),
            (
                "libs/probe/Cargo.toml",
                "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
            ),
            (
                "middle/Cargo.toml",
                "[package]\nname = \"middle\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 [dependencies]\nprobe = { path = \"../libs/probe\" }\n",
            ),
            (
                "app/Cargo.toml",
                "[package]\nname = \"app\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
                 [dependencies]\nmiddle = { path = \"../middle\" }\n",
            ),
            (
                "middle/src/lib.rs",
                "#[inline(always)]\npub fn seen() -> u8 { probe::value() }\n",
            ),
            (
                "app/src/main.rs",
                "fn main() { print!(\"{}\", middle::seen()); }\n",
            ),
        ] {
            std::fs::write(workspace.join(path), contents).expect("fixture file");
        }
        std::fs::write(snapshot.join(CHAIN_INPUT), chain_source(1)).expect("snapshot input");
        std::fs::write(workspace.join(CHAIN_INPUT), chain_source(1)).expect("pristine input");
        (temp, snapshot, Paths::new(workspace, Some(&build)))
    }

    const CHAIN_INPUT: &str = "libs/probe/src/lib.rs";

    fn observed(paths: &Paths) -> String {
        let output = Command::new(env!("CARGO"))
            .current_dir(paths.root())
            .env("CARGO_TARGET_DIR", paths.target_dir())
            .args(["run", "--quiet", "--package", "app"])
            .output()
            .expect("running the fixture application");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("fixture output")
    }

    /// A restored mutation must be what the next build *observes*, not merely
    /// what the file on disk says.
    #[test]
    fn a_restored_input_is_what_the_next_build_observes() {
        let (_temp, snapshot, paths) = chain("restore");
        assert_eq!(observed(&paths), "1");
        std::fs::write(paths.root().join(CHAIN_INPUT), chain_source(2)).expect("mutated input");
        assert_eq!(observed(&paths), "2");

        restore_file(&snapshot, paths.root(), CHAIN_INPUT).expect("restoring input");
        assert_eq!(
            std::fs::read_to_string(paths.root().join(CHAIN_INPUT)).unwrap(),
            chain_source(1)
        );
        assert_eq!(observed(&paths), "1");
    }

    /// A new sweep must rebuild rather than inherit the preceding sweep's last
    /// mutation from its retained build directory.
    #[test]
    fn a_recopied_workspace_is_what_the_next_build_observes() {
        let (_temp, snapshot, paths) = chain("recopy");
        assert_eq!(observed(&paths), "1");
        std::fs::write(paths.root().join(CHAIN_INPUT), chain_source(2)).expect("mutated input");
        assert_eq!(observed(&paths), "2");

        copy_tracked(&snapshot, paths.root(), &[PathBuf::from(CHAIN_INPUT)])
            .expect("recopying tracked source");
        assert_eq!(observed(&paths), "1");
    }

    /// The helper process a parent test really terminates while a mutation is
    /// active. It is ignored unless that parent selects it explicitly.
    #[test]
    #[ignore]
    fn interruption_child() {
        let state = PathBuf::from(std::env::var_os(CHILD_STATE).expect("child state path"));
        let marker = PathBuf::from(std::env::var_os(CHILD_MARKER).expect("child marker path"));
        let _lock = acquire_lock(&state.join(LOCK_NAME)).expect("child holds sweep lock");
        std::fs::write(state.join("workspace/tracked"), b"mutated")
            .expect("child applies isolated mutation");
        std::fs::write(marker, b"entered").expect("child announces active work");
        loop {
            std::thread::park();
        }
    }

    fn kill_at(state: &Path, marker: &Path) {
        let mut child = Command::new(std::env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "mutation_workspace::tests::interruption_child",
                "--ignored",
            ])
            .env(CHILD_STATE, state)
            .env(CHILD_MARKER, marker)
            .spawn()
            .expect("spawning interruptible mutation child");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "mutation child never entered its activity");
        let second_acquired = acquire_lock(&state.join(LOCK_NAME)).is_ok();
        child.kill().expect("killing mutation child");
        child.wait().expect("reaping mutation child");
        assert!(!second_acquired, "a live second sweep acquired the lock");
    }

    /// A killed build or boot leaves only the copy dirty; the released lock
    /// and active journal let the next process restore and rerun it.
    #[test]
    fn a_killed_build_or_boot_resumes_outside_the_user_tree() {
        let temp = TempTree::new("interrupt");
        let state = temp.0.join("state");
        let source = state.join("source");
        let workspace = state.join("workspace");
        let user = temp.0.join("user");
        std::fs::create_dir_all(&source).expect("source directory");
        std::fs::create_dir_all(&workspace).expect("workspace directory");
        std::fs::create_dir_all(&user).expect("user directory");
        std::fs::write(source.join("tracked"), b"original").expect("snapshot file");
        std::fs::write(workspace.join("tracked"), b"original").expect("workspace file");
        std::fs::write(user.join("tracked"), b"user source").expect("user source");
        std::fs::write(user.join("index"), b"user index").expect("user index");
        let before_source = std::fs::read(user.join("tracked")).expect("source snapshot");
        let before_index = std::fs::read(user.join("index")).expect("index snapshot");
        let store = JournalStore::new(state.clone()).unwrap();
        let mut journal = fixture_journal();
        store.save(&mut journal).expect("initial journal");

        for activity in ["build", "boot"] {
            journal.active = Some(0);
            store.save(&mut journal).expect("active journal");
            let marker = state.join(format!("{activity}-entered"));
            kill_at(&state, &marker);

            let _lock = acquire_lock(&state.join(LOCK_NAME)).expect("released lock is reusable");
            let loaded = store
                .load()
                .expect("loading interrupted journal")
                .expect("journal remains");
            assert_eq!(loaded.active, Some(0));
            restore_file(&source, &workspace, "tracked").expect("resuming restores sandbox");
            assert_eq!(
                std::fs::read(workspace.join("tracked")).unwrap(),
                b"original"
            );
            assert_eq!(std::fs::read(user.join("tracked")).unwrap(), before_source);
            assert_eq!(std::fs::read(user.join("index")).unwrap(), before_index);
            journal = loaded;
            journal.active = None;
            store.save(&mut journal).expect("recovered journal");
        }
    }

    /// The newest complete generation wins; a torn rewrite cannot erase the
    /// previous complete one.
    #[test]
    fn journal_uses_the_newest_complete_slot_and_falls_back_from_a_torn_write() {
        let temp = TempTree::new("journal");
        let store = JournalStore::new(temp.0.clone()).unwrap();
        let mut journal = fixture_journal();
        store.save(&mut journal).expect("first generation");
        let older = journal.clone();
        journal.active = Some(0);
        store.save(&mut journal).expect("second generation");

        assert_eq!(store.load().unwrap(), Some(journal.clone()));
        std::fs::write(store.slot(journal.generation), b"torn").expect("tearing newest slot");
        assert_eq!(store.load().unwrap(), Some(older));
    }
}
