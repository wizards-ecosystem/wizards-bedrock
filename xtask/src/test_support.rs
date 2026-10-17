//! Shared filesystem fixtures for host tests.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
const RESERVATION_ATTEMPTS: usize = 4096;

pub(crate) fn reserve_temp_directory(label: &str) -> PathBuf {
    reserve_directory_with(&std::env::temp_dir(), label, &NEXT_DIRECTORY)
}

pub(crate) fn reserve_directory(parent: &Path, label: &str) -> PathBuf {
    reserve_directory_with(parent, label, &NEXT_DIRECTORY)
}

pub(crate) fn reserve_temp_file(label: &str) -> (PathBuf, File) {
    reserve_file(&std::env::temp_dir(), label, &NEXT_DIRECTORY)
}

fn reserve_directory_with(parent: &Path, label: &str, next: &AtomicU64) -> PathBuf {
    for _ in 0..RESERVATION_ATTEMPTS {
        let sequence = next.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!("{label}-{}-{sequence}", std::process::id()));
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

fn reserve_file(parent: &Path, label: &str, next: &AtomicU64) -> (PathBuf, File) {
    for _ in 0..RESERVATION_ATTEMPTS {
        let sequence = next.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!("{label}-{}-{sequence}", std::process::id()));
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::write(&mut options, true);
        match OpenOptions::open(&options, &candidate) {
            Ok(file) => return (candidate, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!(
                "create exclusive test fixture file {}: {error}",
                candidate.display()
            ),
        }
    }
    panic!("could not reserve a unique {label} test fixture file")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_candidates_are_skipped_without_touching_their_bytes() {
        let parent = reserve_temp_directory("wizard-temp-reservation-test");
        let next = AtomicU64::new(0);
        let stale = parent.join(format!("candidate-{}-0", std::process::id()));
        std::fs::create_dir(&stale).expect("stale fixture candidate");
        std::fs::write(stale.join("sentinel"), b"stale bytes").expect("stale sentinel");

        let reserved = reserve_directory_with(&parent, "candidate", &next);
        let expected = parent.join(format!("candidate-{}-1", std::process::id()));
        let sentinel = std::fs::read(stale.join("sentinel"));
        std::fs::remove_dir_all(parent).expect("remove reservation fixture");

        assert_eq!(reserved, expected);
        assert_eq!(sentinel.expect("stale sentinel remains"), b"stale bytes");
    }

    #[test]
    fn stale_file_candidates_are_skipped_without_touching_their_bytes() {
        let parent = reserve_temp_directory("wizard-temp-file-reservation-test");
        let next = AtomicU64::new(0);
        let stale = parent.join(format!("candidate-{}-0", std::process::id()));
        std::fs::write(&stale, b"stale bytes").expect("stale fixture file");

        let (reserved, file) = reserve_file(&parent, "candidate", &next);
        drop(file);
        let expected = parent.join(format!("candidate-{}-1", std::process::id()));
        let sentinel = std::fs::read(&stale);
        std::fs::remove_dir_all(parent).expect("remove file reservation fixture");

        assert_eq!(reserved, expected);
        assert_eq!(
            sentinel.expect("stale fixture file remains"),
            b"stale bytes"
        );
    }
}
