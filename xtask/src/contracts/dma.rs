//! Source contracts for NVMe DMA-stop authority.

use std::path::PathBuf;

use syn::{Expr, ExprCall, ExprMethodCall, FnArg, ImplItem, Item, ItemFn, Type, visit::Visit};

mod entry;
mod isa;
mod nvme;
mod retirement;
mod vtd;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root")
        .to_path_buf()
}

fn source(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("reading {relative}: {error}"))
}

fn parse(relative: &str) -> syn::File {
    let source = source(relative);
    syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"))
}

fn parse_module(relative: &str) -> syn::File {
    super::module_syntax(&root().join(relative))
}

fn module_source(relative: &str) -> String {
    super::module_sources(&root().join(relative))
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
        })
        .collect()
}

fn path_ends(path: &syn::Path, names: &[&str]) -> bool {
    path.segments.len() >= names.len()
        && path
            .segments
            .iter()
            .rev()
            .zip(names.iter().rev())
            .all(|(segment, name)| segment.ident == name)
}

fn path_expression_is(expression: &Expr, names: &[&str]) -> bool {
    matches!(expression, Expr::Path(path) if path_ends(&path.path, names))
}

fn integer_literal_is(expression: &Expr, value: u64) -> bool {
    matches!(expression, Expr::Lit(literal)
        if matches!(&literal.lit, syn::Lit::Int(integer)
            if integer.base10_parse::<u64>().is_ok_and(|parsed| parsed == value)))
}

fn member_is(member: &syn::Member, name: &str) -> bool {
    matches!(member, syn::Member::Named(identifier) if identifier == name)
}

fn field_chain_is(expression: &Expr, root: &str, fields: &[&str]) -> bool {
    let Some((last, prefix)) = fields.split_last() else {
        return path_expression_is(expression, &[root]);
    };
    matches!(expression, Expr::Field(field)
        if member_is(&field.member, last) && field_chain_is(&field.base, root, prefix))
}

fn call_is<'ast>(expression: &'ast Expr, name: &str) -> Option<&'ast ExprCall> {
    let Expr::Call(call) = expression else {
        return None;
    };
    path_expression_is(&call.func, &[name]).then_some(call)
}

fn function<'ast>(syntax: &'ast syn::File, name: &str) -> &'ast ItemFn {
    syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == name => Some(function),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing function `{name}`"))
}

struct MethodFinder<'query, 'ast> {
    owner: &'query str,
    name: &'query str,
    inherent_only: bool,
    found: Vec<&'ast syn::ImplItemFn>,
}

impl<'ast> Visit<'ast> for MethodFinder<'_, 'ast> {
    fn visit_item_impl(&mut self, implementation: &'ast syn::ItemImpl) {
        if (!self.inherent_only || implementation.trait_.is_none())
            && matches!(&*implementation.self_ty, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == self.owner))
        {
            self.found
                .extend(implementation.items.iter().filter_map(|item| match item {
                    ImplItem::Fn(method) if method.sig.ident == self.name => Some(method),
                    _ => None,
                }));
        }
        syn::visit::visit_item_impl(self, implementation);
    }
}

fn find_method<'ast>(
    syntax: &'ast syn::File,
    owner: &str,
    name: &str,
    inherent_only: bool,
) -> Option<&'ast syn::ImplItemFn> {
    let mut methods = MethodFinder {
        owner,
        name,
        inherent_only,
        found: Vec::new(),
    };
    methods.visit_file(syntax);
    methods.found.into_iter().next()
}

fn method<'ast>(syntax: &'ast syn::File, owner: &str, name: &str) -> Option<&'ast syn::ImplItemFn> {
    find_method(syntax, owner, name, true)
}

fn implemented_method<'ast>(
    syntax: &'ast syn::File,
    owner: &str,
    name: &str,
) -> Option<&'ast syn::ImplItemFn> {
    find_method(syntax, owner, name, false)
}

fn exact_derives(attributes: &[syn::Attribute], expected: &[&str]) -> bool {
    let mut derives = Vec::new();
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("derive"))
    {
        let Ok(paths) = attribute.parse_args_with(
            syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
        ) else {
            return false;
        };
        derives.extend(paths.into_iter().filter_map(|path| {
            path.segments
                .last()
                .map(|segment| segment.ident.to_string())
        }));
    }
    derives.sort();
    let mut expected = expected
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    derives == expected
}

fn authority_alias_count(syntax: &syn::File, names: &[&str]) -> usize {
    fn renamed_authorities(tree: &syn::UseTree, names: &[&str], prefix: &mut Vec<String>) -> usize {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                let count = renamed_authorities(&path.tree, names, prefix);
                prefix.pop();
                count
            }
            syn::UseTree::Rename(rename) if names.iter().any(|name| rename.ident == *name) => {
                usize::from(
                    prefix.as_slice() != ["nvme"]
                        || rename.ident != "DmaState"
                        || rename.rename != "DmaFlightState",
                )
            }
            syn::UseTree::Group(group) => group
                .items
                .iter()
                .map(|tree| renamed_authorities(tree, names, prefix))
                .sum(),
            _ => 0,
        }
    }

    struct Aliases<'name> {
        names: &'name [&'name str],
        count: usize,
    }

    impl<'ast> Visit<'ast> for Aliases<'_> {
        fn visit_item_type(&mut self, alias: &'ast syn::ItemType) {
            if matches!(&*alias.ty, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment|
                    self.names.iter().any(|name| segment.ident == *name)))
            {
                self.count += 1;
            }
            syn::visit::visit_item_type(self, alias);
        }

        fn visit_item_use(&mut self, import: &'ast syn::ItemUse) {
            self.count += renamed_authorities(&import.tree, self.names, &mut Vec::new());
            syn::visit::visit_item_use(self, import);
        }
    }

    let mut aliases = Aliases { names, count: 0 };
    aliases.visit_file(syntax);
    aliases.count
}

#[derive(Default)]
struct ReferencedPaths {
    names: Vec<String>,
}

impl Visit<'_> for ReferencedPaths {
    fn visit_expr_path(&mut self, path: &syn::ExprPath) {
        if let Some(last) = path.path.segments.last() {
            self.names.push(last.ident.to_string());
        }
        syn::visit::visit_expr_path(self, path);
    }
}

fn by_value_named_input(argument: Option<&FnArg>, name: &str) -> bool {
    matches!(argument, Some(FnArg::Typed(argument))
        if matches!(&*argument.ty, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == name)))
}

#[test]
fn trait_methods_do_not_satisfy_dma_authority_contracts() {
    let syntax = syn::parse_file(
        "struct Boundary; trait Cosmetic { fn quiesce(&mut self); } \
         impl Cosmetic for Boundary { fn quiesce(&mut self) {} }",
    )
    .expect("valid trait-method witness");
    assert!(method(&syntax, "Boundary", "quiesce").is_none());
    assert!(implemented_method(&syntax, "Boundary", "quiesce").is_some());
}

#[test]
fn nested_inherent_methods_cannot_hide_from_dma_authority_contracts() {
    let syntax = syn::parse_file(
        "struct Boundary; const _: () = { \
         impl Boundary { fn quiesce(&mut self) {} } };",
    )
    .expect("valid nested inherent-method witness");
    assert!(method(&syntax, "Boundary", "quiesce").is_some());
}

#[derive(Default)]
struct CallNames(Vec<String>);

impl Visit<'_> for CallNames {
    fn visit_expr_call(&mut self, call: &ExprCall) {
        if let Expr::Path(path) = &*call.func
            && let Some(segment) = path.path.segments.last()
        {
            self.0.push(segment.ident.to_string());
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &ExprMethodCall) {
        self.0.push(call.method.to_string());
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn call_position(calls: &CallNames, name: &str) -> usize {
    calls
        .0
        .iter()
        .position(|call| call == name)
        .unwrap_or_else(|| panic!("missing `{name}` call from unit transition"))
}
