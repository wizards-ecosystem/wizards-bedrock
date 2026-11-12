//! The `libs/` crates the documents name, checked against the tree; a missing one needs `ABSENT`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A `libs/` crate the documents name and the tree does not have.
#[derive(Clone, Copy, Debug)]
pub struct Absent {
    /// The directory name it would have under `libs/`.
    pub crate_name: &'static str,
    /// Why it is named without existing.
    pub because: &'static str,
}

/// Every crate the documents may name without the tree holding it; a test requires each
/// exemption to go once its crate exists.
pub const ABSENT: &[Absent] = &[
    Absent {
        crate_name: "xstate",
        because: "D-82 planned it; the arithmetic turned out to be four instructions and a \
                  CPUID read, which live in wost. The ADR says so at each of its three mentions.",
    },
    Absent {
        crate_name: "lock-rank",
        because: "D-85 declined it: a rank with one non-trivial pair is a type-level apparatus \
                  for a fact a comment states. All four mentions say it was not written.",
    },
    Absent {
        crate_name: "x86-page-tables",
        because: "`008`'s proposed name for what was built as `x86-paging`. A research note \
                  records what was proposed, so the name stays with a pointer beside it.",
    },
    Absent {
        crate_name: "wizfs-format",
        because: "`002` and `010`: the on-disk format crate, owed before WizFS's own phase \
                  (D-73). Not yet written and not yet due.",
    },
    Absent {
        crate_name: "blockdev-sim",
        because: "`010` §11: a fault-injecting block device for testing WizFS's crash \
                  consistency. Belongs to that phase.",
    },
];

/// Every `libs/<name>` the documents mention.
pub fn named_in_docs(root: &Path) -> Result<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for path in markdown_in_docs(root)? {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        found.extend(mentions(&text));
    }
    Ok(found)
}

fn markdown_in_docs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.join("docs")];
    while let Some(directory) = pending.pop() {
        for entry in crate::paths::directory_entries(&directory)? {
            let path = entry.path();
            if entry.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn mentions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("libs/") {
        let after = &rest[at + "libs/".len()..];
        let name: String = after
            .chars()
            .take_while(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || *char == '-')
            .collect();
        // A trailing hyphen means the scan stopped on punctuation, not at the end of a name.
        if !name.is_empty() && !name.ends_with('-') {
            found.push(name);
        }
        rest = after;
    }
    found
}

#[cfg(test)]
mod tests;
