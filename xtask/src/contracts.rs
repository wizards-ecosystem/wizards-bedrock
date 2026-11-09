//! Compile-time and source-shape contracts that stable Rust cannot assert directly.

use std::process::{Command, Output};

use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, Item, Pat};

use crate::paths::{KERNEL_TARGET, Paths};

pub(crate) mod abi;
mod address_space;
mod apic;
mod dma;
mod doors;
mod errors;
mod handoff;
mod machine;
mod paging;
mod pci;
mod ports;
mod publication;
mod recovery;
mod routing;
mod sources;
mod typed_arguments;

struct Probe {
    root: std::path::PathBuf,
}

impl Probe {
    // A probe directory is created exclusively inside this checkout's target
    // tree, so no other account can pre-place or redirect it (D-344).
    fn create(paths: &Paths, label: &str, source: &str) -> Self {
        let target = crate::mutate::workspace::canonical_existing_real_directory(
            paths.target_dir(),
            "contract probe target",
        )
        .expect("contract probe target");
        std::fs::create_dir_all(target.join("contract-probes")).expect("contract probe parent");
        let parent = crate::mutate::workspace::canonical_existing_real_directory(
            &target.join("contract-probes"),
            "contract probe parent",
        )
        .expect("contract probe parent");
        let root = crate::test_support::reserve_directory(&parent, label);
        std::fs::create_dir(root.join("src")).expect("contract source directory");

        let wost = paths
            .root()
            .join("wost")
            .to_string_lossy()
            .replace('\\', "/");
        let nvme = paths
            .root()
            .join("libs/nvme")
            .to_string_lossy()
            .replace('\\', "/");
        let pci_config = paths
            .root()
            .join("libs/pci-config")
            .to_string_lossy()
            .replace('\\', "/");
        let x86_apic = paths
            .root()
            .join("libs/x86-apic")
            .to_string_lossy()
            .replace('\\', "/");
        let manifest = format!(
            "[package]\nname = \"{label}-contract\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             \n[workspace]\n\n[dependencies]\nnvme = {{ path = {nvme:?} }}\npci-config = {{ path = {pci_config:?} }}\nwost = {{ path = {wost:?} }}\nx86-apic = {{ path = {x86_apic:?} }}\n"
        );
        std::fs::write(root.join("Cargo.toml"), manifest).expect("contract manifest");
        std::fs::write(root.join("src/lib.rs"), source).expect("contract source");
        Self { root }
    }

    fn compile(&self, paths: &Paths) -> Output {
        Command::new(env!("CARGO"))
            .current_dir(&self.root)
            .env(
                "CARGO_TARGET_DIR",
                crate::cargo::child_process_path(&paths.target_dir().join("guard-contracts")),
            )
            .args([
                "check",
                "--offline",
                "--target",
                KERNEL_TARGET,
                "--message-format=short",
            ])
            .output()
            .expect("compile contract")
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Create a directory no earlier entry occupies; an existing name is skipped.
fn fresh_probe_directory(
    parent: &std::path::Path,
    label: &str,
    mut next: impl FnMut() -> usize,
) -> std::path::PathBuf {
    core::iter::repeat_with(|| parent.join(format!("{label}-{}-{}", std::process::id(), next())))
        .take(128)
        .find(|root| match std::fs::create_dir(root) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("creating contract probe {}: {error}", root.display()),
        })
        .expect("a fresh contract probe directory")
}

#[test]
fn contract_probes_are_fresh_directories_inside_the_target_tree() {
    let paths = Paths::discover().expect("workspace paths");
    let target = crate::mutate::workspace::canonical_existing_real_directory(
        paths.target_dir(),
        "contract probe target",
    )
    .expect("contract probe target");
    let probe = Probe::create(&paths, "fresh-probe", "#![no_std]\n");
    assert!(
        probe.root.starts_with(target.join("contract-probes")),
        "contract probe escaped the target tree: {}",
        probe.root.display()
    );
    drop(probe);

    let parent = crate::test_support::reserve_directory(
        &target.join("contract-probes"),
        "contract-probe-reservation-test",
    );
    let occupied = parent.join(format!("occupied-{}-0", std::process::id()));
    let outside = parent.join("outside");
    std::fs::create_dir(&outside).expect("outside fixture");
    std::fs::write(outside.join("marker"), b"outside").expect("outside marker");
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&outside, &occupied).expect("occupying link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &occupied).expect("occupying link");
    let mut counter = 0;
    let fresh = fresh_probe_directory(&parent, "occupied", || {
        counter += 1;
        counter - 1
    });
    assert_eq!(
        fresh,
        parent.join(format!("occupied-{}-1", std::process::id()))
    );
    assert_eq!(
        std::fs::read_dir(&outside)
            .expect("outside listing")
            .count(),
        1,
        "a pre-placed link redirected the probe"
    );
    std::fs::remove_dir_all(parent).expect("removing the contract reservation fixture");
}

fn diagnostic_codes(output: &Output) -> Vec<&str> {
    let diagnostics = std::str::from_utf8(&output.stderr).expect("rustc diagnostics are UTF-8");
    diagnostics
        .lines()
        .filter_map(|line| {
            let (_, after_open) = line.split_once("error[")?;
            after_open.split_once(']').map(|(code, _)| code)
        })
        .collect()
}

fn require_error(paths: &Paths, label: &str, source: &str, code: &str) -> Result<(), String> {
    let output = Probe::create(paths, label, source).compile(paths);
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let codes = diagnostic_codes(&output);
    let source_diagnostic = diagnostics.lines().any(|line| {
        (line.contains("src/lib.rs:") || line.contains("src\\lib.rs:"))
            && line.contains(&format!("error[{code}]"))
    });
    (!output.status.success() && codes.len() == 1 && codes[0] == code && source_diagnostic)
        .then_some(())
        .ok_or_else(|| diagnostics.into_owned())
}

fn require_success(paths: &Paths, label: &str, source: &str) -> Result<(), String> {
    let output = Probe::create(paths, label, source).compile(paths);
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    (output.status.success() && diagnostic_codes(&output).is_empty())
        .then_some(())
        .ok_or_else(|| diagnostics.into_owned())
}

fn cfg_test(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test")
    })
}

#[cfg(windows)]
fn is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
const fn is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn rust_sources_below(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let metadata = std::fs::symlink_metadata(directory)
        .unwrap_or_else(|error| panic!("inspecting {}: {error}", directory.display()));
    assert!(
        metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && !is_reparse_point(&metadata),
        "source root is not a real directory: {}",
        directory.display()
    );

    let mut pending = vec![directory.to_path_buf()];
    let mut sources = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut entries = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("reading {}: {error}", directory.display()))
            .map(|entry| {
                entry.unwrap_or_else(|error| panic!("reading {}: {error}", directory.display()))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)
                .unwrap_or_else(|error| panic!("inspecting {}: {error}", path.display()));
            assert!(
                !metadata.file_type().is_symlink() && !is_reparse_point(&metadata),
                "source tree contains a redirected entry: {}",
                path.display()
            );
            if metadata.file_type().is_dir() {
                pending.push(path);
            } else {
                assert!(
                    metadata.file_type().is_file(),
                    "source tree contains a non-file entry: {}",
                    path.display()
                );
                if path.extension().is_some_and(|extension| extension == "rs") {
                    sources.push(path);
                }
            }
        }
    }
    sources.sort();
    sources
}

fn source_tree(directory: &std::path::Path) -> String {
    rust_sources_below(directory)
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
        })
        .collect()
}

fn module_sources(file: &std::path::Path) -> Vec<std::path::PathBuf> {
    let children = file.with_extension("");
    let mut sources = vec![file.to_path_buf()];
    if children.is_dir() {
        sources.extend(rust_sources_below(&children));
    }
    sources
}

fn module_syntax(file: &std::path::Path) -> syn::File {
    let mut sources = module_sources(file).into_iter().map(|path| {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
        syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", path.display()))
    });
    let mut syntax = sources.next().expect("a module has its root file");
    for child in sources {
        syntax.items.extend(child.items);
    }
    syntax
}

fn module_source(root: &std::path::Path, relative: &str) -> String {
    let file = root.join(relative);
    let children = file.with_extension("");
    let mut files = vec![file];
    if children.is_dir() {
        files.extend(rust_sources_below(&children));
    }
    files
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
        })
        .collect()
}

fn is_named_path(expression: &Expr, expected: &str) -> bool {
    matches!(expression, Expr::Path(path)
        if path.qself.is_none()
            && path.path.segments.len() == 1
            && path.path.segments[0].ident == expected)
}

fn selftest_cfg(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        if !attribute.path().is_ident("cfg") {
            return false;
        }
        let Ok(syn::Meta::NameValue(feature)) = attribute.parse_args::<syn::Meta>() else {
            return false;
        };
        feature.path.is_ident("feature")
            && matches!(
                &feature.value,
                Expr::Lit(value)
                    if matches!(&value.lit, syn::Lit::Str(name) if name.value() == "selftest")
            )
    })
}

fn cfg_not_selftest(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        if !attribute.path().is_ident("cfg") {
            return false;
        }
        let Ok(syn::Meta::List(not)) = attribute.parse_args::<syn::Meta>() else {
            return false;
        };
        not.path.is_ident("not") && not.tokens.to_string() == "feature = \"selftest\""
    })
}

fn observer_item_attributes(item: &Item) -> &[syn::Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn observer_impl_item_attributes(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Macro(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        _ => &[],
    }
}

fn observer_expr_attributes(expression: &Expr) -> &[syn::Attribute] {
    match expression {
        Expr::Array(value) => &value.attrs,
        Expr::Assign(value) => &value.attrs,
        Expr::Async(value) => &value.attrs,
        Expr::Await(value) => &value.attrs,
        Expr::Binary(value) => &value.attrs,
        Expr::Block(value) => &value.attrs,
        Expr::Break(value) => &value.attrs,
        Expr::Call(value) => &value.attrs,
        Expr::Cast(value) => &value.attrs,
        Expr::Closure(value) => &value.attrs,
        Expr::Const(value) => &value.attrs,
        Expr::Continue(value) => &value.attrs,
        Expr::Field(value) => &value.attrs,
        Expr::ForLoop(value) => &value.attrs,
        Expr::Group(value) => &value.attrs,
        Expr::If(value) => &value.attrs,
        Expr::Index(value) => &value.attrs,
        Expr::Infer(value) => &value.attrs,
        Expr::Let(value) => &value.attrs,
        Expr::Lit(value) => &value.attrs,
        Expr::Loop(value) => &value.attrs,
        Expr::Macro(value) => &value.attrs,
        Expr::Match(value) => &value.attrs,
        Expr::MethodCall(value) => &value.attrs,
        Expr::Paren(value) => &value.attrs,
        Expr::Path(value) => &value.attrs,
        Expr::Range(value) => &value.attrs,
        Expr::RawAddr(value) => &value.attrs,
        Expr::Reference(value) => &value.attrs,
        Expr::Repeat(value) => &value.attrs,
        Expr::Return(value) => &value.attrs,
        Expr::Struct(value) => &value.attrs,
        Expr::Try(value) => &value.attrs,
        Expr::TryBlock(value) => &value.attrs,
        Expr::Tuple(value) => &value.attrs,
        Expr::Unary(value) => &value.attrs,
        Expr::Unsafe(value) => &value.attrs,
        Expr::While(value) => &value.attrs,
        Expr::Yield(value) => &value.attrs,
        _ => &[],
    }
}

fn asm_tokens(tokens: proc_macro2::TokenStream, flattened: &mut Vec<String>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Group(group) => asm_tokens(group.stream(), flattened),
            token => flattened.push(token.to_string()),
        }
    }
}

#[derive(Default)]
struct DiscardedAddressSpaceReclaims(Vec<usize>);

impl DiscardedAddressSpaceReclaims {
    fn record(&mut self, expression: &Expr) {
        if let Some(call) = address_space_reclaim(expression) {
            self.0.push(call.method.span().start().line);
        }
    }
}

impl<'ast> Visit<'ast> for DiscardedAddressSpaceReclaims {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if pattern_discards_value(&local.pat)
            && let Some(initializer) = &local.init
        {
            self.record(&initializer.expr);
        }
        visit::visit_local(self, local);
    }

    fn visit_stmt(&mut self, statement: &'ast syn::Stmt) {
        if let syn::Stmt::Expr(expression, Some(_)) = statement {
            self.record(expression);
        }
        visit::visit_stmt(self, statement);
    }

    fn visit_expr_assign(&mut self, assignment: &'ast syn::ExprAssign) {
        if matches!(assignment.left.as_ref(), Expr::Infer(_)) {
            self.record(&assignment.right);
        }
        visit::visit_expr_assign(self, assignment);
    }

    fn visit_expr_let(&mut self, expression: &'ast syn::ExprLet) {
        if pattern_discards_value(&expression.pat) {
            self.record(&expression.expr);
        }
        visit::visit_expr_let(self, expression);
    }

    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if expression.args.len() == 1
            && let Expr::Path(function) = expression.func.as_ref()
            && function
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "drop")
        {
            self.record(&expression.args[0]);
        }
        visit::visit_expr_call(self, expression);
    }
}

fn pattern_discards_value(pattern: &Pat) -> bool {
    match pattern {
        Pat::Wild(_) => true,
        Pat::Paren(pattern) => pattern_discards_value(&pattern.pat),
        Pat::Reference(pattern) => pattern_discards_value(&pattern.pat),
        Pat::Type(pattern) => pattern_discards_value(&pattern.pat),
        _ => false,
    }
}

fn address_space_reclaim(expression: &Expr) -> Option<&syn::ExprMethodCall> {
    match expression {
        Expr::Group(group) => address_space_reclaim(&group.expr),
        Expr::Paren(parenthesized) => address_space_reclaim(&parenthesized.expr),
        Expr::MethodCall(call) if call.method == "reclaim" && call.args.len() == 1 => Some(call),
        _ => None,
    }
}

fn discarded_address_space_reclaims(source: &str) -> syn::Result<Vec<usize>> {
    let syntax = syn::parse_file(source)?;
    let mut discarded = DiscardedAddressSpaceReclaims::default();
    discarded.visit_file(&syntax);
    discarded.0.sort_unstable();
    discarded.0.dedup();
    Ok(discarded.0)
}

fn path_ends_with(path: &syn::Path, expected: &[&str]) -> bool {
    path.segments.len() >= expected.len()
        && path
            .segments
            .iter()
            .rev()
            .zip(expected.iter().rev())
            .all(|(segment, expected)| segment.ident == *expected)
}

fn expression_is_ident(expression: &Expr, expected: &str) -> bool {
    match expression {
        Expr::Group(group) => expression_is_ident(&group.expr, expected),
        Expr::Paren(parenthesized) => expression_is_ident(&parenthesized.expr, expected),
        Expr::Path(path) => {
            path.qself.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].ident == expected
        }
        _ => false,
    }
}
