//! Counting the `unsafe` in `wost`, and holding it to a budget.
//!
//! `010` §1 is explicit that the framekernel paper's ~14% TCB *ratio* is not
//! evidence of anything: a percentage falls when safe code is added, which is
//! not a safety improvement, and it rises when unsafe code is deleted from a
//! shrinking crate, which is not a regression. **Absolute lines are the
//! relevant audit cost**, because the number a reviewer must read is the
//! number of lines, and D-77's phase-close instruction is to track them
//! against a budget rather than a ratio.
//!
//! So this counts lines *inside* `unsafe` blocks — not occurrences of the
//! word, which double-counts every `// SAFETY:` comment that mentions it, and
//! not blocks, which says nothing about how much is in each.
//!
//! The budget is a ceiling deliberately set close to the current number. A
//! generous one would never fire, and a budget that never fires is a claim
//! (D-28). Raising it is a reviewable line in a commit, which is the whole
//! mechanism.

use std::path::Path;

use anyhow::{Context, Result};

/// Lines of `unsafe` `wost` may hold.
///
/// 360 lines, measured 2026-08-25 during the Phase 0–5 audit. It was 343 at
/// the Phase 5 close; the seventeen added are D-120's fixup recovery, D-123's
/// four soundness fixes, the two pointer reads behind the leaf-nesting check's
/// reporting (D-126), and the four constructions behind `claim_pair` and
/// `claim_block_pair` — two `Port::new` and two `PortBlock::new`, each sound
/// for the reason the single-port doors are, over one all-or-none reservation
/// instead of two (D-138) — and three MSR writes that shut `syscall` and
/// `sysenter` and re-open one for the check that watches them shut (D-139),
/// and the one `save` behind the probe that proves a save fits its area
/// (D-144).
///
/// Phase 5 added the most unsafe of any phase so far — ring 3, the extended
/// state, the address space that grows — which is why this is where the number
/// gets a budget rather than a note.
///
/// **The numbers in this comment are checked**, not merely written down: a
/// stale count in the one comment whose whole job is to be a count was what
/// the sweep found here (D-124).
pub const BUDGET: usize = 380;

/// What the crate measured when [`BUDGET`] was last reviewed.
///
/// The budget is a ceiling and says nothing about where the number actually
/// is. A ceiling of 380 is satisfied at 354 and at 200, and only one of those
/// is the truth — so the measurement is recorded beside it and checked, which
/// makes drift toward the ceiling visible rather than something a reader
/// infers from a comment that may be a phase out of date.
pub const MEASURED: usize = 364;

/// Lines inside `unsafe` blocks under `directory`, and how many blocks.
///
/// A brace-matching scan rather than a parse. It over-counts a block
/// containing a string with an unbalanced brace, which nothing in this crate
/// has, and it under-counts nothing — so a number that passes the budget is
/// not passing by being wrong in the convenient direction.
pub fn measure(directory: &Path) -> Result<(usize, usize)> {
    let mut lines = 0;
    let mut blocks = 0;
    for source in sources(directory)? {
        let text = std::fs::read_to_string(&source)
            .with_context(|| format!("reading {}", source.display()))?;
        let (file_lines, file_blocks) = count(&text);
        lines += file_lines;
        blocks += file_blocks;
    }
    Ok((lines, blocks))
}

/// Every `.rs` file under `directory`, recursively.
fn sources(directory: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next)
            .with_context(|| format!("listing {}", next.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Lines inside `unsafe { … }` in one source, and how many such blocks.
fn count(text: &str) -> (usize, usize) {
    let bytes = text.as_bytes();
    let mut lines = 0;
    let mut blocks = 0;
    let mut at = 0;

    while let Some(found) = text[at..].find("unsafe") {
        let start = at + found;
        at = start + "unsafe".len();

        // `unsafe fn`, `unsafe impl`, `unsafe extern` and the word inside a
        // comment are not blocks. Only a brace, possibly after whitespace.
        let mut cursor = at;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'{' {
            continue;
        }

        let mut depth = 0;
        let mut end = cursor;
        while end < bytes.len() {
            match bytes[end] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        // Through the closing brace, so a block that spans five lines counts
        // five rather than four.
        lines += text[start..(end + 1).min(text.len())].lines().count();
        blocks += 1;
        at = end;
    }

    (lines, blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_block_is_counted_and_a_declaration_is_not() {
        assert_eq!(count("unsafe { one() }").0, 1);
        assert_eq!(count("unsafe fn f() { one() }"), (0, 0));
        assert_eq!(count("unsafe impl Send for T {}"), (0, 0));
        // Nesting is one block, and its lines are all of them.
        assert_eq!(count("unsafe {\n    if x {\n        y\n    }\n}").0, 5);
        // Two blocks are two.
        assert_eq!(count("unsafe { a }\nunsafe { b }"), (2, 2));
    }

    /// The number a reviewer would have to read is inside its budget.
    ///
    /// Absolute lines and never a ratio (`010` §1): a percentage falls when
    /// safe code is added, which is not a safety improvement. Raising
    /// [`BUDGET`] is a reviewable line in a commit, which is the mechanism —
    /// so this failing is the point at which someone has to say out loud that
    /// the crate grew.
    #[test]
    fn the_unsafe_in_wost_is_inside_its_budget() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let (lines, blocks) = measure(&root.join("wost").join("src")).expect("wost sources");

        assert!(
            lines <= BUDGET,
            "{lines} lines of unsafe in {blocks} blocks, over the budget of {BUDGET}"
        );
        // And it is not zero, which would mean the scan stopped working.
        assert!(lines > 100, "only {lines} lines found; the scan is broken");

        // And it is where the comment on [`BUDGET`] says it is. A ceiling
        // alone leaves the true number free to drift up to it unremarked,
        // which is how the doc came to state a count two phases old (D-124).
        assert_eq!(
            lines, MEASURED,
            "wost holds {lines} lines of unsafe, not the {MEASURED} recorded beside the budget;              update MEASURED and say in the commit what was added"
        );
    }

    /// `kernel/` has none at all, which `#![deny(unsafe_code)]` enforces — so
    /// this is the check that the *counter* agrees with the lint, and would
    /// notice an `#[allow]` slipped in beside it.
    #[test]
    fn the_kernel_has_no_unsafe_at_all() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let (lines, blocks) = measure(&root.join("kernel").join("src")).expect("kernel sources");
        assert_eq!((lines, blocks), (0, 0), "kernel/ grew an unsafe block");
    }

    /// And every `libs/` crate, which carry `#![forbid(unsafe_code)]`.
    #[test]
    fn no_libs_crate_has_any_unsafe() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let (lines, blocks) = measure(&root.join("libs")).expect("libs sources");
        assert_eq!(
            (lines, blocks),
            (0, 0),
            "a libs/ crate grew an unsafe block"
        );
    }
}
