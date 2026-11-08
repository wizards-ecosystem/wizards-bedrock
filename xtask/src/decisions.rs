//! The decision log, checked against the code that cites it.
//!
//! `docs/004-decision-log.md` is the authority on why this kernel is shaped the
//! way it is, either as one log of `## D-N` entries or as the index of one file
//! per decision under `docs/decisions/`. Host tests enforce two text properties
//! where `cargo test` can reach them (D-41):
//!
//! - the entries run from `D-01` upward with no gap and no repeat, so a number
//!   cannot be skipped when one is minted;
//! - every `D-NNN` written anywhere in the tree names an entry that exists.
//!

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

/// Where the log lives, relative to the workspace root.
pub const LOG: &str = "docs/004-decision-log.md";

/// Where the decisions live once split one per file; the log is then their index.
pub const DIRECTORY: &str = "docs/decisions";

struct Entry {
    number: Option<u32>,
    text: String,
}

/// Every decision number the log defines, in order.
///
/// # Errors
///
/// If the log cannot be read, or a split layout is malformed.
pub fn defined(root: &Path) -> Result<BTreeSet<u32>> {
    Ok(entries(root)?
        .into_iter()
        .filter_map(|entry| entry.number)
        .collect())
}

/// The whole text of one decision.
///
/// # Errors
///
/// If the log cannot be read, a split layout is malformed, or the decision is
/// not defined.
pub fn decision_text(root: &Path, decision: u32) -> Result<String> {
    entries(root)?
        .into_iter()
        .find(|entry| entry.number == Some(decision))
        .map(|entry| entry.text)
        .with_context(|| format!("D-{decision:02} is not defined"))
}

fn entries(root: &Path) -> Result<Vec<Entry>> {
    let directory = root.join(DIRECTORY);
    if directory
        .try_exists()
        .with_context(|| format!("probing {}", directory.display()))?
    {
        split_entries(root, &directory)
    } else {
        Ok(log_entries(&read(&root.join(LOG))?))
    }
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn log_entries(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("## D-") {
            entries.push(Entry {
                number: number(rest).map(|(number, _)| number),
                text: String::new(),
            });
            inside = true;
        } else if line.starts_with("## ") {
            inside = false;
        }
        if inside && let Some(entry) = entries.last_mut() {
            entry.text.push_str(line);
        }
    }
    entries
}

fn split_entries(root: &Path, directory: &Path) -> Result<Vec<Entry>> {
    let mut paths: Vec<PathBuf> = crate::paths::directory_entries(directory)?
        .iter()
        .map(crate::paths::DirectoryEntry::path)
        .collect();
    paths.sort();
    let mut entries = Vec::new();
    let mut expected = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let filed = file_number(name)
            .with_context(|| format!("{} is not a `NNNN-slug.md` decision", path.display()))?;
        let text = read(&path)?;
        ensure!(
            opens_as_decision(&text, filed),
            "{} must open with `# D-{filed:02} <title>`, a blank line and a `Status:` line",
            path.display()
        );
        let heading = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("# "))
            .unwrap_or_default();
        expected.push((heading.to_owned(), format!("decisions/{name}")));
        entries.push(Entry {
            number: Some(filed),
            text,
        });
    }
    let listed = index_rows(&read(&root.join(LOG))?);
    ensure!(
        listed == expected,
        "{LOG} must link every decision file once, in order, under its own heading"
    );
    Ok(entries)
}

fn opens_as_decision(text: &str, filed: u32) -> bool {
    let mut lines = text.lines();
    let titled = lines
        .next()
        .and_then(|line| line.strip_prefix("# D-"))
        .and_then(number)
        .is_some_and(|(number, title)| {
            number == filed && title.len() > 1 && title.starts_with(' ')
        });
    titled
        && lines.next() == Some("")
        && lines
            .next()
            .is_some_and(|line| line.starts_with("Status: "))
}

fn file_number(name: &str) -> Option<u32> {
    let (digits, rest) = name.split_at_checked(4)?;
    let slug = rest.strip_prefix('-')?.strip_suffix(".md")?;
    if slug.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    <u32 as core::str::FromStr>::from_str(digits).ok()
}

fn index_rows(index: &str) -> Vec<(String, String)> {
    index
        .lines()
        .filter_map(|line| {
            let (label, rest) = line.strip_prefix("| [")?.split_once("](")?;
            let (target, _) = rest.split_once(')')?;
            Some((label.to_owned(), target.to_owned()))
        })
        .collect()
}

/// The decision number a `D-`-stripped string starts with, and what follows.
///
/// The digits must end at a boundary, symmetric with the one the caller checks
/// in front: a run of hex that merely opens with a digit is not a citation, and
/// reporting one would name a decision nobody claimed.
fn number(rest: &str) -> Option<(u32, &str)> {
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let after = &rest[digits.len()..];
    if after.starts_with(|char: char| char.is_alphanumeric() || char == '_') {
        return None;
    }
    Some((<u32 as core::str::FromStr>::from_str(&digits).ok()?, after))
}

/// Every `D-NNN` cited in a file, with the line it is on.
#[cfg(test)]
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

#[cfg(test)]
fn searchable_text(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|reason| panic!("reading searchable file {}: {reason}", path.display()))
}

/// Every file worth searching for citations: sources, docs, and manifests.
#[cfg(test)]
fn searchable(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    /// Directories whose contents are not ours to check.
    const SKIP: &[&str] = &["target", ".git", "vendor", "tools", "node_modules"];

    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in crate::paths::directory_entries(&directory)? {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if entry.is_dir() {
                if !SKIP.contains(&name) && !name.starts_with('.') {
                    pending.push(path);
                }
            } else if matches!(
                path.extension().and_then(|extension| extension.to_str()),
                // Linker scripts cite decisions and therefore belong in the
                // source inventory (D-147).
                Some("rs" | "md" | "toml" | "h" | "cpp" | "tla" | "cfg" | "ld")
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
    fn root() -> std::path::PathBuf {
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
        let headings = entries(&root()).expect("reading the decision log").len();
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
            let text = searchable_text(&path);
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

    const FIRST: (&str, &str) = (
        "0001-first.md",
        "# D-01 First\n\nStatus: Accepted\n\nBody.\n",
    );
    const SECOND: (&str, &str) = (
        "0002-second.md",
        "# D-2 Second\n\nStatus: Accepted, 2026-10-25.\n\nAmends D-01.\n",
    );
    const FIRST_ROW: &str = "| [D-01 First](decisions/0001-first.md) | Accepted | 2026-07-18 |";
    const SECOND_ROW: &str = "| [D-2 Second](decisions/0002-second.md) | Accepted | 2026-10-25 |";

    fn split_tree(files: &[(&str, &str)], rows: &[&str]) -> std::path::PathBuf {
        let root = crate::test_support::reserve_temp_directory("wizard-decisions");
        std::fs::create_dir_all(root.join(DIRECTORY)).expect("creating the decision directory");
        for (name, text) in files {
            std::fs::write(root.join(DIRECTORY).join(name), text).expect("writing a decision");
        }
        let index = format!(
            "# 004 - Decision log\n\n| Decision | Status | Date |\n|---|---|---|\n{}\n",
            rows.join("\n")
        );
        std::fs::write(root.join(LOG), index).expect("writing the index");
        root
    }

    fn refused(files: &[(&str, &str)], rows: &[&str]) -> bool {
        let root = split_tree(files, rows);
        let refused = defined(&root).is_err();
        std::fs::remove_dir_all(&root).expect("removing the decision fixture");
        refused
    }

    #[test]
    fn the_split_layout_defines_one_decision_per_file() {
        let root = split_tree(&[FIRST, SECOND], &[FIRST_ROW, SECOND_ROW]);
        let numbers = defined(&root).expect("reading the split layout");
        let second = decision_text(&root, 2).expect("reading D-2");
        let count = entries(&root).expect("reading the split layout").len();
        std::fs::remove_dir_all(&root).expect("removing the decision fixture");
        assert_eq!(numbers, BTreeSet::from([1, 2]));
        assert_eq!(count, 2);
        assert_eq!(second, SECOND.1);
    }

    #[test]
    fn an_index_that_disagrees_with_the_files_is_refused() {
        assert!(
            refused(&[FIRST, SECOND], &[SECOND_ROW, FIRST_ROW]),
            "out of order"
        );
        assert!(refused(&[FIRST, SECOND], &[FIRST_ROW]), "a file left out");
        assert!(
            refused(&[FIRST, SECOND], &[FIRST_ROW, SECOND_ROW, SECOND_ROW]),
            "a file listed twice"
        );
        assert!(
            refused(
                &[FIRST, SECOND],
                &[
                    FIRST_ROW,
                    "| [D-2 Other](decisions/0002-second.md) | Accepted | 2026-10-25 |"
                ]
            ),
            "a title that is not the file's"
        );
    }

    #[test]
    fn a_decision_file_out_of_shape_is_refused() {
        assert!(
            !refused(&[FIRST], &[FIRST_ROW]),
            "the well-formed fixture is accepted"
        );
        assert!(
            refused(
                &[("0001-first.md", "# D-3 First\n\nStatus: Accepted\n")],
                &["| [D-3 First](decisions/0001-first.md) | Accepted | |"]
            ),
            "a heading that disagrees with its file name"
        );
        assert!(
            refused(
                &[("0001-first.md", "# D-01 First\n\nBody.\n")],
                &[FIRST_ROW]
            ),
            "no status line"
        );
        assert!(
            refused(
                &[("0001-first.txt", FIRST.1)],
                &["| [D-01 First](decisions/0001-first.txt) | Accepted | |"]
            ),
            "a decision that is not Markdown"
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

    #[test]
    fn an_unreadable_searchable_file_fails_the_citation_proof() {
        let missing = root().join("xtask/src/__missing_decision_proof_file__");
        assert!(
            !missing.exists(),
            "the missing-file fixture unexpectedly exists"
        );
        assert!(std::panic::catch_unwind(|| searchable_text(&missing)).is_err());
    }
}
