//! Page-size constants stated in more than one crate, checked to agree.
//! Definitions are discovered by name, so a new transcription is covered when it is written.

use std::path::Path;

use anyhow::{Context, Result};

/// Constant names that mean the size of a page.
/// Adding a name here widens the comparison; a new definition is found without editing this file.
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
/// A computed value fails here: only a literal can be compared.
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
        None => <u64 as core::str::FromStr>::from_str(&text).ok(),
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

        // The hardware's page size, not merely a number five files agree on:
        // they would agree if all five were wrong together.
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
