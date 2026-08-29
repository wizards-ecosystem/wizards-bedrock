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
/// 360 lines, measured 2026-08-25 during the Phase 0–5 audit. It was 343 at
/// the Phase 5 close; the seventeen added are D-120's fixup recovery, D-123's
/// four soundness fixes, the two pointer reads behind the leaf-nesting check's
/// reporting (D-126), and the four constructions behind `claim_pair` and
/// `claim_block_pair` — two `Port::new` and two `PortBlock::new`, each sound
/// for the reason the single-port doors are, over one all-or-none reservation
/// instead of two (D-138) — and three MSR writes that shut `syscall` and
/// `sysenter` and re-open one for the check that watches them shut (D-139),
/// and the one `save` behind the probe that proves a save fits its area
/// (D-144). D-185 added two read/write operations at the shared-mapping
/// lifetime and transaction preflight boundaries.
/// D-186 added one call into the fixed write-protection fault probe.
/// D-199 added three lines to consume a unique MMIO capability, inspect its
/// exact leaves and construct its time-bounded register region.
/// D-206 removed the two unused dynamic-port read/write operations.
/// D-221 added two volatile shared-payload operations around the atomic ring
/// publication boundary.
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
pub const MEASURED: usize = 370;

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

/// Lines spanned by unsafe block expressions in one Rust source.
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
            "e27f1fe75e0e801281415c7943b87c7bc24c61ff0a92b50f00d493d5c3f59238",
        ),
        (
            "wost/src/arch/x86_64/cpu.rs",
            21,
            2,
            "e84721dc04988ea3c00d9c38bd89dd666e6c6c53b7be5961b58674056dafec33",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            20,
            1,
            "ad6203b4ad4418c3541037af167d85ecdd0f6c7372074cff15a73d2246207900",
        ),
        (
            "wost/src/arch/x86_64/msr.rs",
            2,
            1,
            "84db1467d56a638c35b858dd8c63742e5cf50dee1c26e2a4b5f1d667cb05d11f",
        ),
        (
            "wost/src/arch/x86_64/paging.rs",
            24,
            0,
            "99ebc503ea354c5966fd08f1652a1320e233f06b9af85163614bcdcb8fafb4e4",
        ),
        (
            "wost/src/arch/x86_64/percpu.rs",
            2,
            0,
            "bd26af12aa55f2ceff430383663543d3dc5dd6cbe5f2d5ba809d5424599b6e9d",
        ),
        (
            "wost/src/arch/x86_64/port.rs",
            16,
            10,
            "c779d323e49685e9119d2a0b8ff358d8f83ed4debf114c5a2ec51a48b172e2f6",
        ),
        (
            "wost/src/arch/x86_64/segment.rs",
            15,
            0,
            "b3345cf8fd6b970e6e5f210aa37a4b0a3b9cfae35f6f174ee08a42cade363927",
        ),
        (
            "wost/src/arch/x86_64/task.rs",
            20,
            4,
            "0d88186a4e74c3e19f3e3cc42795def90040f4076f9b504fe3aac4bc3522f24a",
        ),
        (
            "wost/src/arch/x86_64/usercopy.rs",
            5,
            1,
            "eaa21a6e89da38b48ce5430372dd8da8dfb35af8c697c3239c2690da7461d87d",
        ),
        (
            "wost/src/arch/x86_64/xstate.rs",
            9,
            3,
            "d0c4396b88caa5c618a6fc537feb0086448fad58f5fbbfeb26b159a50c567dc5",
        ),
        (
            "wost/src/boot/limine_glue.rs",
            3,
            0,
            "3602d77654140ed3f8dbfb2cfe37919b405750d6a7348ef0f8c15e590fab41c4",
        ),
        (
            "wost/src/boot/mod.rs",
            0,
            1,
            "250ad7c8df1c5d987f935c6408a531f5bc14af4ecc6c186a9682339e41321d58",
        ),
        (
            "wost/src/cell.rs",
            0,
            1,
            "92e1579f8d90b74e8d0897dc5a1da8e23b238131d156d7a565b36ae82b6b4ce9",
        ),
        (
            "wost/src/mm/heap.rs",
            7,
            5,
            "c6c9e484c519d2f67bbfbe421baa705fc6429314173d4766704490cbcc73c26e",
        ),
        (
            "wost/src/mm/mod.rs",
            13,
            2,
            "7a20c2a3a1fa6566e21381483e946e8f0235d11b9e41a58fbe00fdcb87ea7850",
        ),
        (
            "wost/src/mmio.rs",
            6,
            3,
            "47e6af301af29669ed3a8f30fe80413e81576370757c15ea5a2e29f0db96a205",
        ),
        (
            "wost/src/sync.rs",
            4,
            4,
            "d7af9374912fe0dc9157f55b6ae6eafc322c79158cba87f52df1e8bad2051d49",
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
        // Nesting is one block, and its lines are all of them.
        assert_eq!(body("unsafe {\n    if x {\n        y\n    }\n}").0, 5);
        // Two blocks are two.
        assert_eq!(body("unsafe { a }\nunsafe { b }"), (2, 2));
    }

    /// Every unsafe source and the safe wrappers beside it were read at this
    /// exact revision.
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

    /// Safe kernel modules cannot borrow the running page tables wholesale.
    #[test]
    fn the_running_space_has_no_broad_safe_borrow() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let source = std::fs::read_to_string(root.join("kernel/src/mem.rs")).expect("mem.rs");

        assert!(
            source.contains("\nfn with_active_space<T>("),
            "the private address-space authority moved; review its new boundary"
        );
        assert!(
            !source.contains("\npub(crate) fn with_active_space<T>("),
            "safe kernel modules can borrow the running AddressSpace wholesale"
        );
    }

    /// And every `libs/` crate, which carry `#![forbid(unsafe_code)]`.
    /// Every `libs/` crate still forbids unsafe, and `kernel/` still denies it.
    ///
    /// The counter beside this measures **blocks**, deliberately — `unsafe fn`
    /// and `unsafe impl` are declarations, not operations, and the budget is
    /// about operations. But that leaves the crate boundary itself unchecked:
    /// a `libs/` crate that dropped `#![forbid(unsafe_code)]` and grew an
    /// `unsafe fn` has zero blocks and passes, and the boundary is D-13's
    /// whole claim (D-147).
    ///
    /// Checked as text, because the attribute is the thing being asserted and
    /// a build that compiles proves only that nothing violated it *yet*.
    #[test]
    fn the_crate_boundary_is_still_declared() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");

        let mut crates = 0;
        for entry in std::fs::read_dir(root.join("libs")).expect("libs") {
            let path = entry.expect("a libs entry").path();
            if !path.is_dir() {
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

        // And `wost` is the one that lifts it, which is what makes the other
        // two mean something.
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
