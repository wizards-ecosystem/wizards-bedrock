//! Page-size constants that crates with no dependency between them each declare, checked to
//! agree: every kernel layout is computed from one, and a mismatch fails no test, only a slab.
//! The sites are discovered rather than listed, so a new transcription is covered the day it is
//! written.

use std::path::Path;

use anyhow::{Context, Result};

/// The names are the check's subject: adding one widens what is compared, and a new definition
/// anywhere in the tree is found without touching this file.
const PAGE_SIZE_NAMES: [&str; 2] = ["PAGE_SIZE", "FRAME_SIZE"];

const ROOTS: [&str; 3] = ["libs", "wost/src", "kernel/src"];

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Definition {
    /// Relative to the workspace root.
    pub file: String,
    /// 1-based.
    pub line: usize,
    pub name: String,
    pub value: u64,
}

/// A value that is not a literal fails, because a computed page size cannot be compared.
pub fn page_size_definitions(root: &Path) -> Result<Vec<Definition>> {
    let mut found = Vec::new();
    for tree in ROOTS {
        let mut pending = vec![root.join(tree)];
        while let Some(next) = pending.pop() {
            for entry in crate::paths::directory_entries(&next)? {
                let path = entry.path();
                if entry.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().is_some_and(|extension| extension == "rs") {
                    let text = std::fs::read_to_string(&path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    scan(
                        &path
                            .strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string(),
                        &text,
                        &mut found,
                    )?;
                }
            }
        }
    }
    found.sort_by(|left, right| (&left.file, left.line).cmp(&(&right.file, right.line)));
    Ok(found)
}

fn scan(file: &str, text: &str, into: &mut Vec<Definition>) -> Result<()> {
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("pub const ")
            .or_else(|| trimmed.strip_prefix("const "))
        else {
            continue;
        };
        let Some((name, after)) = rest.split_once(':') else {
            continue;
        };
        if !PAGE_SIZE_NAMES.contains(&name) {
            continue;
        }
        let value = after
            .split_once('=')
            .map(|(_, value)| value.trim().trim_end_matches(';').trim())
            .with_context(|| format!("{file}:{} states no value", index + 1))?;
        into.push(Definition {
            file: file.replace(std::path::MAIN_SEPARATOR, "/"),
            line: index + 1,
            name: name.to_string(),
            value: literal(value)
                .with_context(|| format!("{file}:{} is not a literal: {value}", index + 1))?,
        });
    }
    Ok(())
}

fn literal(text: &str) -> Option<u64> {
    let text = text.replace('_', "");
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => <u64 as core::str::FromStr>::from_str(&text).ok(),
    }
}

#[cfg(test)]
mod tests;
