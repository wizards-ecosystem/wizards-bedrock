use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::files::is_reparse_point;

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

/// The handle, opened without following any component,
/// is the authority for later relative operations.
pub(crate) fn open_real_directory(path: &Path, label: &str) -> Result<File> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("reading the current directory")?
            .join(path)
    };
    let mut components = absolute.components().peekable();
    let mut traversed = PathBuf::new();
    while let Some(component @ (Component::Prefix(_) | Component::RootDir)) = components.peek() {
        traversed.push(component.as_os_str());
        components.next();
    }
    ensure!(
        traversed.has_root(),
        "{label} has no absolute filesystem root: {}",
        path.display()
    );
    let mut directory = open_directory_without_following(&traversed)
        .with_context(|| format!("opening directory root {}", traversed.display()))?;
    let root_metadata = directory
        .metadata()
        .with_context(|| format!("inspecting opened directory {}", traversed.display()))?;
    require_real_directory_metadata(&traversed, &root_metadata, label)?;
    let mut options = fs_at::OpenOptions::default();
    options.read(true).follow(false);
    for component in components {
        let Component::Normal(name) = component else {
            bail!(
                "{label} contains a non-normal directory component: {}",
                path.display()
            );
        };
        let child = options
            .open_dir_at(&directory, name)
            .with_context(|| format!("opening directory component {}", name.to_string_lossy()))?;
        traversed.push(name);
        let metadata = child
            .metadata()
            .with_context(|| format!("inspecting opened directory {}", traversed.display()))?;
        require_real_directory_metadata(&traversed, &metadata, label)?;
        directory = child;
    }
    Ok(directory)
}

#[cfg(windows)]
fn open_directory_without_following(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(unix)]
fn open_directory_without_following(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
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

pub(super) fn require_real_child(parent: &Path, child: &Path, label: &str) -> Result<()> {
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

pub(super) fn require_real_directory_metadata(
    path: &Path,
    metadata: &std::fs::Metadata,
    label: &str,
) -> Result<()> {
    ensure!(
        metadata_is_real_directory(metadata),
        "{label} is not a real directory: {}",
        path.display()
    );
    Ok(())
}

pub(crate) fn metadata_is_real_directory(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_dir()
        && !metadata.file_type().is_symlink()
        && !is_reparse_point(metadata)
}

#[derive(Debug)]
pub(super) struct SweepLock {
    _file: File,
}

pub(super) fn acquire_lock(path: &Path) -> Result<SweepLock> {
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

/// A path check alone cannot stop a link swapped in before an ordinary open, so the opened
/// handle is checked. Only the final component is guarded: Windows opens it without following
/// a link, and elsewhere the post-open entry check refuses a swap. Earlier ones are not checked.
pub(crate) fn open_real_file(path: &Path, label: &str) -> Result<File> {
    let file =
        open_without_following(path).with_context(|| format!("opening {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("inspecting opened {}", path.display()))?;
    require_real_file_metadata(path, &metadata, label)?;
    require_opened_entry(path, &metadata, label)?;
    Ok(file)
}

#[cfg(windows)]
fn open_without_following(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(windows))]
fn open_without_following(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

// Windows opened the entry itself; elsewhere the entry must still be the file the handle reached.
#[cfg(windows)]
const fn require_opened_entry(
    _path: &Path,
    _opened: &std::fs::Metadata,
    _label: &str,
) -> Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn require_opened_entry(path: &Path, opened: &std::fs::Metadata, label: &str) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let entry = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspecting {}", path.display()))?;
    ensure!(
        entry.dev() == opened.dev() && entry.ino() == opened.ino(),
        "{label} was replaced while it was opened: {}",
        path.display()
    );
    Ok(())
}
