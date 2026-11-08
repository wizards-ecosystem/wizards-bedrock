//! Shared filesystem fixtures and source lookups for host tests.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

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

pub(crate) fn absolute_root(name: &str) -> PathBuf {
    #[cfg(windows)]
    let filesystem_root = Path::new(r"C:\");
    #[cfg(not(windows))]
    let filesystem_root = Path::new("/");
    filesystem_root.join(name)
}

pub(crate) fn remove_directory_link(path: &Path) {
    #[cfg(windows)]
    let removed = std::fs::remove_dir(path);
    #[cfg(not(windows))]
    let removed = std::fs::remove_file(path);
    removed.unwrap_or_else(|error| panic!("remove directory link {}: {error}", path.display()));
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

pub(crate) fn function_after<'a>(source: &'a str, opening: &str) -> Option<&'a str> {
    source_through(source, opening, opening)
}

pub(crate) fn source_through<'a>(source: &'a str, start: &str, last: &str) -> Option<&'a str> {
    let from = source.find(start)?;
    let end = function_end(source, from + source[from..].find(last)?)?;
    Some(&source[from + start.len()..end])
}

pub(crate) fn function_end(source: &str, start: usize) -> Option<usize> {
    let rest = &source[start..];
    let mut ends = FunctionEnd {
        at: start + rest.len() - rest.trim_start().len(),
        end: None,
    };
    ends.visit_file(&syn::parse_file(source).ok()?);
    ends.end
}

struct FunctionEnd {
    at: usize,
    end: Option<usize>,
}

impl FunctionEnd {
    fn consider(&mut self, function: &impl Spanned, block: &syn::Block) {
        let braces = block.brace_token.span;
        let header = function.span().byte_range().start..braces.open().byte_range().start;
        if header.contains(&self.at) {
            self.end = Some(braces.close().byte_range().end);
        }
    }
}

impl<'ast> Visit<'ast> for FunctionEnd {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        self.consider(function, &function.block);
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.consider(function, &function.block);
        visit::visit_impl_item_fn(self, function);
    }

    fn visit_trait_item_fn(&mut self, function: &'ast syn::TraitItemFn) {
        if let Some(block) = &function.default {
            self.consider(function, block);
        }
        visit::visit_trait_item_fn(self, function);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_function_slice_ends_at_its_own_block_whatever_the_comments_say() {
        let source = "impl Queue {\n    /// Claim one entry.\n    pub fn claim(&self) -> &str {\n        fn nested() {}\n        // }\n        \"} fn settle(\"\n    }\n    fn settle(&self) {}\n}\n\ntrait Owner {\n    fn owned(&self) -> bool { true }\n}\n";
        let claim = "    pub fn claim(";

        assert_eq!(
            function_after(source, claim),
            Some(
                "&self) -> &str {\n        fn nested() {}\n        // }\n        \"} fn settle(\"\n    }"
            )
        );
        assert_eq!(function_after(source, "fn nested("), Some(") {}"));
        assert_eq!(
            function_after(source, "fn owned("),
            Some("&self) -> bool { true }")
        );
        assert_eq!(
            source_through(source, claim, "\n    fn settle(")
                .map(|slice| slice.ends_with("fn settle(&self) {}")),
            Some(true)
        );
        let start = source.find(claim).expect("claim method");
        assert_eq!(
            function_end(source, start),
            Some(source.find("\n    fn settle(").expect("settle method"))
        );
        assert_eq!(function_after(source, "impl Queue"), None);
    }

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
