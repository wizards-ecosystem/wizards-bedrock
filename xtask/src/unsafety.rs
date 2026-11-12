//! Counts lines inside `unsafe` blocks in `wost` against a budget kept close to the current count.
//!
//! Absolute lines are the audit cost, not a ratio (`010` §1); raising `BUDGET` is a reviewed edit.

use std::path::Path;

use anyhow::{Context, Result};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

#[derive(PartialEq, Eq, Default, Debug)]
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

/// What the crate measured when [`BUDGET`] was last reviewed; a ceiling cannot show drift below it.
pub const MEASURED: usize = 386;

/// Lines inside `unsafe` blocks under `directory`, and how many blocks.
///
/// Parsed syntax, so comments, strings and unsafe declarations do not enter the count.
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
mod tests;
