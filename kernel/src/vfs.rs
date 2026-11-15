use fat32::FatError;
use path_walk::PathError;

// Fixed, so the length of a ring 3 argument cannot set the resolver's work.
const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Directory {
    Fat { cluster: u32 },
    Tmp { index: usize },
    Dev,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum File {
    Fat { cluster: u32, size: u32 },
    Tmp { at: usize, len: usize },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Node {
    Directory(Directory),
    File(File),
    Device(crate::devfs::Device),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Error {
    Path(PathError),
    Backend(FatError),
    NoVolume,
    NotADirectory,
    NotFound,
}

impl From<PathError> for Error {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

impl From<FatError> for Error {
    fn from(error: FatError) -> Self {
        match error {
            FatError::NotFound => Self::NotFound,
            FatError::WrongKind => Self::NotADirectory,
            other => Self::Backend(other),
        }
    }
}

pub(crate) fn root() -> Option<Directory> {
    crate::fs::mount()
        .and_then(Result::ok)
        .map(|volume| Directory::Fat {
            cluster: volume.root_cluster(),
        })
}

pub(crate) const fn tmp_root() -> Directory {
    Directory::Tmp {
        index: crate::tmpfs::ROOT,
    }
}

// Each step uses the node it judged and never looks a name up twice.
pub(crate) fn resolve(at: Directory, path: &[u8]) -> Result<Node, Error> {
    let mut components = [b"".as_slice(); MAX_DEPTH];
    let depth = path_walk::split(path, &mut components)?;

    // `.` and `a/..` name the starting directory, which only a rights-checked duplicate may copy.
    if depth == 0 {
        return Err(Error::Path(PathError::Empty));
    }

    match at {
        Directory::Fat { cluster } => resolve_in_fat(cluster, &components[..depth]),
        Directory::Tmp { index } => resolve_in_tmp(index, &components[..depth]),
        Directory::Dev => resolve_in_dev(&components[..depth]),
    }
}

fn resolve_in_tmp(from: usize, components: &[&[u8]]) -> Result<Node, Error> {
    let mut directory = from;

    for (index, name) in components.iter().enumerate() {
        let found = crate::tmpfs::lookup(directory, name).ok_or(Error::NotFound)?;
        let is_last = index + 1 == components.len();

        match found {
            crate::tmpfs::Found::Directory(next) => {
                if is_last {
                    return Ok(Node::Directory(Directory::Tmp { index: next }));
                }
                directory = next;
            }
            crate::tmpfs::Found::File { at, len } => {
                if !is_last {
                    // A file walked through would be answered from whatever its bytes say.
                    return Err(Error::NotADirectory);
                }
                return Ok(Node::File(File::Tmp { at, len }));
            }
        }
    }

    Err(Error::NotFound)
}

fn resolve_in_dev(components: &[&[u8]]) -> Result<Node, Error> {
    let (first, rest) = components.split_first().ok_or(Error::NotFound)?;
    let device = crate::devfs::lookup(first).ok_or(Error::NotFound)?;
    if rest.is_empty() {
        Ok(Node::Device(device))
    } else {
        Err(Error::NotADirectory)
    }
}

fn resolve_in_fat(from: u32, components: &[&[u8]]) -> Result<Node, Error> {
    let mut volume = crate::fs::mount()
        .ok_or(Error::NoVolume)?
        .map_err(Error::from)?;
    let entry = volume.resolve(from, components).map_err(Error::from)?;
    Ok(if entry.is_directory() {
        Node::Directory(Directory::Fat {
            cluster: entry.cluster,
        })
    } else {
        Node::File(File::Fat {
            cluster: entry.cluster,
            size: entry.size,
        })
    })
}

pub(crate) fn read(file: File, offset: u32, into: &mut [u8]) -> Result<usize, Error> {
    match file {
        File::Fat { cluster, size } => crate::fs::mount()
            .ok_or(Error::NoVolume)?
            .map_err(Error::from)?
            .read_file(cluster, size, offset, into)
            .map_err(Error::from),
        File::Tmp { at, len } => Ok(crate::tmpfs::read(at, len, offset as usize, into)),
    }
}

pub(crate) fn enumerate(
    directory: Directory,
    mut found: impl FnMut(&[u8], bool) -> bool,
) -> Result<(), Error> {
    match directory {
        Directory::Fat { cluster } => {
            let mut volume = crate::fs::mount()
                .ok_or(Error::NoVolume)?
                .map_err(Error::from)?;
            let mut cursor = fat32::DirectoryCursor::new(cluster);
            loop {
                let chunk = volume.directory_chunk(&mut cursor).map_err(Error::from)?;
                for entry in chunk.entries() {
                    if !found(entry.name(), entry.is_directory()) {
                        return Ok(());
                    }
                }
                if cursor.is_done() {
                    return Ok(());
                }
            }
        }
        Directory::Tmp { index } => {
            let listing = crate::tmpfs::list(index).ok_or(Error::NotFound)?;
            for entry in listing.entries() {
                if !found(entry.name(), entry.is_directory()) {
                    break;
                }
            }
            Ok(())
        }
        Directory::Dev => {
            crate::devfs::walk(found);
            Ok(())
        }
    }
}
