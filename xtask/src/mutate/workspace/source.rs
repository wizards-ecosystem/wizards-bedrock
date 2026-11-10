use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::paths::Paths;
use crate::sha256;

use super::journal::is_hex;
use super::tree::{
    canonical_existing_real_directory, is_reparse_point, open_real_file, prepare_real_child,
    require_direct_file, require_direct_file_if_present, require_real_child,
    require_real_directory_metadata,
};
use super::{STATE_ROOT_NAME, read_bounded_bytes};

// Git may contend with the compiler-heavy host suite on release builders.
// Keep the query bounded without making scheduler latency a gate failure.
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const MAX_GIT_IDENTITY_BYTES: usize = 4096;
const MAX_GIT_STATUS_BYTES: usize = 1024 * 1024;
const MAX_GIT_INDEX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TRACKED_PATH_BYTES: usize = 4 * 1024 * 1024;
const MAX_TRACKED_FILES: usize = 4096;
pub(super) const MAX_TRACKED_FILE_BYTES: u64 = 16 * 1024 * 1024;
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

/// Populate an already-owned directory with newly written copies of every
/// tracked file from one clean checkout, and prove that the copied bytes match.
pub(crate) fn copy_clean_tracked_to(source: &Path, destination: &Path) -> Result<()> {
    let baseline = Baseline::discover(source)?;
    let destination =
        canonical_existing_real_directory(destination, "owned clean source-copy destination")?;
    copy_tracked(source, &destination, &baseline.tracked)?;
    ensure!(
        digest_tracked(&destination, &baseline.tracked)? == baseline.source_digest,
        "the owned clean source copy differs from the checkout"
    );
    Ok(())
}

#[derive(Debug)]
pub(super) struct Baseline {
    pub(super) root: PathBuf,
    pub(super) commit: String,
    pub(super) tracked: Vec<PathBuf>,
    pub(super) source_digest: String,
    pub(super) index_path: PathBuf,
    pub(super) index_digest: String,
}

pub(super) fn parse_tracked_paths(listed: &[u8]) -> Result<Vec<PathBuf>> {
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
    pub(super) fn discover(root: &Path) -> Result<Self> {
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
            ["rev-parse", "--path-format=absolute", "--git-path", "index"],
            MAX_GIT_IDENTITY_BYTES,
            "mutation Git index path",
        )?;
        let index_path = require_confined_absolute_file(root, &index_name, "mutation Git index")?;
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

    pub(super) fn verify(&self) -> Result<()> {
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

pub(super) fn git_text<const N: usize>(
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

pub(super) fn copy_tracked(from: &Path, to: &Path, tracked: &[PathBuf]) -> Result<()> {
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
pub(super) fn write_source(destination: &Path, bytes: &[u8]) -> Result<()> {
    write_source_with(destination, bytes, |from, to| std::fs::rename(from, to))
}

pub(super) fn write_source_with(
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

pub(super) fn finish_staged_operation_with<T>(
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

pub(super) fn restore_file(snapshot: &Path, workspace: &Path, relative: &str) -> Result<()> {
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

fn read_direct_file(parent: &Path, path: &Path, label: &str) -> Result<Vec<u8>> {
    require_direct_file(parent, path, label)?;
    let bytes = read_bounded_bytes(path, MAX_TRACKED_FILE_BYTES, label)?;
    require_direct_file(parent, path, label)?;
    Ok(bytes)
}

pub(super) fn restore_workspace(
    snapshot: &Path,
    workspace: &Path,
    tracked: &[PathBuf],
) -> Result<()> {
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

pub(super) fn require_mutation_layout(snapshot: &Path, workspace: &Path) -> Result<()> {
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

pub(super) fn require_real_tree(root: &Path, label: &str) -> Result<()> {
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

pub(super) fn require_confined_file(root: &Path, relative: &str, label: &str) -> Result<PathBuf> {
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

pub(super) fn require_confined_absolute_file(
    root: &Path,
    absolute: &str,
    label: &str,
) -> Result<PathBuf> {
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

pub(super) fn validate_relative(relative: &str) -> Result<()> {
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

pub(super) fn digest_tracked(root: &Path, tracked: &[PathBuf]) -> Result<String> {
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
        let mut file = open_real_file(&path, "tracked mutation source")?;
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

pub(super) fn digest_file(path: &Path, maximum: u64, label: &str) -> Result<String> {
    digest_file_with(path, maximum, label, || Ok(()))
}

pub(super) fn digest_file_with<F>(
    path: &Path,
    maximum: u64,
    label: &str,
    after_measurement: F,
) -> Result<String>
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
