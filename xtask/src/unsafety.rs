//! Count lines inside `unsafe` blocks in `wost` and hold that count to a budget.
//! A budget that cannot fire is not a check; comments and declarations are not operations.

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
pub const BUDGET: usize = 386;

/// What the crate measured when [`BUDGET`] was last reviewed.
/// Kept separately because a ceiling cannot reveal drift below it.
pub const MEASURED: usize = 386;

/// Lines inside `unsafe` blocks under `directory`, and how many blocks.
/// Parsed syntax: comments, strings, and declarations are not operations.
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
            "270458aeed7af26973e46803fe2e850e6cc9c12dcb19b34634ad337c9fd070ff",
        ),
        (
            "wost/src/arch/x86_64/cpu.rs",
            22,
            3,
            "ecce35feb9b7fa0d71210480b4f270f7d74c6bd930f8d06f7d1d75c9cc25f28d",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            23,
            2,
            "5e51be12a58a24cba455378ce07fc641bedd2aecde74a46c13d7b8f883b568ae",
        ),
        (
            "wost/src/arch/x86_64/kpti.rs",
            2,
            0,
            "6010b7466563d53cca7320c4e5efe06da5e51f0f1df67c9653a6ec84663ae8a0",
        ),
        (
            "wost/src/arch/x86_64/msr.rs",
            2,
            1,
            "c277cb79205ba0018012ba90d70d2618f41590093b0937c874a73254b5964163",
        ),
        (
            "wost/src/arch/x86_64/paging.rs",
            14,
            0,
            "84265573baa6ea7014c34aca361709d52c49f3bd37c7e5ef4495115425fe26d8",
        ),
        (
            "wost/src/arch/x86_64/percpu.rs",
            2,
            0,
            "8fbe2f7c898ace42568113259a4d311fa2e41fd3440fd60a9cc83ec7b70b05e0",
        ),
        (
            "wost/src/arch/x86_64/port.rs",
            16,
            10,
            "5ecf3b5ef6ee9b7673d659b7a458379008db9ac72fcaffa3021b1abbf5d34a91",
        ),
        (
            "wost/src/arch/x86_64/segment.rs",
            14,
            0,
            "67291cd83ece3470c4694337831b355f4bc1cbdcf4c040314b7fd8ca9f03d501",
        ),
        (
            "wost/src/arch/x86_64/shared_memory.rs",
            2,
            2,
            "21a7992a4cb834274416c74b5a3b0de3f22b7facc27ddbbc12bde8c6f8eaf3e6",
        ),
        (
            "wost/src/arch/x86_64/task.rs",
            14,
            2,
            "7678136c97ab186c3516f4a74bb5c2db11ecc7f8bcb5fcb9d7f1c66af1e926d4",
        ),
        (
            "wost/src/arch/x86_64/usercopy.rs",
            5,
            1,
            "a60dd3d145a34a9f9e6a9ae0f83c50ac9fd51f901ff7ef7c0d78e5a3635dbe2c",
        ),
        (
            "wost/src/arch/x86_64/xstate.rs",
            9,
            3,
            "faa6844d69eb2088247bcb3385eca1401672f20441fac384d61daa7d760f1d51",
        ),
        (
            "wost/src/boot/limine_glue.rs",
            3,
            0,
            "716b93e40af90b0a3b6c8569b4e048424b5072a3c4484ac63f44d64d40d028bd",
        ),
        (
            "wost/src/boot/mod.rs",
            0,
            1,
            "813ba6bd456ac865a936bdd37668c7f28a9b24ac93f155ceffbfe1a5de9f6074",
        ),
        (
            "wost/src/cell.rs",
            0,
            1,
            "0d26fb8fb7ab8b7e9de0329c9531006a77e9bf8f448e0a85a5b2e25d2ae06f85",
        ),
        (
            "wost/src/mm/heap.rs",
            6,
            5,
            "af61cb302c4ee5fce60898eb621f99662ca152d7c9abe591684ad38af8b4feef",
        ),
        (
            "wost/src/mm/mod.rs",
            17,
            3,
            "e1366f5257705e0438d7ef7cb48805d82db0953f465cbcacf23e069b28e367c3",
        ),
        (
            "wost/src/mmio.rs",
            2,
            2,
            "07b811a919ca5e3d90f2dc1252c2d5d4cfa6dca7a8708413f40bc4e9588040ec",
        ),
        (
            "wost/src/sync.rs",
            4,
            4,
            "30890a520f87c4377839791482edabbd11fcc3051af528738fa9274a15eedc67",
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

    /// The counter must agree with `#![deny(unsafe_code)]`, including an `#[allow]` slipped in beside it.
    #[test]
    fn the_kernel_has_no_unsafe_at_all() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let (lines, blocks) = measure(&root.join("kernel").join("src")).expect("kernel sources");
        assert_eq!((lines, blocks), (0, 0), "kernel/ grew an unsafe block");
    }

    /// Check the declarations because counting blocks cannot see an unused
    /// `unsafe fn` or `unsafe impl`.
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
        // macro expands `#[allow(unsafe_code)]` items into it.
        let main = std::fs::read_to_string(root.join("kernel/src/main.rs")).expect("main.rs");
        assert!(
            main.contains("#![deny(unsafe_code)]"),
            "kernel/ does not deny unsafe"
        );

        let wost = std::fs::read_to_string(root.join("wost/src/lib.rs")).expect("wost lib.rs");
        assert!(
            wost.contains("#![allow(") && wost.contains("unsafe_code"),
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
