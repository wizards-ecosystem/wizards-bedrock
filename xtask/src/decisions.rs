//! The decision log, checked against the code that cites it.
//!
//! `docs/004-decision-log.md` is the authority on why this kernel is shaped the
//! way it is, and roughly two hundred comments cite it by number. Nothing
//! checked those numbers, and the cost of that showed up as **D-121 cited in
//! eight places with no such entry in the log** — a decision the code says was
//! taken and the record does not contain. Every one of those citations reads as
//! a pointer to a rationale, and every one led nowhere.
//!
//! Two tests, both host-side and both milliseconds, because this is a text
//! property and belongs where `cargo test` reaches it (D-41):
//!
//! - the entries run from `D-01` upward with no gap and no repeat, so a number
//!   cannot be skipped when one is minted;
//! - every `D-NNN` written anywhere in the tree names an entry that exists.
//!
//! The second is the one that would have caught D-121. The first is what stops
//! the next gap being introduced, which is the cheaper end to fix it from.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Where the log lives, relative to the workspace root.
pub const LOG: &str = "docs/004-decision-log.md";

/// Every decision number the log defines, in order.
///
/// # Errors
///
/// If the log cannot be read.
pub fn defined(root: &Path) -> Result<BTreeSet<u32>> {
    let path = root.join(LOG);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(text
        .lines()
        .filter_map(|line| line.strip_prefix("## D-"))
        .filter_map(|rest| number(rest).map(|(number, _)| number))
        .collect())
}

/// The decision number a `D-`-stripped string starts with, and what follows.
///
/// The digits must end at a boundary, symmetric with the one the caller checks
/// in front: a run of hex that merely opens with a digit is not a citation, and
/// reporting one would name a decision nobody claimed. This function's own doc
/// comment was the first thing the check caught.
fn number(rest: &str) -> Option<(u32, &str)> {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let after = &rest[digits.len()..];
    if after.starts_with(|char: char| char.is_alphanumeric() || char == '_') {
        return None;
    }
    Some((digits.parse().ok()?, after))
}

/// Every `D-NNN` cited in a file, with the line it is on.
fn cited(text: &str) -> Vec<(u32, usize)> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find("D-") {
            let after = &rest[at + 2..];
            // A citation is preceded by nothing word-like, so `MD-5` and an
            // identifier ending in `d_` are not one.
            let boundary = rest[..at]
                .chars()
                .next_back()
                .is_none_or(|char| !char.is_alphanumeric() && char != '_');
            if boundary && let Some((decision, _)) = number(after) {
                found.push((decision, index + 1));
            }
            rest = after;
        }
    }
    found
}

/// Every file worth searching for citations: sources, docs, and manifests.
fn searchable(root: &Path) -> Result<Vec<PathBuf>> {
    /// Directories whose contents are not ours to check.
    const SKIP: &[&str] = &["target", ".git", "vendor", "tools", "node_modules"];

    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .with_context(|| format!("reading {}", directory.display()))?
        {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if path.is_dir() {
                if !SKIP.contains(&name) && !name.starts_with('.') {
                    pending.push(path);
                }
            } else if matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("rs" | "md" | "toml" | "h" | "cpp" | "tla" | "cfg")
            ) {
                found.push(path);
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace root, from where this crate sits below it.
    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .to_path_buf()
    }

    /// The log numbers its entries from one upward with no gap and no repeat.
    ///
    /// A gap is a decision that was minted and lost, or a number skipped by
    /// accident; the log is append-only, so either is a mistake rather than a
    /// style. The repeat half is not hypothetical either — two sessions
    /// appending on the same day is exactly how it would happen.
    #[test]
    fn the_decisions_are_numbered_without_a_gap() {
        let numbers = defined(&root()).expect("reading the decision log");
        assert!(!numbers.is_empty(), "the log defines no decisions");

        let last = *numbers.last().expect("non-empty");
        let missing: Vec<u32> = (1..=last).filter(|n| !numbers.contains(n)).collect();
        assert!(
            missing.is_empty(),
            "the log jumps over {missing:?}; a cited number with no entry is a rationale that leads nowhere"
        );

        // A repeat cannot survive the set above, so it is counted separately.
        let text = std::fs::read_to_string(root().join(LOG)).expect("reading the log");
        let headings = text
            .lines()
            .filter(|line| line.starts_with("## D-"))
            .count();
        assert_eq!(
            headings,
            numbers.len(),
            "two entries share a number, or one heading is malformed"
        );
    }

    /// Every `D-NNN` written anywhere names an entry the log defines.
    ///
    /// This is the one that would have caught D-121, cited in eight places
    /// with nothing behind it.
    #[test]
    fn every_citation_names_a_decision_that_exists() {
        let root = root();
        let defined = defined(&root).expect("reading the decision log");
        let mut dangling = Vec::new();

        for path in searchable(&root).expect("walking the tree") {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (decision, line) in cited(&text) {
                if !defined.contains(&decision) {
                    let shown = path.strip_prefix(&root).unwrap_or(&path);
                    dangling.push(format!("{}:{line} cites D-{decision:02}", shown.display()));
                }
            }
        }

        assert!(
            dangling.is_empty(),
            "{} citation(s) name a decision the log does not define:\n  {}",
            dangling.len(),
            dangling.join("\n  ")
        );
    }

    #[test]
    fn a_citation_is_recognised_and_a_lookalike_is_not() {
        assert_eq!(cited("as D-105 says"), vec![(105, 1)]);
        assert_eq!(cited("(D-12, D-100)"), vec![(12, 1), (100, 1)]);
        // Not citations: a boundary in front, a boundary behind, and digits
        // too long to be one.
        assert!(cited("MD-5 is a hash").is_empty());
        assert!(cited("D-0DEADBEEF").is_empty(), "hex, not a decision");
        assert!(cited("D-1234").is_empty(), "too long to be a decision");
        assert!(cited("D- alone").is_empty());
        // And the forms that are: mid-sentence, parenthesised, possessive.
        assert_eq!(cited("D-105's rule"), vec![(105, 1)]);
    }
}
