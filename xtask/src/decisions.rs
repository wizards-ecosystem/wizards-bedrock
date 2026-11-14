//! The decision log: numbered without a gap or repeat, and every `D-NNN` cited names an entry.

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
pub fn defined(root: &Path) -> Result<BTreeSet<u32>> {
    Ok(entries(root)?
        .into_iter()
        .filter_map(|entry| entry.number)
        .collect())
}

/// The whole text of one decision; an undefined decision is an error, never an empty text.
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
/// The digits must end at a boundary: a run of hex that opens with a digit is not a citation.
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

#[cfg(test)]
fn cited(text: &str) -> Vec<(u32, usize)> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find("D-") {
            let after = &rest[at + 2..];
            // A citation is preceded by nothing word-like, so `MD-5` is not one.
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

#[cfg(test)]
fn searchable(root: &Path) -> Result<Vec<std::path::PathBuf>> {
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
                // Linker scripts cite decisions, so they are searched too.
                Some("rs" | "md" | "toml" | "h" | "cpp" | "tla" | "cfg" | "ld")
            ) {
                found.push(path);
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests;
