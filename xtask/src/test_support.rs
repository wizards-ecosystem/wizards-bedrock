//! Temporary directories and absolute roots for host tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const RESERVATION_ATTEMPTS: usize = 4096;

pub(crate) fn reserve_temp_directory(label: &str) -> PathBuf {
    for _ in 0..RESERVATION_ATTEMPTS {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let candidate =
            std::env::temp_dir().join(format!("{label}-{}-{sequence}", std::process::id()));
        match std::fs::create_dir(&candidate) {
            Ok(()) => return candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!(
                "create exclusive test fixture directory {}: {error}",
                candidate.display()
            ),
        }
    }
    panic!("could not reserve a unique {label} test fixture directory")
}

pub(crate) fn absolute_root(name: &str) -> PathBuf {
    #[cfg(windows)]
    let filesystem_root = Path::new(r"C:\");
    #[cfg(not(windows))]
    let filesystem_root = Path::new("/");
    filesystem_root.join(name)
}
