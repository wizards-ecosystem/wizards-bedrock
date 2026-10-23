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
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

#[derive(Debug, Default, PartialEq, Eq)]
struct UnsafeFacts {
    lines: usize,
    blocks: usize,
    declarations: usize,
}

impl<'ast> Visit<'ast> for UnsafeFacts {
    fn visit_expr_unsafe(&mut self, node: &'ast syn::ExprUnsafe) {
        let span = node.span();
        self.lines += span.end().line.saturating_sub(span.start().line) + 1;
        self.blocks += 1;
        visit::visit_expr_unsafe(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.declarations += usize::from(node.sig.unsafety.is_some());
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.declarations += usize::from(node.sig.unsafety.is_some());
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        self.declarations += usize::from(node.sig.unsafety.is_some());
        visit::visit_trait_item_fn(self, node);
    }

    fn visit_foreign_item_fn(&mut self, node: &'ast syn::ForeignItemFn) {
        self.declarations += usize::from(node.sig.unsafety.is_some());
        visit::visit_foreign_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        self.declarations += usize::from(node.unsafety.is_some());
        visit::visit_item_impl(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        self.declarations += usize::from(node.unsafety.is_some());
        visit::visit_item_trait(self, node);
    }

    fn visit_item_foreign_mod(&mut self, node: &'ast syn::ItemForeignMod) {
        self.declarations += usize::from(node.unsafety.is_some());
        visit::visit_item_foreign_mod(self, node);
    }
}

/// Lines of `unsafe` `wost` may hold.
///
/// The measurement and per-file audit records below are executable evidence;
/// change history belongs in the decision log (D-124).
pub const BUDGET: usize = 388;

/// What the crate measured when [`BUDGET`] was last reviewed.
///
/// Kept separately because a ceiling cannot reveal drift below it.
pub const MEASURED: usize = 388;

/// Lines inside `unsafe` blocks under `directory`, and how many blocks.
///
/// Parsed Rust syntax rather than occurrences of the word: comments, strings
/// and unsafe declarations are not operations and do not enter this count.
pub fn measure(directory: &Path) -> Result<(usize, usize)> {
    let mut lines = 0;
    let mut blocks = 0;
    for source in sources(directory)? {
        let text = std::fs::read_to_string(&source)
            .with_context(|| format!("reading {}", source.display()))?;
        let (file_lines, file_blocks) = count(&text)
            .with_context(|| format!("parsing unsafe syntax in {}", source.display()))?;
        lines += file_lines;
        blocks += file_blocks;
    }
    Ok((lines, blocks))
}

fn sources(directory: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in crate::paths::directory_entries(&next)? {
            let path = entry.path();
            if entry.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn count(text: &str) -> Result<(usize, usize)> {
    let syntax = syn::parse_file(text).context("parsing Rust source")?;
    let mut facts = UnsafeFacts::default();
    facts.visit_file(&syntax);
    Ok((facts.lines, facts.blocks))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::sha256;

    type AuditRecord = (&'static str, usize, usize, &'static str);

    const AUDITED: &[AuditRecord] = &[
        (
            "wost/src/arch/x86_64/apic.rs",
            3,
            0,
            "3619e18a579da07b79774cc3aeaed42c41039218f7aaa7a82c2a7cb3cfe89604",
        ),
        (
            "wost/src/arch/x86_64/cpu.rs",
            22,
            3,
            "ff949f1cead5bf7ebf06c387cdfd673a4aeabc9699c4fa6c2a318c8334ccf0e3",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            23,
            2,
            "d464094cc35edfa067d7d2ef3dfb55d06491769d7d1c9c7e97cdd2ce8eabb83f",
        ),
        (
            "wost/src/arch/x86_64/kpti.rs",
            2,
            0,
            "1f86a1c6569b16f1c934f69134f9eab72109102971c374799d5ab6d6cb5be15e",
        ),
        (
            "wost/src/arch/x86_64/msr.rs",
            2,
            1,
            "e1f019c6f2a2b6df61070d201c08f578a4fc02af943cdfed68a480687bfbddfb",
        ),
        (
            "wost/src/arch/x86_64/paging.rs",
            14,
            0,
            "ffcd1a49b6854135bb875a9da60e514b17ec02016ab2be2822c855fa991cf370",
        ),
        (
            "wost/src/arch/x86_64/percpu.rs",
            2,
            0,
            "4cf6b632b37b1be9af9c66a885a9b7e40650a494a69a075ac5e00f4b265d916c",
        ),
        (
            "wost/src/arch/x86_64/port.rs",
            16,
            10,
            "a698cd8d042170c7f89b027c61e48ba6b67779f9c01cb9f86f26679a906b555e",
        ),
        (
            "wost/src/arch/x86_64/segment.rs",
            14,
            0,
            "71e5cfef2416ec47e01dd6504544710c7e7138943d6b95ce1a5ebd492c310196",
        ),
        (
            "wost/src/arch/x86_64/shared_memory.rs",
            2,
            2,
            "6143ca421ca29d7c048b84661bdd13768bd3a0d5c40d4746c2831e500304a8d0",
        ),
        (
            "wost/src/arch/x86_64/task.rs",
            14,
            2,
            "e5e26213bedfd484ea89f1de2f4b3aca57099512f24646b508251c926b54ba7a",
        ),
        (
            "wost/src/arch/x86_64/usercopy.rs",
            5,
            1,
            "8aa48581c0a0c3c929b63982c6b53d8d11f6044d902142624f966110766a65d5",
        ),
        (
            "wost/src/arch/x86_64/xstate.rs",
            9,
            3,
            "5d360ea2c8fd1276816d2bbc282f4550126f36050c3f001d181748948a105495",
        ),
        (
            "wost/src/boot/limine_glue.rs",
            3,
            0,
            "1c2e8d9218d0d70befd4402a5440372ef39af823e1c5c83da4e6e4b19451d470",
        ),
        (
            "wost/src/boot/mod.rs",
            0,
            1,
            "ef4fd0dcbf2c1d6ca31501b4d60ed3d7386fbce18bea96b3c672d47ecd7556c1",
        ),
        (
            "wost/src/cell.rs",
            0,
            1,
            "92e1579f8d90b74e8d0897dc5a1da8e23b238131d156d7a565b36ae82b6b4ce9",
        ),
        (
            "wost/src/mm/heap.rs",
            6,
            5,
            "306112aa957d6bb5e374897a6fd938fcaef7e96952db942acc716c5ebfc05eff",
        ),
        (
            "wost/src/mm/mod.rs",
            17,
            3,
            "2892232c249bcc53e7e930a06385a21fc212cce605e98a83d61fdcd54793d16a",
        ),
        (
            "wost/src/mmio.rs",
            2,
            2,
            "982617ae9b24dd72bfeeca53ce86e76546b4ee3edc827bd43db5cdc51018a99b",
        ),
        (
            "wost/src/sync.rs",
            4,
            4,
            "feda23d612ceb63311ffbe033360da789f2c7a319c5537c6edb1f6431b9058c5",
        ),
    ];

    fn unsafe_file_inventory(root: &Path) -> Vec<(String, UnsafeFacts, String)> {
        let source_root = root.join("wost/src");
        let mut inventory = Vec::new();
        for source in sources(&source_root).expect("wost source walk") {
            let text = std::fs::read_to_string(&source)
                .unwrap_or_else(|_| panic!("reading {}", source.display()));
            let syntax =
                syn::parse_file(&text).unwrap_or_else(|_| panic!("parsing {}", source.display()));
            let mut facts = UnsafeFacts::default();
            facts.visit_file(&syntax);
            if facts.blocks == 0 && facts.declarations == 0 {
                continue;
            }
            let relative = source
                .strip_prefix(root)
                .expect("source below workspace")
                .to_string_lossy()
                .replace('\\', "/");
            let normalised = text.replace("\r\n", "\n");
            let digest = sha256::hex(&sha256::digest(normalised.as_bytes()));
            inventory.push((relative, facts, digest));
        }
        inventory
    }

    fn render_inventory(inventory: &[(String, UnsafeFacts, String)]) -> String {
        inventory
            .iter()
            .map(|(path, facts, digest)| {
                format!(
                    "    (\"{path}\", {}, {}, \"{digest}\"),",
                    facts.blocks, facts.declarations
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_block_is_counted_and_a_declaration_is_not() {
        let body = |source: &str| count(&format!("fn f() {{ {source} }}")).expect("syntax");

        assert_eq!(body("unsafe { one() }").0, 1);
        assert_eq!(count("unsafe fn f() { one() }").expect("syntax"), (0, 0));
        assert_eq!(count("unsafe impl Send for T {}").expect("syntax"), (0, 0));
        assert_eq!(count("// unsafe { not_code() }").expect("syntax"), (0, 0));
        assert_eq!(body("unsafe {\n    if x {\n        y\n    }\n}").0, 5);
        assert_eq!(body("unsafe { a }\nunsafe { b }"), (2, 2));
    }

    #[test]
    fn every_unsafe_source_has_one_reviewed_fingerprint() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let inventory = unsafe_file_inventory(root);
        let rendered = render_inventory(&inventory);

        assert_eq!(
            inventory.len(),
            AUDITED.len(),
            "unsafe source inventory changed; review it and replace AUDITED with:\n{rendered}"
        );
        for ((path, facts, digest), expected) in inventory.iter().zip(AUDITED) {
            assert_eq!(
                (
                    path.as_str(),
                    facts.blocks,
                    facts.declarations,
                    digest.as_str()
                ),
                *expected,
                "unsafe source changed; review it and replace AUDITED with:\n{rendered}"
            );
        }
    }

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
        // A near-zero result means the scanner stopped reaching the TCB.
        assert!(lines > 100, "only {lines} lines found; the scan is broken");

        assert_eq!(
            lines, MEASURED,
            "wost holds {lines} lines of unsafe, not the {MEASURED} recorded beside the budget;              update MEASURED and say in the commit what changed"
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

    /// Check the declarations because counting blocks cannot see an unused
    /// `unsafe fn` or `unsafe impl` (D-147).
    #[test]
    fn the_crate_boundary_is_still_declared() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");

        let mut crates = 0;
        for entry in crate::paths::directory_entries(&root.join("libs"))
            .expect("complete libs directory listing")
        {
            let path = entry.path();
            if !entry.is_dir() {
                continue;
            }
            let lib = path.join("src").join("lib.rs");
            let text = std::fs::read_to_string(&lib)
                .unwrap_or_else(|_| panic!("reading {}", lib.display()));
            assert!(
                text.contains("#![forbid(unsafe_code)]"),
                "{} does not forbid unsafe",
                lib.display()
            );
            crates += 1;
        }
        assert!(
            crates > 10,
            "only {crates} libs crates found; the walk is broken"
        );

        // `kernel/` denies rather than forbids, because `wost`'s `entry!`
        // macro expands `#[allow(unsafe_code)]` items into it (D-67).
        let main = std::fs::read_to_string(root.join("kernel/src/main.rs")).expect("main.rs");
        assert!(
            main.contains("#![deny(unsafe_code)]"),
            "kernel/ does not deny unsafe"
        );

        let wost = std::fs::read_to_string(root.join("wost/src/lib.rs")).expect("wost lib.rs");
        assert!(
            wost.contains("#![allow(unsafe_code)]"),
            "wost no longer lifts the workspace deny; the boundary has moved"
        );
    }

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
