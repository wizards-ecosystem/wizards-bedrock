//! Crash-safe isolation and progress for mutation sweeps.
//!
//! Restoring source also invalidates its compiled package. Cargo's timestamp
//! cache can otherwise keep the previous mutation's object after the snapshot
//! copy, contaminating the next mutation even though the bytes are pristine
//! (D-233).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;
use crate::sha256;

const JOURNAL_MAGIC: &str = "wizard-mutation-journal-v1";
const LOCK_NAME: &str = "LOCK";
const JOURNAL_SLOTS: [&str; 2] = ["journal.0", "journal.1"];

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
        let state_root = paths.target_dir().join("mutate");
        std::fs::create_dir_all(&state_root)
            .with_context(|| format!("creating {}", state_root.display()))?;
        let lock = acquire_lock(&state_root.join(LOCK_NAME))?;
        let baseline = Baseline::discover(paths.root())?;
        let key = format!("{}-{}", &baseline.commit[..12], &registry[..12]);
        let state = state_root.join(key);
        let snapshot = state.join("source");
        let workspace_root = state.join("workspace");
        let build = state.join("build");
        std::fs::create_dir_all(&state).with_context(|| format!("creating {}", state.display()))?;
        let store = JournalStore::new(state);

        let journal = match store.load()? {
            Some(journal) => {
                journal.matches(&baseline, registry, mutation_count)?;
                ensure!(
                    snapshot.is_dir(),
                    "the resumable source snapshot is missing"
                );
                ensure!(
                    workspace_root.is_dir(),
                    "the resumable workspace is missing"
                );
                ensure!(
                    digest_tracked(&snapshot, &baseline.tracked)? == baseline.source_digest,
                    "the resumable source snapshot changed"
                );
                journal
            }
            None => {
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

        Ok(Self {
            _lock: lock,
            baseline,
            snapshot,
            workspace: Paths::new(workspace_root, Some(&build)),
            store,
            journal,
        })
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
        ensure!(
            self.journal.active == Some(index),
            "another mutation is active"
        );
        ensure!(
            index == self.journal.completed.len(),
            "active progress is not contiguous"
        );
        restore_build_input(&self.snapshot, &self.workspace, relative)?;
        self.journal.active = None;
        self.store.save(&mut self.journal)
    }

    /// Read one pristine tracked file from the commit-pinned snapshot.
    pub fn original(&self, relative: &str) -> Result<String> {
        let path = checked_join(&self.snapshot, relative)?;
        std::fs::read_to_string(&path)
            .with_context(|| format!("reading snapshot {}", path.display()))
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
        let path = checked_join(self.workspace.root(), relative)?;
        std::fs::write(&path, mutated)
            .with_context(|| format!("writing isolated mutation {}", path.display()))
    }

    /// Restore the active file exactly, then make its outcome durable.
    pub fn finish(&mut self, index: usize, relative: &str, outcome: Outcome) -> Result<()> {
        ensure!(
            self.journal.active == Some(index),
            "another mutation is active"
        );
        restore_build_input(&self.snapshot, &self.workspace, relative)?;
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
        self.baseline.verify()?;

        let report = self.journal.encode_with_checksum();
        let path = self.store.directory.join("last-run");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        file.write_all(report.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", path.display()))?;
        self.store.clear()?;
        Ok(std::mem::take(&mut self.journal.completed))
    }
}

struct SweepLock {
    _file: File,
}

fn acquire_lock(path: &Path) -> Result<SweepLock> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening mutation lock {}", path.display()))?;
    file.try_lock().with_context(|| {
        format!(
            "another mutation sweep holds {}; wait for it or stop it before resuming",
            path.display()
        )
    })?;
    file.set_len(0)
        .with_context(|| format!("truncating mutation lock {}", path.display()))?;
    writeln!(file, "pid {}", std::process::id())
        .with_context(|| format!("writing mutation lock {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing mutation lock {}", path.display()))?;
    Ok(SweepLock { _file: file })
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

impl Baseline {
    fn discover(root: &Path) -> Result<Self> {
        let status = git(root, ["status", "--porcelain=v1", "--untracked-files=all"])?;
        ensure!(
            status.is_empty(),
            "mutation sweeps require a clean checkout; git reports:\n{}",
            String::from_utf8_lossy(&status)
        );
        let commit = git_text(root, ["rev-parse", "--verify", "HEAD"])?;
        ensure!(is_hex(&commit, 40), "HEAD is not a full Git object id");
        let listed = git(root, ["ls-files", "-z"])?;
        let mut tracked = Vec::new();
        for raw in listed
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let text = std::str::from_utf8(raw).context("a tracked path is not UTF-8")?;
            validate_relative(text)?;
            tracked.push(PathBuf::from(text));
        }
        ensure!(!tracked.is_empty(), "Git reports no tracked files");

        let index_name = git_text(root, ["rev-parse", "--git-path", "index"])?;
        let index_path = if Path::new(&index_name).is_absolute() {
            PathBuf::from(index_name)
        } else {
            root.join(index_name)
        };
        let index_digest = digest_file(&index_path)?;
        let source_digest = digest_tracked(root, &tracked)?;
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
            digest_file(&self.index_path)? == self.index_digest,
            "the Git index changed during the mutation sweep"
        );
        Ok(())
    }
}

fn git<const N: usize>(root: &Path, args: [&str; N]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .context("spawning read-only Git query")?;
    if !output.status.success() {
        bail!(
            "Git query failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn git_text<const N: usize>(root: &Path, args: [&str; N]) -> Result<String> {
    let bytes = git(root, args)?;
    Ok(std::str::from_utf8(&bytes)
        .context("Git returned non-UTF-8 text")?
        .trim()
        .to_string())
}

fn copy_tracked(from: &Path, to: &Path, tracked: &[PathBuf]) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    for relative in tracked {
        let source = from.join(relative);
        let destination = to.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::copy(&source, &destination).with_context(|| {
            format!("copying {} to {}", source.display(), destination.display())
        })?;
    }
    Ok(())
}

fn restore_file(snapshot: &Path, workspace: &Path, relative: &str) -> Result<()> {
    let source = checked_join(snapshot, relative)?;
    let destination = checked_join(workspace, relative)?;
    std::fs::copy(&source, &destination).with_context(|| {
        format!(
            "restoring isolated mutation {} from {}",
            destination.display(),
            source.display()
        )
    })?;
    ensure!(
        std::fs::read(&source).with_context(|| format!("reading {}", source.display()))?
            == std::fs::read(&destination)
                .with_context(|| format!("reading {}", destination.display()))?,
        "restored mutation file differs from its snapshot"
    );
    Ok(())
}

fn restore_build_input(snapshot: &Path, paths: &Paths, relative: &str) -> Result<()> {
    restore_file(snapshot, paths.root(), relative)?;
    invalidate_package(paths, relative)
}

fn invalidate_package(paths: &Paths, relative: &str) -> Result<()> {
    let Some(package) = source_package(relative) else {
        return Ok(());
    };
    let output = Command::new(env!("CARGO"))
        .current_dir(paths.root())
        .env("CARGO_TARGET_DIR", paths.target_dir())
        .args(["clean", "--package", package])
        .output()
        .with_context(|| format!("spawning Cargo to invalidate restored package {package}"))?;
    if !output.status.success() {
        bail!(
            "invalidating restored package {package} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn source_package(relative: &str) -> Option<&str> {
    if relative.starts_with("kernel/") {
        Some(crate::paths::KERNEL_PACKAGE)
    } else if relative.starts_with("wost/") {
        Some("wost")
    } else if relative.starts_with("xtask/") {
        Some("xtask")
    } else {
        relative
            .strip_prefix("libs/")?
            .split_once('/')
            .map(|(package, _)| package)
    }
}

fn checked_join(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative(relative)?;
    Ok(root.join(relative))
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
    let mut material = Vec::new();
    for relative in tracked {
        let name = relative.to_string_lossy();
        let bytes = std::fs::read(root.join(relative))
            .with_context(|| format!("reading tracked file {}", relative.display()))?;
        material.extend_from_slice(&(name.len() as u64).to_le_bytes());
        material.extend_from_slice(name.as_bytes());
        material.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        material.extend_from_slice(&bytes);
    }
    Ok(sha256::hex(&sha256::digest(&material)))
}

fn digest_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(sha256::hex(&sha256::digest(&bytes)))
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
        ensure!(lines.next() == Some(JOURNAL_MAGIC), "wrong journal version");
        let generation = field(&mut lines, "generation")?
            .parse()
            .context("parsing journal generation")?;
        let commit = field(&mut lines, "commit")?.to_string();
        let registry = field(&mut lines, "registry")?.to_string();
        let source_digest = field(&mut lines, "source")?.to_string();
        let index_digest = field(&mut lines, "index")?.to_string();
        let active = match field(&mut lines, "active")? {
            "none" => None,
            index => Some(index.parse().context("parsing active mutation")?),
        };
        let count: usize = field(&mut lines, "completed")?
            .parse()
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
            let line = lines.next().context("journal outcome is missing")?;
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
        ensure!(lines.next().is_none(), "journal has trailing records");
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

fn field<'a>(lines: &mut impl Iterator<Item = &'a str>, name: &str) -> Result<&'a str> {
    let line = lines
        .next()
        .with_context(|| format!("journal field {name} is missing"))?;
    line.strip_prefix(&format!("{name} "))
        .with_context(|| format!("journal field {name} is malformed"))
}

struct JournalStore {
    directory: PathBuf,
}

impl JournalStore {
    fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn slot(&self, generation: u64) -> PathBuf {
        self.directory
            .join(JOURNAL_SLOTS[generation as usize % JOURNAL_SLOTS.len()])
    }

    fn load(&self) -> Result<Option<Journal>> {
        let mut present = 0;
        let mut valid = Vec::new();
        let mut errors = Vec::new();
        for name in JOURNAL_SLOTS {
            let path = self.directory.join(name);
            if !path.exists() {
                continue;
            }
            present += 1;
            match std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))
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
        journal.generation = journal
            .generation
            .checked_add(1)
            .context("mutation journal generation exhausted")?;
        let path = self.slot(journal.generation);
        let encoded = journal.encode_with_checksum();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        file.write_all(encoded.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", path.display()))
    }

    fn clear(&self) -> Result<()> {
        for name in JOURNAL_SLOTS {
            let path = self.directory.join(name);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing completed journal {}", path.display()))?;
            }
        }
        Ok(())
    }
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
    fn restoring_a_compiled_input_invalidates_its_package() {
        let temp = TempTree::new("invalidation");
        let snapshot = temp.0.join("snapshot");
        let workspace = temp.0.join("workspace");
        let build = temp.0.join("build");
        std::fs::create_dir_all(snapshot.join("wost/src")).expect("snapshot source");
        std::fs::create_dir_all(workspace.join("wost/src")).expect("workspace source");
        std::fs::write(
            workspace.join("Cargo.toml"),
            "[workspace]\nmembers = [\"wost\"]\nresolver = \"3\"\n",
        )
        .expect("workspace manifest");
        std::fs::write(
            workspace.join("wost/Cargo.toml"),
            "[package]\nname = \"wost\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
        )
        .expect("package manifest");
        std::fs::write(
            snapshot.join("wost/src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .expect("snapshot input");
        std::fs::write(
            workspace.join("wost/src/lib.rs"),
            "pub fn value() -> u8 { 2 }\n",
        )
        .expect("mutated input");

        let paths = Paths::new(workspace, Some(&build));
        let output = Command::new(env!("CARGO"))
            .current_dir(paths.root())
            .env("CARGO_TARGET_DIR", paths.target_dir())
            .args(["check", "--package", "wost"])
            .output()
            .expect("building mutated package");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let dependencies = build.join("debug/deps");
        let has_wost_artifact = || {
            std::fs::read_dir(&dependencies)
                .expect("package artifacts")
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().contains("wost"))
        };
        assert!(has_wost_artifact());

        restore_build_input(&snapshot, &paths, "wost/src/lib.rs").expect("restoring input");
        assert_eq!(
            std::fs::read_to_string(paths.root().join("wost/src/lib.rs")).unwrap(),
            "pub fn value() -> u8 { 1 }\n"
        );
        assert!(!has_wost_artifact());
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
        let store = JournalStore::new(state.clone());
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
        let store = JournalStore::new(temp.0.clone());
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
