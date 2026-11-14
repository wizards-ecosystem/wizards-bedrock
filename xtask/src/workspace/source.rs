use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::files::is_reparse_point;
use crate::paths::Paths;
use crate::sha256;

use super::tree::{
    canonical_existing_real_directory, open_real_file, prepare_real_child, require_direct_file,
    require_direct_file_if_present, require_real_child, require_real_directory_metadata,
};

// Git may contend with the compiler-heavy host suite on release builders; the bound is
// generous so that scheduler latency alone should not fail the gate.
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_GIT_IDENTITY_BYTES: usize = 4096;
const MAX_GIT_STATUS_BYTES: usize = 1024 * 1024;
const MAX_GIT_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRACKED_PATH_BYTES: usize = 4 * 1024 * 1024;
const MAX_TRACKED_FILES: usize = 4096;
const MAX_TRACKED_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRACKED_TREE_BYTES: u64 = 64 * 1024 * 1024;
static NEXT_SOURCE_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static NEXT_SOURCE_COPY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Run one operation against an exact clean tracked copy outside the checkout.
pub(crate) fn with_clean_source_copy<T>(
    paths: &Paths,
    operation: impl FnOnce(&Path) -> Result<T>,
) -> Result<T> {
    let baseline = Baseline::discover(paths.root())?;
    let parent = canonical_existing_real_directory(
        &std::env::temp_dir(),
        "clean source-copy temporary parent",
    )?;
    let root = parent.join(format!(
        "wizard-clean-source-{}-{}",
        std::process::id(),
        NEXT_SOURCE_COPY.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&root)
        .with_context(|| format!("creating clean source copy {}", root.display()))?;
    let result = (|| {
        copy_tracked(paths.root(), &root, &baseline.tracked)?;
        ensure!(
            digest_tracked(&root, &baseline.tracked)? == baseline.source_digest,
            "the clean source copy differs from the checkout"
        );
        operation(&root)
    })();
    let cleanup = std::fs::symlink_metadata(&root)
        .with_context(|| format!("inspecting clean source copy {}", root.display()))
        .and_then(|metadata| remove_workspace_entry(&root, &metadata));
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup.context("removing clean source copy")),
        (Err(error), Err(cleanup)) => Err(anyhow::anyhow!(
            "{error:#}; source-copy cleanup also failed: {cleanup:#}"
        )),
    }
}

struct Baseline {
    tracked: Vec<PathBuf>,
    source_digest: String,
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
        "Git reports {} tracked files, exceeding the {MAX_TRACKED_FILES}-file limit",
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
            "source status",
        )?;
        ensure!(
            status.is_empty(),
            "a clean source copy requires a clean checkout; git reports:\n{}",
            String::from_utf8_lossy(&status)
        );
        let commit = git_text(
            root,
            ["rev-parse", "--verify", "HEAD"],
            MAX_GIT_IDENTITY_BYTES,
            "source commit",
        )?;
        ensure!(
            crate::git::is_git_id(&commit),
            "HEAD is not a full Git object id"
        );
        let listed = git(
            root,
            ["ls-files", "-z"],
            MAX_TRACKED_PATH_BYTES,
            "tracked-path inventory",
        )?;
        let tracked = parse_tracked_paths(&listed)?;

        let index_name = git_text(
            root,
            ["rev-parse", "--path-format=absolute", "--git-path", "index"],
            MAX_GIT_IDENTITY_BYTES,
            "Git index path",
        )?;
        let index_path = require_confined_absolute_file(root, &index_name, "Git index")?;
        let index_digest = digest_file(&index_path, MAX_GIT_INDEX_BYTES, "Git index")?;
        let source_digest = digest_tracked(root, &tracked)?;
        let settled_status = git(
            root,
            ["status", "--porcelain=v1", "--untracked-files=all"],
            MAX_GIT_STATUS_BYTES,
            "settled source status",
        )?;
        ensure!(
            settled_status.is_empty(),
            "source changed while its baseline was captured; git reports:\n{}",
            String::from_utf8_lossy(&settled_status)
        );
        ensure!(
            git_text(
                root,
                ["rev-parse", "--verify", "HEAD"],
                MAX_GIT_IDENTITY_BYTES,
                "settled source commit",
            )? == commit,
            "source commit changed while its baseline was captured"
        );
        ensure!(
            digest_file(&index_path, MAX_GIT_INDEX_BYTES, "settled Git index")? == index_digest,
            "Git index changed while the baseline was captured"
        );
        Ok(Self {
            tracked,
            source_digest,
        })
    }
}

fn git<const N: usize>(
    root: &Path,
    args: [&str; N],
    maximum: usize,
    label: &str,
) -> Result<Vec<u8>> {
    let mut command = crate::git::resolved()?.command(root);
    command.args(args);
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
    let root = canonical_existing_real_directory(to, "source copy root")?;
    for relative in tracked {
        let relative = relative
            .to_str()
            .context("a tracked source path is not UTF-8")?;
        let source = require_confined_file(from, relative, "source copy source")?;
        let (_, destination) =
            prepare_confined_destination(&root, relative, "source copy destination")?;
        let bytes = crate::files::read_bounded_file(
            &source,
            MAX_TRACKED_FILE_BYTES,
            "tracked source file",
        )?;
        write_source(&destination, &bytes)?;
    }
    Ok(())
}

/// Staged beside the destination and renamed into place, so a reader never sees half a file.
fn write_source(destination: &Path, bytes: &[u8]) -> Result<()> {
    let parent = destination
        .parent()
        .context("source destination has no parent")?;
    ensure!(
        destination.parent() == Some(parent),
        "source destination escaped its parent"
    );

    let mut reserved = None;
    for _ in 0..64 {
        let nonce = NEXT_SOURCE_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = parent.join(format!(".source-{}-{nonce:016x}", std::process::id()));
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::write(&mut options, true);
        match OpenOptions::open(&options, &path) {
            Ok(file) => {
                require_direct_file(parent, &path, "temporary source")?;
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
    let (temporary, mut file) = reserved.context("could not reserve a temporary source")?;
    let result = (|| {
        file.write_all(bytes)
            .with_context(|| format!("writing {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", temporary.display()))?;
        drop(file);

        publish_staged_file(parent, &temporary, destination, "source", |published| {
            ensure!(
                crate::files::read_bounded_file(
                    published,
                    MAX_TRACKED_FILE_BYTES,
                    "published source",
                )? == bytes,
                "published source differs from its requested bytes"
            );
            Ok(())
        })?;
        Ok(())
    })();
    finish_staged_operation(parent, &temporary, "temporary source", result)
}

pub(crate) fn finish_staged_operation<T>(
    parent: &Path,
    staging: &Path,
    label: &str,
    result: Result<T>,
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
                std::fs::remove_file(staging).with_context(|| {
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
        std::fs::rename(staging, destination).with_context(|| {
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
            ".source-{kind}-{}-{nonce:016x}",
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
    bail!("could not reserve a source {kind} path")
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

fn prepare_confined_destination(
    root: &Path,
    relative: &str,
    label: &str,
) -> Result<(PathBuf, PathBuf)> {
    validate_relative(relative)?;
    let canonical_root = canonical_existing_real_directory(root, label)?;
    let mut parent = canonical_root;
    let mut components = Path::new(relative).components().peekable();
    while let Some(component) = components.next() {
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

fn require_confined_file(root: &Path, relative: &str, label: &str) -> Result<PathBuf> {
    validate_relative(relative)?;
    let root_metadata = std::fs::symlink_metadata(root)
        .with_context(|| format!("inspecting {}", root.display()))?;
    require_real_directory_metadata(root, &root_metadata, label)?;

    let path = Path::new(relative);
    let mut parent = root.to_path_buf();
    let mut components = path.components().peekable();
    while let Some(component) = components.next() {
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

fn require_confined_absolute_file(root: &Path, absolute: &str, label: &str) -> Result<PathBuf> {
    let root = canonical_existing_real_directory(root, label)?;
    let path = Path::new(absolute);
    ensure!(
        path.is_absolute(),
        "{label} is not an absolute path: {}",
        path.display()
    );
    let name = path
        .file_name()
        .context("absolute confined file has no final component")?;
    let parent = path
        .parent()
        .context("absolute confined file has no parent")?;
    let parent = canonical_existing_real_directory(parent, label)?;
    let relative_parent = parent
        .strip_prefix(&root)
        .with_context(|| format!("{label} is outside {}", root.display()))?;
    let relative = relative_parent.join(name);
    let relative = relative
        .to_str()
        .context("absolute confined file path is not UTF-8")?;
    require_confined_file(&root, relative, label)
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
            .context("a tracked source path is not UTF-8")?;
        let path = require_confined_file(root, name, "tracked source")?;
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
        require_confined_file(root, name, "tracked source")?;
        let mut file = open_real_file(&path, "tracked source")?;
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
        require_confined_file(root, name, "tracked source")?;
    }
    Ok(sha256::hex(&hasher.finish()))
}

fn digest_file(path: &Path, maximum: u64, label: &str) -> Result<String> {
    let mut file = open_real_file(path, label)?;
    let initial = file
        .metadata()
        .with_context(|| format!("measuring {label} {}", path.display()))?
        .len();
    ensure!(
        initial <= maximum,
        "{label} is {initial} bytes, exceeding its {maximum}-byte limit"
    );
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
