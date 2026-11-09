//! What the documents say about the tree, checked against the tree.
//!
//! Documentation may name planned or rejected crates, so missing paths are not
//! automatically errors. Every missing documented crate instead needs one
//! explicit exemption with its reason, making drift reviewable (D-124).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A `libs/` crate the documents name and the tree does not have.
#[derive(Debug, Clone, Copy)]
pub struct Absent {
    /// The directory name it would have under `libs/`.
    pub crate_name: &'static str,
    /// Why it is named without existing.
    pub because: &'static str,
}

/// Every crate the documents may name without the tree holding it.
///
/// Adding a line here is the reviewable act. Removing one when the crate gets
/// written is the other half, and the check requires that too: an exemption
/// nobody removes is an exemption nobody reads.
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
///
/// # Errors
///
/// If `docs/` cannot be read.
pub fn named_in_docs(root: &Path) -> Result<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for path in markdown_in_docs(root)? {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        found.extend(mentions(&text));
    }
    Ok(found)
}

/// Every Markdown file anywhere under `docs/`, in path order.
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

/// The crate names in `libs/<name>` mentions, in order of appearance.
fn mentions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("libs/") {
        let after = &rest[at + "libs/".len()..];
        let name: String = after
            .chars()
            .take_while(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || *char == '-')
            .collect();
        // A trailing hyphen means the scan stopped mid-word on punctuation
        // rather than at the end of a name.
        if !name.is_empty() && !name.ends_with('-') {
            found.push(name);
        }
        rest = after;
    }
    found
}

#[cfg(test)]
mod tests;
