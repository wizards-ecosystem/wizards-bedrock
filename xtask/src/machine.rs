//! Facts about the machine that more than one crate states, checked to agree.
//!
//! A page is 4096 bytes, and five places in this tree say so: `frame-space`
//! and `slab-layout` call it `FRAME_SIZE`, `stack-layout` and `x86-paging`
//! call it `PAGE_SIZE`, and `kernel/src/mem.rs` has its own. They are four
//! crates and a kernel module with no dependency between them, which is why
//! each declares its own — and nothing compared them (D-160).
//!
//! Every layout in the kernel is computed from one of these. A frame allocator
//! handing out 4096-byte frames to a slab allocator that believes they are
//! 2048 does not fail a test; it fails at the second slab. The literal is the
//! same in all five today, which is exactly the state a duplication is in right
//! up until it is not.
//!
//! **The sites are discovered, not listed** (D-157). What this module names is
//! the *fact* — which constant names mean "the size of a page" — and it then
//! goes and finds every definition of one. A sixth transcription is covered by
//! the check the day it is written.

use std::path::Path;

use anyhow::{Context, Result};

/// Constant names that all mean "the size of a page".
///
/// The list is the check's subject rather than its data: adding a name here
/// widens what is compared, and adding a *definition* anywhere in the tree is
/// found without touching this file.
const PAGE_SIZE_NAMES: [&str; 2] = ["PAGE_SIZE", "FRAME_SIZE"];

/// The trees a machine constant can live in.
const ROOTS: [&str; 3] = ["libs", "wost/src", "kernel/src"];

/// One definition of a machine constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    /// Where it is, relative to the workspace root.
    pub file: String,
    /// The line it is on, 1-based.
    pub line: usize,
    /// The name it is given.
    pub name: String,
    /// What it is set to.
    pub value: u64,
}

/// Every definition of a page-size constant under `root`.
///
/// # Errors
///
/// If a source tree cannot be read, or a definition's value is not a literal
/// this can evaluate — which is itself worth failing on, because a page size
/// computed rather than stated is a page size this cannot compare.
pub fn page_size_definitions(root: &Path) -> Result<Vec<Definition>> {
    let mut found = Vec::new();
    for tree in ROOTS {
        let mut pending = vec![root.join(tree)];
        while let Some(next) = pending.pop() {
            let entries =
                std::fs::read_dir(&next).with_context(|| format!("reading {}", next.display()))?;
            for entry in entries {
                let path = entry?.path();
                if path.is_dir() {
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

/// Pull every page-size definition out of one file's text.
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

/// Evaluate a Rust integer literal, decimal or hexadecimal.
fn literal(text: &str) -> Option<u64> {
    let text = text.replace('_', "");
    match text.strip_prefix("0x") {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
    }

    /// Every crate that states a page size states the same one.
    #[test]
    fn every_page_size_in_the_tree_is_the_same_page_size() {
        let found = page_size_definitions(root()).expect("the source trees are readable");

        // A regex that stops matching is the failure that reads as a pass:
        // nothing found means nothing compared, and this reports success.
        assert!(
            found.len() >= 5,
            "only {} page-size definitions were found, so the scan is wrong: {found:#?}",
            found.len()
        );

        let (first, rest) = found.split_first().expect("at least one definition");
        for other in rest {
            assert_eq!(
                other.value,
                first.value,
                "{}:{} says {} is {}, {}:{} says {} is {}",
                other.file,
                other.line,
                other.name,
                other.value,
                first.file,
                first.line,
                first.name,
                first.value
            );
        }

        // And it is the page size the hardware has, not merely a number five
        // files agree on — which is what they would also be if all five were
        // wrong together.
        assert_eq!(first.value, 4096, "a page is 4096 bytes on x86-64");
    }

    #[test]
    fn a_hexadecimal_page_size_is_read_as_the_same_number() {
        let mut found = Vec::new();
        scan(
            "made/up.rs",
            "pub const PAGE_SIZE: u64 = 0x1_000;\nconst FRAME_SIZE: usize = 4096;\n",
            &mut found,
        )
        .expect("both lines state a literal");
        assert_eq!(found.len(), 2, "{found:#?}");
        assert_eq!(found[0].value, 4096);
        assert_eq!(found[1].value, 4096);
    }

    #[test]
    fn a_page_size_that_is_computed_rather_than_stated_is_refused() {
        let mut found = Vec::new();
        let refused = scan(
            "made/up.rs",
            "pub const PAGE_SIZE: u64 = 1 << 12;\n",
            &mut found,
        );
        assert!(
            refused.is_err(),
            "a computed page size cannot be compared and must not pass quietly"
        );
    }
}
