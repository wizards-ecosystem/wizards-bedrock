//! Source contracts for NVMe DMA-stop authority.

use std::path::PathBuf;

use syn::{
    Expr, ExprAssign, ExprCall, ExprMatch, ExprMethodCall, ExprStruct, Fields, FnArg, ImplItem,
    Item, ItemFn, ItemImpl, Pat, ReturnType, Stmt, Type, Visibility, visit::Visit,
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits below the workspace root")
        .to_path_buf()
}

fn parse(relative: &str) -> syn::File {
    let source = std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
    syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"))
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

fn dereferenced_path_is(expression: &Expr, names: &[&str]) -> bool {
    matches!(expression, Expr::Unary(unary)
        if matches!(unary.op, syn::UnOp::Deref(_)) && path_expression_is(&unary.expr, names))
}

fn call_is<'a>(expression: &'a Expr, name: &str) -> Option<&'a ExprCall> {
    let Expr::Call(call) = expression else {
        return None;
    };
    path_expression_is(&call.func, &[name]).then_some(call)
}

fn tried_call_is<'a>(statement: &'a Stmt, name: &str) -> Option<&'a ExprCall> {
    let Stmt::Expr(Expr::Try(tried), Some(_)) = statement else {
        return None;
    };
    call_is(&tried.expr, name)
}

fn tried_local_initializer<'a>(statement: &'a Stmt, name: &str) -> Option<&'a Expr> {
    let Stmt::Local(local) = statement else {
        return None;
    };
    if !matches!(&local.pat, Pat::Ident(binding) if binding.ident == name) {
        return None;
    }
    let Expr::Try(tried) = &*local.init.as_ref()?.expr else {
        return None;
    };
    Some(&tried.expr)
}

fn function<'a>(syntax: &'a syn::File, name: &str) -> &'a ItemFn {
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

fn find_method<'a>(
    syntax: &'a syn::File,
    owner: &str,
    name: &str,
    inherent_only: bool,
) -> Option<&'a syn::ImplItemFn> {
    let mut methods = MethodFinder {
        owner,
        name,
        inherent_only,
        found: Vec::new(),
    };
    methods.visit_file(syntax);
    core::iter::Iterator::next(&mut methods.found.into_iter())
}

fn method<'a>(syntax: &'a syn::File, owner: &str, name: &str) -> Option<&'a syn::ImplItemFn> {
    find_method(syntax, owner, name, true)
}

fn implemented_method<'a>(
    syntax: &'a syn::File,
    owner: &str,
    name: &str,
) -> Option<&'a syn::ImplItemFn> {
    find_method(syntax, owner, name, false)
}

struct Implementations<'ast, 'name> {
    owner: &'name str,
    items: Vec<&'ast ItemImpl>,
}

impl<'ast> Visit<'ast> for Implementations<'ast, '_> {
    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        if matches!(&*implementation.self_ty, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == self.owner))
        {
            self.items.push(implementation);
        }
        syn::visit::visit_item_impl(self, implementation);
    }
}

fn implementations<'a>(syntax: &'a syn::File, owner: &str) -> Vec<&'a ItemImpl> {
    let mut found = Implementations {
        owner,
        items: Vec::new(),
    };
    found.visit_file(syntax);
    found.items
}

fn exact_inherent_methods(syntax: &syn::File, owner: &str, expected: &[&str]) -> bool {
    let implementations = implementations(syntax, owner);
    if implementations.len() != 1 || implementations[0].trait_.is_some() {
        return false;
    }
    let mut methods = Vec::new();
    for item in &implementations[0].items {
        let ImplItem::Fn(method) = item else {
            return false;
        };
        methods.push(method.sig.ident.to_string());
    }
    methods.sort();
    let mut expected = expected
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    expected.sort();
    methods == expected
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
struct StopUses {
    constructors: usize,
    quiesce_calls: Vec<ExprMethodCall>,
    retain_calls: Vec<ExprMethodCall>,
    assignments: Vec<ExprAssign>,
    quiesced_values: usize,
    retained_values: usize,
    stopped_trait_impls: usize,
    disposition_fields: usize,
    dma_transfer_calls: usize,
    dma_assignments: usize,
    dma_fields: usize,
}

impl<'ast> Visit<'ast> for StopUses {
    fn visit_expr_struct(&mut self, expression: &'ast ExprStruct) {
        if path_ends(&expression.path, &["ControllerStopped"]) {
            self.constructors += 1;
        }
        syn::visit::visit_expr_struct(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast ExprMethodCall) {
        if expression.method == "quiesce" {
            self.quiesce_calls.push(expression.clone());
        }
        if expression.method == "retain_unquiesced" {
            self.retain_calls.push(expression.clone());
        }
        syn::visit::visit_expr_method_call(self, expression);
    }

    fn visit_expr_assign(&mut self, expression: &'ast ExprAssign) {
        if expression_mentions_dma(&expression.left) || expression_mentions_dma(&expression.right) {
            self.dma_assignments += 1;
        }
        self.assignments.push(expression.clone());
        syn::visit::visit_expr_assign(self, expression);
    }

    fn visit_expr_field(&mut self, expression: &'ast syn::ExprField) {
        if member_is(&expression.member, "disposition") {
            self.disposition_fields += 1;
        }
        if member_is(&expression.member, "dma") {
            self.dma_fields += 1;
        }
        syn::visit::visit_expr_field(self, expression);
    }

    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if matches!(&*expression.func, Expr::Path(path)
            if path.path.segments.last().is_some_and(|segment|
                matches!(segment.ident.to_string().as_str(), "swap" | "replace" | "take")))
            && expression.args.iter().any(expression_mentions_dma)
        {
            self.dma_transfer_calls += 1;
        }
        syn::visit::visit_expr_call(self, expression);
    }

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        if path_ends(&expression.path, &["DmaDisposition", "Quiesced"]) {
            self.quiesced_values += 1;
        }
        if path_ends(&expression.path, &["DmaDisposition", "Retained"]) {
            self.retained_values += 1;
        }
        syn::visit::visit_expr_path(self, expression);
    }

    fn visit_item_impl(&mut self, implementation: &'ast syn::ItemImpl) {
        if implementation.trait_.is_some()
            && matches!(&*implementation.self_ty, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "ControllerStopped"))
        {
            self.stopped_trait_impls += 1;
        }
        syn::visit::visit_item_impl(self, implementation);
    }
}

fn expression_mentions_dma(expression: &Expr) -> bool {
    struct DmaField {
        found: bool,
    }

    impl<'ast> Visit<'ast> for DmaField {
        fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
            if member_is(&field.member, "dma") {
                self.found = true;
            }
            syn::visit::visit_expr_field(self, field);
        }
    }

    let mut visitor = DmaField { found: false };
    visitor.visit_expr(expression);
    visitor.found
}

#[derive(Default)]
struct Matches {
    expressions: Vec<ExprMatch>,
}

impl<'ast> Visit<'ast> for Matches {
    fn visit_expr_match(&mut self, expression: &'ast ExprMatch) {
        self.expressions.push(expression.clone());
        syn::visit::visit_expr_match(self, expression);
    }
}

#[derive(Default)]
struct ControllerRetirementArms<'ast> {
    retired: usize,
    quarantined: Vec<&'ast syn::Arm>,
}

impl<'ast> Visit<'ast> for ControllerRetirementArms<'ast> {
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if matches!(&arm.pat, Pat::Path(path)
            if path_ends(&path.path, &["ControllerRetirement", "Retired"]))
        {
            self.retired += 1;
        }
        if matches!(&arm.pat, Pat::TupleStruct(tuple)
            if path_ends(&tuple.path, &["ControllerRetirement", "Quarantined"])
                && tuple.elems.len() == 1
                && matches!(tuple.elems.first(), Some(Pat::Ident(binding))
                    if binding.ident == "quarantine"))
        {
            self.quarantined.push(arm);
        }
        syn::visit::visit_arm(self, arm);
    }
}

fn controller_quarantine_arm_is_exact(arm: &syn::Arm) -> bool {
    let Expr::Block(body) = &*arm.body else {
        return false;
    };
    let keepers = body
        .block
        .stmts
        .iter()
        .filter(|statement| {
            matches!(statement,
            Stmt::Expr(Expr::Call(call), Some(_))
                if path_expression_is(&call.func,
                    &["nvme", "keep_controller_quarantine"])
                    && call.args.len() == 2
                    && path_expression_is(&call.args[0], &["quarantine"])
                    && path_expression_is(&call.args[1], &["device"]))
        })
        .count();
    let halted = matches!(body.block.stmts.last(),
        Some(Stmt::Expr(Expr::Return(returned), Some(_)))
            if returned.expr.as_deref().is_some_and(settles_halted));
    keepers == 1 && halted
}

fn settles_halted(expression: &Expr) -> bool {
    matches!(expression, Expr::Call(settled)
        if path_expression_is(&settled.func, &["CandidateStage", "Settled"])
            && settled.args.len() == 1
            && path_expression_is(&settled.args[0], &["StorageSelection", "Halted"]))
}

#[derive(Default)]
struct DeviceRetirementUses {
    initial_unmaps: usize,
    retry_unmaps: usize,
    retired: usize,
    retained_region: usize,
    retained_second_region: usize,
}

impl<'ast> Visit<'ast> for DeviceRetirementUses {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if path_expression_is(&call.func, &["unmap_device"]) && call.args.len() == 2 {
            if path_expression_is(&call.args[1], &["region"]) {
                self.initial_unmaps += 1;
            }
            if field_chain_is(&call.args[1], "first", &["region"]) {
                self.retry_unmaps += 1;
            }
        }
        if path_expression_is(&call.func, &["DeviceRetirement", "Retained"]) && call.args.len() == 1
        {
            if path_expression_is(&call.args[0], &["region"]) {
                self.retained_region += 1;
            }
            if field_chain_is(&call.args[0], "second", &["region"]) {
                self.retained_second_region += 1;
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path_ends(&path.path, &["DeviceRetirement", "Retired"]) {
            self.retired += 1;
        }
        syn::visit::visit_expr_path(self, path);
    }
}

fn routing_retention_block_is_exact(block: &syn::Block) -> bool {
    matches!(block.stmts.as_slice(), [warning, Stmt::Expr(Expr::Call(retain), Some(_)), Stmt::Expr(Expr::Return(returned), Some(_))]
        if matches!(warning, Stmt::Macro(statement)
            if path_ends(&statement.mac.path, &["warn"]))
            && path_expression_is(&retain.func, &["mem", "retain_routing"])
            && retain.args.len() == 1
            && path_expression_is(&retain.args[0], &["routing"])
            && returned.expr.as_deref().is_some_and(|expression|
                matches!(expression, Expr::Lit(literal)
                    if matches!(&literal.lit, syn::Lit::Bool(value) if !value.value))))
}

#[derive(Default)]
struct CalledFunctions {
    names: Vec<String>,
}

impl<'ast> Visit<'ast> for CalledFunctions {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(path) = &*call.func
            && let Some(last) = path.path.segments.last()
        {
            self.names.push(last.ident.to_string());
        }
        syn::visit::visit_expr_call(self, call);
    }
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

struct StageReferences<'name> {
    watched: &'name [String],
    enclosing: Vec<String>,
    found: Vec<(String, String)>,
}

impl StageReferences<'_> {
    fn record(&mut self, name: String) {
        if self.watched.contains(&name) {
            let caller = self.enclosing.last().cloned().unwrap_or_default();
            self.found.push((caller, name));
        }
    }

    fn record_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => self.record_tokens(group.stream()),
                proc_macro2::TokenTree::Ident(identifier) => self.record(identifier.to_string()),
                proc_macro2::TokenTree::Literal(_) | proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for StageReferences<'_> {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.enclosing.push(function.sig.ident.to_string());
        syn::visit::visit_item_fn(self, function);
        self.enclosing.pop();
    }

    fn visit_impl_item_fn(&mut self, method: &'ast syn::ImplItemFn) {
        self.enclosing.push(method.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, method);
        self.enclosing.pop();
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if let Some(last) = path.path.segments.last() {
            self.record(last.ident.to_string());
        }
        syn::visit::visit_expr_path(self, path);
    }

    fn visit_token_stream(&mut self, tokens: &'ast proc_macro2::TokenStream) {
        self.record_tokens(tokens.clone());
    }
}

#[derive(Default)]
struct WindowQuarantineArms<'ast> {
    found: Vec<&'ast syn::Arm>,
}

impl<'ast> Visit<'ast> for WindowQuarantineArms<'ast> {
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if matches!(&arm.pat, Pat::Path(path)
            if path_ends(&path.path, &["ControllerWindowAdmission", "Quarantined"]))
        {
            self.found.push(arm);
        }
        syn::visit::visit_arm(self, arm);
    }
}

fn window_quarantine_arm_is_exact(arm: &syn::Arm) -> bool {
    let Expr::Block(body) = &*arm.body else {
        return false;
    };
    matches!(body.block.stmts.as_slice(), [
        Stmt::Expr(Expr::Call(retain), Some(_)),
        Stmt::Expr(Expr::Return(returned), Some(_)),
    ] if path_expression_is(&retain.func, &["pci", "retain_failed_release"])
        && retain.args.len() == 1
        && path_expression_is(&retain.args[0], &["device"])
        && returned.expr.as_deref().is_some_and(settles_halted))
}

#[derive(Default)]
struct WindowAdmissionUses {
    release_admitted: usize,
    retain_authority: usize,
    retain_mapping: usize,
    retirement_quarantines: usize,
    retirement_to_window_quarantine: usize,
}

impl<'ast> Visit<'ast> for WindowAdmissionUses {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if path_expression_is(&call.func, &["platform_pci", "release_admitted_bar"])
            && call.args.len() == 1
            && matches!(&call.args[0], Expr::MethodCall(method)
                if method.method == "into_admitted"
                    && method.args.is_empty()
                    && path_expression_is(&method.receiver, &["failure"]))
        {
            self.release_admitted += 1;
        }
        if path_expression_is(&call.func, &["mem", "retain_pci_bar_authority"])
            && call.args.len() == 1
            && path_expression_is(&call.args[0], &["failure"])
        {
            self.retain_authority += 1;
        }
        if path_expression_is(&call.func, &["mem", "retain_pci_bar_mapping"])
            && call.args.len() == 1
            && path_expression_is(&call.args[0], &["mapping"])
        {
            self.retain_mapping += 1;
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if matches!(&arm.pat, Pat::Path(path)
            if path_ends(&path.path, &["PciBarRetirement", "Quarantined"]))
        {
            self.retirement_quarantines += 1;
            if path_expression_is(&arm.body, &["ControllerWindowAdmission", "Quarantined"]) {
                self.retirement_to_window_quarantine += 1;
            }
        }
        syn::visit::visit_arm(self, arm);
    }
}

fn fixed_retention_sink_is_exact(function: &ItemFn, storage: &str, owner: &str) -> bool {
    let Some(Stmt::Local(lock)) = function.block.stmts.first() else {
        return false;
    };
    let locked = matches!(&lock.pat, Pat::Ident(binding) if binding.ident == "retained")
        && matches!(&lock.init, Some(initializer)
            if matches!(&*initializer.expr, Expr::MethodCall(call)
                if call.method == "lock_leaf"
                    && call.args.is_empty()
                    && path_expression_is(&call.receiver, &[storage])));

    struct ExactStore<'name> {
        owner: &'name str,
        count: usize,
    }

    impl<'ast> Visit<'ast> for ExactStore<'_> {
        fn visit_expr_assign(&mut self, assign: &'ast ExprAssign) {
            if matches!(&*assign.left, Expr::Unary(dereference)
                if matches!(dereference.op, syn::UnOp::Deref(_))
                    && path_expression_is(&dereference.expr, &["slot"]))
                && matches!(&*assign.right, Expr::Call(some)
                    if path_expression_is(&some.func, &["Some"])
                        && some.args.len() == 1
                        && path_expression_is(&some.args[0], &[self.owner]))
            {
                self.count += 1;
            }
            syn::visit::visit_expr_assign(self, assign);
        }
    }

    let mut stores = ExactStore { owner, count: 0 };
    stores.visit_block(&function.block);
    locked && stores.count == 1
}

fn pci_bar_retention_wrapper_is_exact(
    function: &ItemFn,
    variant: &str,
    field: &str,
    owner: &str,
) -> bool {
    matches!(function.block.stmts.as_slice(), [Stmt::Expr(Expr::Call(retain), Some(_))]
        if path_expression_is(&retain.func, &["retain_pci_bar"])
            && retain.args.len() == 1
            && matches!(&retain.args[0], Expr::Struct(value)
                if path_ends(&value.path, &["PciBarQuarantine", variant])
                    && value.fields.len() == 1
                    && value.fields.iter().any(|value|
                        member_is(&value.member, field)
                            && path_expression_is(&value.expr, &[owner]))))
}

#[derive(Default)]
struct PanicMacros {
    count: usize,
}

impl<'ast> Visit<'ast> for PanicMacros {
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if path_ends(&invocation.path, &["panic"]) {
            self.count += 1;
        }
        syn::visit::visit_macro(self, invocation);
    }
}

fn variant_binding(pattern: &Pat, variant: &str, binding: Option<&str>) -> bool {
    let Pat::TupleStruct(tuple) = pattern else {
        return false;
    };
    if !path_ends(&tuple.path, &[variant]) || tuple.elems.len() != 1 {
        return false;
    }
    match (tuple.elems.first(), binding) {
        (Some(Pat::Ident(identifier)), Some(expected)) => identifier.ident == expected,
        (Some(Pat::Wild(_)), None) => true,
        _ => false,
    }
}

fn dma_call(expression: &Expr, method: &str, argument: Option<&str>) -> bool {
    let Expr::MethodCall(call) = expression else {
        return false;
    };
    matches!(&*call.receiver, Expr::Field(field)
        if path_expression_is(&field.base, &["self"])
            && member_is(&field.member, "dma"))
        && call.method == method
        && match argument {
            Some(name) => call.args.len() == 1 && path_expression_is(&call.args[0], &[name]),
            None => call.args.is_empty(),
        }
}

fn receiver_call(expression: &Expr, receiver: &str, method: &str) -> bool {
    matches!(expression, Expr::MethodCall(call)
        if path_expression_is(&call.receiver, &[receiver])
            && call.method == method
            && call.args.is_empty())
}

fn sole_expression(method: &syn::ImplItemFn) -> Option<&Expr> {
    match method.block.stmts.as_slice() {
        [Stmt::Expr(expression, None)] => Some(expression),
        _ => None,
    }
}

fn disposition_is(expression: &Expr, fields: &[&str], variant: &str) -> bool {
    matches!(expression, Expr::Binary(binary)
        if matches!(binary.op, syn::BinOp::Eq(_))
            && field_chain_is(&binary.left, "self", fields)
            && path_expression_is(&binary.right, &["DmaDisposition", variant]))
}

fn close_statement(statement: &Stmt, fields: &[&str]) -> bool {
    matches!(statement, Stmt::Expr(Expr::MethodCall(call), Some(_))
        if call.method == "close"
            && call.args.is_empty()
            && field_chain_is(&call.receiver, "self", fields))
}

fn disposition_assignment(statement: &Stmt, fields: &[&str], variant: &str) -> bool {
    matches!(statement, Stmt::Expr(Expr::Assign(assignment), Some(_))
        if field_chain_is(&assignment.left, "self", fields)
            && path_expression_is(&assignment.right, &["DmaDisposition", variant]))
}

fn disposition_transition_is_exact(
    method: &syn::ImplItemFn,
    disposition: &[&str],
    flight: &[&str],
    target: &str,
) -> bool {
    let Some(Expr::If(branch)) = sole_expression(method) else {
        return false;
    };
    branch.else_branch.is_none()
        && disposition_is(&branch.cond, disposition, "Active")
        && matches!(branch.then_branch.stmts.as_slice(), [close, assign]
            if close_statement(close, flight)
                && disposition_assignment(assign, disposition, target))
}

fn matches_disposition(expression: &Expr, variant: &str, negated: bool) -> bool {
    let expression = if negated {
        let Expr::Unary(unary) = expression else {
            return false;
        };
        if !matches!(unary.op, syn::UnOp::Not(_)) {
            return false;
        }
        &*unary.expr
    } else {
        expression
    };
    matches!(expression, Expr::Macro(invocation)
        if invocation.mac.path.is_ident("matches")
            && invocation.mac.tokens.to_string()
                == format!("& self . disposition , DmaDisposition :: {variant}"))
}

fn method_call_on(
    statement: &Stmt,
    receiver: impl FnOnce(&Expr) -> bool,
    method: &str,
    argument: Option<&str>,
) -> bool {
    matches!(statement, Stmt::Expr(Expr::MethodCall(call), Some(_))
    if receiver(&call.receiver)
        && call.method == method
        && match argument {
            Some(argument) => call.args.len() == 1
                && path_expression_is(&call.args[0], &[argument]),
            None => call.args.is_empty(),
        })
}

fn calls_stop_dma_none(statement: &Stmt) -> bool {
    matches!(statement, Stmt::Expr(Expr::MethodCall(call), Some(_))
        if path_expression_is(&call.receiver, &["self"])
            && call.method == "stop_dma"
            && call.args.len() == 1
            && path_expression_is(&call.args[0], &["None"]))
}

fn local_option_branch(
    statement: &Stmt,
    binding: &str,
    source: impl FnOnce(&Expr) -> bool,
    action: &str,
) -> bool {
    matches!(statement, Stmt::Expr(Expr::If(branch), None)
        if branch.else_branch.is_none()
            && matches!(&*branch.cond, Expr::Let(condition)
                if variant_binding(&condition.pat, "Some", Some(binding))
                    && source(&condition.expr))
            && matches!(branch.then_branch.stmts.as_slice(), [body]
                if method_call_on(body,
                    |receiver| path_expression_is(receiver, &[binding]), action, None)))
}

fn stop_dma_is_exact(method: &syn::ImplItemFn) -> bool {
    let [
        Stmt::Expr(Expr::If(stop), None),
        Stmt::Expr(Expr::If(retain), None),
    ] = method.block.stmts.as_slice()
    else {
        return false;
    };
    let stop_guard = matches!(&*stop.cond, Expr::Unary(not)
        if matches!(not.op, syn::UnOp::Not(_))
            && dma_call(&not.expr, "is_settled", None));
    let stop_decision = matches!(stop.then_branch.stmts.as_slice(),
        [Stmt::Expr(Expr::Match(decision), None)]
            if path_expression_is(&decision.expr, &["stop_controller"])
                || matches!(&*decision.expr, Expr::Call(call)
                    if path_expression_is(&call.func, &["stop_controller"])
                        && call.args.len() == 1
                        && path_expression_is(&call.args[0], &["self"])));
    let retain_guard = dma_call(&retain.cond, "must_retain", None);
    let retain_body = matches!(retain.then_branch.stmts.as_slice(), [admin, queue, data]
        if method_call_on(admin,
            |receiver| field_chain_is(receiver, "self", &["admin"]), "retain", None)
            && local_option_branch(queue, "queue",
                |source| matches!(source, Expr::MethodCall(call)
                    if call.method == "as_mut" && call.args.is_empty()
                        && field_chain_is(&call.receiver, "self", &["io"])),
                "retain")
            && local_option_branch(data, "segment",
                |source| path_expression_is(source, &["data"]), "retain"));
    stop.else_branch.is_none()
        && retain.else_branch.is_none()
        && stop_guard
        && stop_decision
        && retain_guard
        && retain_body
}

fn queue_retention_is_exact(method: &syn::ImplItemFn) -> bool {
    matches!(method.block.stmts.as_slice(), [submission, completion]
        if method_call_on(submission,
            |receiver| field_chain_is(receiver, "self", &["submission"]), "retain", None)
            && method_call_on(completion,
                |receiver| field_chain_is(receiver, "self", &["completion"]), "retain", None))
}

fn controller_quarantine_is(expression: &Expr) -> bool {
    let expression = match expression {
        Expr::Block(block) => match block.block.stmts.as_slice() {
            [Stmt::Expr(expression, None)] => expression,
            _ => return false,
        },
        expression => expression,
    };
    let Some(quarantined) = call_is(expression, "quarantine_controller") else {
        return false;
    };
    quarantined.args.len() == 1 && path_expression_is(&quarantined.args[0], &["self"])
}

fn returns_controller_quarantine(block: &syn::Block) -> bool {
    matches!(block.stmts.as_slice(), [Stmt::Expr(Expr::Return(returned), Some(_))]
        if returned.expr.as_deref().is_some_and(controller_quarantine_is))
        || matches!(block.stmts.as_slice(), [Stmt::Expr(Expr::Return(returned), None)]
            if returned.expr.as_deref().is_some_and(controller_quarantine_is))
}

fn ends_with_controller_quarantine(block: &syn::Block) -> bool {
    matches!(block.stmts.last(), Some(Stmt::Expr(Expr::Return(returned), _))
        if returned.expr.as_deref().is_some_and(controller_quarantine_is))
}

fn controller_retirement_is_exact(method: &syn::ImplItemFn) -> bool {
    let [
        Stmt::Expr(Expr::If(fence), None),
        stop,
        Stmt::Expr(Expr::If(refusal), None),
        Stmt::Expr(Expr::If(queues), None),
        Stmt::Local(registers),
        Stmt::Expr(Expr::Match(unmap), None),
    ] = method.block.stmts.as_slice()
    else {
        return false;
    };
    let fenced = matches!(&*fence.cond, Expr::Let(condition)
        if matches!(&*condition.pat, Pat::TupleStruct(pattern)
            if path_ends(&pattern.path, &["Err"]))
            && matches!(&*condition.expr, Expr::MethodCall(call)
                if call.method == "disable_bus_master_for_retirement"
                    && call.args.is_empty()
                    && path_expression_is(&call.receiver, &["device"])))
        && ends_with_controller_quarantine(&fence.then_branch)
        && fence.else_branch.is_none();
    let stopped = calls_stop_dma_none(stop);
    let refused = matches!(&*refusal.cond, Expr::Unary(not)
        if matches!(not.op, syn::UnOp::Not(_))
            && dma_call(&not.expr, "is_quiesced", None))
        && returns_controller_quarantine(&refusal.then_branch)
        && refusal.else_branch.is_none();
    let queues_retired = matches!(&*queues.cond, Expr::Unary(not)
        if matches!(not.op, syn::UnOp::Not(_))
            && matches!(&*not.expr, Expr::MethodCall(call)
                if call.method == "unmap_queues"
                    && call.args.is_empty()
                    && path_expression_is(&call.receiver, &["self"])))
        && ends_with_controller_quarantine(&queues.then_branch)
        && queues.else_branch.is_none();
    let extracted = variant_binding(&registers.pat, "Some", Some("registers"))
        && matches!(&registers.init, Some(initializer)
            if matches!(&*initializer.expr, Expr::MethodCall(call)
                if call.method == "take"
                    && call.args.is_empty()
                    && field_chain_is(&call.receiver, "self", &["registers"]))
                && matches!(&initializer.diverge, Some((_, otherwise))
                    if matches!(&**otherwise, Expr::Block(otherwise)
                        if returns_controller_quarantine(&otherwise.block))));
    let unmapped = matches!(&*unmap.expr, Expr::Call(call)
        if path_expression_is(&call.func, &["mem", "unmap_mapped_pci_bar_exact"])
            && call.args.len() == 2
            && path_expression_is(&call.args[0], &["context"])
            && path_expression_is(&call.args[1], &["registers"]));
    let exact_outcomes = matches!(unmap.arms.as_slice(), [retired, retained, quarantined]
        if matches!(&retired.pat, Pat::Path(path)
            if path_ends(&path.path, &["MappedPciBarRetirement", "Retired"]))
        && path_expression_is(&retired.body, &["ControllerRetirement", "Retired"])
        && variant_binding(&retained.pat, "Retained", Some("registers"))
        && matches!(&*retained.body, Expr::Block(body)
            if matches!(body.block.stmts.as_slice(), [restore, Stmt::Expr(result, None)]
                if matches!(restore, Stmt::Expr(Expr::Assign(assignment), Some(_))
                    if field_chain_is(&assignment.left, "self", &["registers"])
                        && matches!(&*assignment.right, Expr::Call(some)
                            if path_expression_is(&some.func, &["Some"])
                                && some.args.len() == 1
                                && path_expression_is(&some.args[0], &["registers"])))
                    && controller_quarantine_is(result)))
        && matches!(&quarantined.pat, Pat::Path(path)
            if path_ends(&path.path, &["MappedPciBarRetirement", "Quarantined"]))
        && controller_quarantine_is(&quarantined.body));
    fenced && stopped && refused && queues_retired && extracted && unmapped && exact_outcomes
}

fn drop_stops_dma_exactly(syntax: &syn::File) -> bool {
    let drops = implementations(syntax, "Controller")
        .into_iter()
        .filter(|implementation| {
            implementation
                .trait_
                .as_ref()
                .is_some_and(|(_, path, _)| path_ends(path, &["Drop"]))
        })
        .collect::<Vec<_>>();
    if drops.len() != 1 || drops[0].items.len() != 1 {
        return false;
    }
    let ImplItem::Fn(drop) = &drops[0].items[0] else {
        return false;
    };
    drop.sig.ident == "drop"
        && matches!(drop.sig.inputs.first(), Some(FnArg::Receiver(receiver))
            if receiver.reference.is_some() && receiver.mutability.is_some())
        && drop.sig.inputs.len() == 1
        && matches!(drop.block.stmts.as_slice(), [statement]
            if calls_stop_dma_none(statement))
}

fn readiness_poll_is_exact(function: &ItemFn) -> bool {
    let Some(Stmt::Expr(Expr::While(poll), None)) = function.block.stmts.get(3) else {
        return false;
    };
    let [
        Stmt::Local(bits),
        Stmt::Local(observed),
        Stmt::Expr(Expr::If(success), None),
        Stmt::Expr(Expr::If(fatal), None),
        spin,
    ] = poll.body.stmts.as_slice()
    else {
        return false;
    };
    let poll_is_bounded = matches!(&*poll.cond, Expr::MethodCall(permit)
        if permit.method == "permit"
            && path_expression_is(&permit.receiver, &["limit"])
            && permit.args.len() == 1
            && matches!(permit.args.first(), Some(Expr::Call(clock))
                if path_expression_is(&clock.func, &["clock_ns"])
                    && clock.args.is_empty()));
    let bits_are_live = matches!(&bits.pat, Pat::Ident(binding) if binding.ident == "bits")
        && matches!(&bits.init, Some(initializer)
            if initializer.diverge.is_none()
                && matches!(&*initializer.expr, Expr::Try(tried)
                    if matches!(&*tried.expr, Expr::Call(read)
                        if path_expression_is(&read.func, &["read"])
                            && read.args.len() == 2
                            && path_expression_is(&read.args[0], &["registers"])
                            && path_expression_is(&read.args[1], &["register", "CSTS"]))));
    let observed_is_ready_bit = matches!(&observed.pat, Pat::Ident(binding)
        if binding.ident == "observed")
        && matches!(&observed.init, Some(initializer)
            if initializer.diverge.is_none()
                && matches!(&*initializer.expr, Expr::If(branch)
                    if matches!(&*branch.cond, Expr::Binary(nonzero)
                        if matches!(nonzero.op, syn::BinOp::Ne(_))
                            && matches!(&*nonzero.left, Expr::Binary(mask)
                                if matches!(mask.op, syn::BinOp::BitAnd(_))
                                    && path_expression_is(&mask.left, &["bits"])
                                    && path_expression_is(&mask.right, &["status", "READY"]))
                            && matches!(&*nonzero.right, Expr::Lit(literal)
                                if matches!(&literal.lit, syn::Lit::Int(value)
                                    if matches!(value.base10_parse::<u32>(), Ok(0)))))
                        && matches!(branch.then_branch.stmts.as_slice(),
                            [Stmt::Expr(ready, None)]
                                if path_expression_is(ready, &["Readiness", "Ready"]))
                        && matches!(branch.else_branch.as_ref(), Some((_, otherwise))
                            if matches!(&**otherwise, Expr::Block(otherwise)
                                if matches!(otherwise.block.stmts.as_slice(),
                                    [Stmt::Expr(not_ready, None)]
                                        if path_expression_is(not_ready,
                                            &["Readiness", "NotReady"]))))));
    let success_is_exact = matches!(&*success.cond, Expr::Binary(equal)
        if matches!(equal.op, syn::BinOp::Eq(_))
            && path_expression_is(&equal.left, &["observed"])
            && path_expression_is(&equal.right, &["wanted"]))
        && matches!(success.then_branch.stmts.as_slice(),
            [Stmt::Expr(Expr::Return(returned), Some(_))]
                if matches!(returned.expr.as_deref(), Some(Expr::Call(ok))
                    if path_expression_is(&ok.func, &["Ok"])
                        && matches!(ok.args.first(), Some(Expr::Tuple(unit))
                            if unit.elems.is_empty())))
        && success.else_branch.is_none();
    let fatal_is_enable_only = matches!(&*fatal.cond, Expr::Binary(and)
        if matches!(and.op, syn::BinOp::And(_))
            && matches!(&*and.left, Expr::Binary(equal)
                if matches!(equal.op, syn::BinOp::Eq(_))
                    && path_expression_is(&equal.left, &["wanted"])
                    && path_expression_is(&equal.right, &["Readiness", "Ready"]))
            && matches!(&*and.right, Expr::Binary(nonzero)
                if matches!(nonzero.op, syn::BinOp::Ne(_))
                    && matches!(&*nonzero.left, Expr::Binary(mask)
                        if matches!(mask.op, syn::BinOp::BitAnd(_))
                            && path_expression_is(&mask.left, &["bits"])
                            && path_expression_is(&mask.right, &["status", "FATAL"]))
                    && matches!(&*nonzero.right, Expr::Lit(literal)
                        if matches!(&literal.lit, syn::Lit::Int(value)
                            if matches!(value.base10_parse::<u32>(), Ok(0))))))
        && matches!(fatal.then_branch.stmts.as_slice(),
            [Stmt::Expr(Expr::Return(returned), Some(_))]
                if matches!(returned.expr.as_deref(), Some(Expr::Call(error))
                    if path_expression_is(&error.func, &["Err"])
                        && matches!(error.args.first(), Some(reason)
                            if path_expression_is(reason,
                                &["BringUpError", "NotReady"]))))
        && fatal.else_branch.is_none();
    let spin_is_exact = matches!(spin, Stmt::Expr(Expr::Call(call), Some(_))
        if matches!(&*call.func, Expr::Path(path)
            if path_ends(&path.path, &["core", "hint", "spin_loop"]))
            && call.args.is_empty());
    let terminal_timeout = matches!(function.block.stmts.last(),
        Some(Stmt::Expr(Expr::Call(error), None))
            if path_expression_is(&error.func, &["Err"])
                && matches!(error.args.first(), Some(reason)
                    if path_expression_is(reason, &["BringUpError", "NotReady"])));
    poll_is_bounded
        && bits_are_live
        && observed_is_ready_bit
        && success_is_exact
        && fatal_is_enable_only
        && spin_is_exact
        && terminal_timeout
}

#[derive(Default)]
struct BusMasterUses {
    authority_mints: usize,
    bar_zero_guards: usize,
    wrong_bar_refusals: usize,
    cc_reads: usize,
    csts_reads: usize,
    classifications: usize,
    mapped_admissions: usize,
    exact_mapped_admissions: usize,
    hardware_admissions: usize,
    observations: usize,
    device_admissions: usize,
}

impl<'ast> Visit<'ast> for BusMasterUses {
    fn visit_expr_struct(&mut self, value: &'ast ExprStruct) {
        if path_ends(&value.path, &["NvmeBusMasterAuthority"]) {
            self.authority_mints += 1;
        }
        syn::visit::visit_expr_struct(self, value);
    }

    fn visit_expr_if(&mut self, value: &'ast syn::ExprIf) {
        if matches!(&*value.cond, Expr::Binary(binary)
            if matches!(binary.op, syn::BinOp::Ne(_))
                && matches!(&*binary.left, Expr::MethodCall(call)
                    if call.method == "index"
                        && field_chain_is(&call.receiver, "self", &["authority"]))
                && matches!(&*binary.right, Expr::Lit(literal)
                    if matches!(&literal.lit, syn::Lit::Int(number)
                        if matches!(number.base10_parse::<u32>(), Ok(0)))))
        {
            self.bar_zero_guards += 1;
        }
        syn::visit::visit_expr_if(self, value);
    }

    fn visit_expr_path(&mut self, value: &'ast syn::ExprPath) {
        if path_ends(&value.path, &["NvmeObservationError", "WrongBar"]) {
            self.wrong_bar_refusals += 1;
        }
        syn::visit::visit_expr_path(self, value);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if call.method == "read_u32" && path_expression_is(&call.receiver, &["self"]) {
            match call.args.first() {
                Some(argument) if path_expression_is(argument, &["register", "CC"]) => {
                    self.cc_reads += 1;
                }
                Some(argument) if path_expression_is(argument, &["register", "CSTS"]) => {
                    self.csts_reads += 1;
                }
                _ => {}
            }
        }
        if call.method == "observe_nvme_disabled" {
            self.observations += 1;
        }
        if call.method == "admit_bus_master"
            && path_expression_is(&call.receiver, &["device"])
            && call.args.len() == 2
            && path_expression_is(&call.args[0], &["authority"])
            && path_expression_is(&call.args[1], &["domain"])
        {
            self.device_admissions += 1;
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if path_expression_is(&call.func, &["DisabledRegisters", "from_bits"])
            && call.args.len() == 2
            && path_expression_is(&call.args[0], &["cc"])
            && path_expression_is(&call.args[1], &["csts"])
        {
            self.classifications += 1;
        }
        if path_expression_is(&call.func, &["pci_config", "admit_mapped_bus_master"]) {
            self.mapped_admissions += 1;
            if call.args.len() == 5
                && matches!(&call.args[1], Expr::Reference(reference)
                    if field_chain_is(&reference.expr, "bar", &["authority"]))
                && matches!(&call.args[2], Expr::MethodCall(function)
                    if function.method == "function"
                        && function.args.is_empty()
                        && path_expression_is(&function.receiver, &["domain"]))
            {
                self.exact_mapped_admissions += 1;
            }
        }
        if path_expression_is(&call.func, &["pci", "admit_bus_master"])
            && call.args.len() == 3
            && path_expression_is(&call.args[0], &["authority"])
            && path_expression_is(&call.args[1], &["domain"])
            && path_expression_is(&call.args[2], &["state"])
        {
            self.hardware_admissions += 1;
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn by_value_named_input(argument: Option<&FnArg>, name: &str) -> bool {
    matches!(argument, Some(FnArg::Typed(argument))
        if matches!(&*argument.ty, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == name)))
}

#[test]
fn nvme_bus_master_authority_is_fresh_linear_and_mapping_bound() {
    let wost = parse("wost/src/arch/x86_64/pci.rs");
    let authority = wost
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "NvmeBusMasterAuthority" => Some(item),
            _ => None,
        })
        .expect("missing NVMe bus-master authority");
    assert!(matches!(authority.vis, Visibility::Public(_)));
    assert_eq!(authority.generics.lifetimes().count(), 1);
    assert!(exact_derives(&authority.attrs, &["Debug"]));
    let Fields::Named(fields) = &authority.fields else {
        panic!("NVMe bus-master authority must name its exact evidence");
    };
    assert_eq!(fields.named.len(), 2);
    let bar = fields
        .named
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|name| name == "bar"))
        .expect("authority must retain its mapped BAR");
    assert!(matches!(bar.vis, Visibility::Inherited));
    assert!(matches!(&bar.ty, Type::Reference(reference)
        if reference.mutability.is_none()
            && matches!(&*reference.elem, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "MappedBar"))));
    let observed = fields
        .named
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|name| name == "_observed"))
        .expect("authority must retain its disabled-register observation");
    assert!(matches!(observed.vis, Visibility::Inherited));
    assert!(matches!(&observed.ty, Type::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == "DisabledRegisters")));

    let observe =
        method(&wost, "MappedBar", "observe_nvme_disabled").expect("mapped BAR observation door");
    assert!(
        matches!(observe.sig.inputs.first(), Some(FnArg::Receiver(receiver))
        if receiver.reference.is_some() && receiver.mutability.is_none())
    );
    let mut observation = BusMasterUses::default();
    observation.visit_block(&observe.block);
    assert_eq!(observation.bar_zero_guards, 1);
    assert_eq!(observation.wrong_bar_refusals, 1);
    assert_eq!(observation.cc_reads, 1);
    assert_eq!(observation.csts_reads, 1);
    assert_eq!(observation.classifications, 1);
    assert_eq!(observation.authority_mints, 1);

    let mut all_wost = BusMasterUses::default();
    all_wost.visit_file(&wost);
    assert_eq!(all_wost.authority_mints, 1, "authority gained another mint");

    let admission = function(&wost, "admit_bus_master");
    assert!(by_value_named_input(
        admission.sig.inputs.first(),
        "NvmeBusMasterAuthority"
    ));
    assert!(
        by_value_named_input(admission.sig.inputs.iter().nth(1), "DmaDomain"),
        "the hardware door admits bus mastering without a DMA domain proof"
    );
    let mut admitted = BusMasterUses::default();
    admitted.visit_block(&admission.block);
    assert_eq!(admitted.mapped_admissions, 1);
    assert_eq!(admitted.exact_mapped_admissions, 1);

    let kernel_pci = parse("kernel/src/pci.rs");
    assert!(method(&kernel_pci, "Device", "enable_bus_master").is_none());
    let device_admission = method(&kernel_pci, "Device", "admit_bus_master")
        .expect("device bus-master admission boundary");
    assert!(by_value_named_input(
        device_admission.sig.inputs.iter().nth(1),
        "NvmeBusMasterAuthority"
    ));
    assert!(
        by_value_named_input(device_admission.sig.inputs.iter().nth(2), "DmaDomain"),
        "the device door admits bus mastering without a DMA domain proof"
    );
    let mut device = BusMasterUses::default();
    device.visit_block(&device_admission.block);
    assert_eq!(device.hardware_admissions, 1);

    let kernel_nvme = parse("kernel/src/nvme.rs");
    assert!(
        !kernel_nvme
            .items
            .iter()
            .any(|item| matches!(item, Item::Struct(item) if item.ident == "DisabledObservation"))
    );
    let controller_admission = method(&kernel_nvme, "DisabledController", "admit")
        .expect("disabled-controller admission steps");
    let mut controller = BusMasterUses::default();
    controller.visit_block(&controller_admission.block);
    assert_eq!(controller.observations, 1);
    assert_eq!(controller.device_admissions, 1);
    let mut all_nvme = BusMasterUses::default();
    all_nvme.visit_file(&kernel_nvme);
    assert_eq!(
        all_nvme.observations, 1,
        "a second observation door appeared"
    );
    assert_eq!(
        all_nvme.device_admissions, 1,
        "a second admission door appeared"
    );
}

#[test]
fn controller_stop_evidence_has_one_hardware_mint() {
    let syntax = parse("kernel/src/nvme.rs");
    let evidence = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "ControllerStopped" => Some(item),
            _ => None,
        })
        .expect("missing ControllerStopped evidence");
    assert!(matches!(evidence.vis, Visibility::Inherited));
    assert_eq!(evidence.generics.lifetimes().count(), 1);
    let Fields::Named(fields) = &evidence.fields else {
        panic!("ControllerStopped must name its register borrow");
    };
    assert_eq!(fields.named.len(), 3);
    let registers = fields
        .named
        .iter()
        .find(|field| {
            field
                .ident
                .as_ref()
                .is_some_and(|name| name == "_registers")
        })
        .expect("stop evidence must borrow its observed register file");
    assert!(matches!(registers.vis, Visibility::Inherited));
    assert!(matches!(&registers.ty, Type::Reference(reference)
        if reference.mutability.is_none()
            && matches!(&*reference.elem, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "MappedBar"))));
    let observed = fields
        .named
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|name| name == "_observed"))
        .expect("stop evidence must retain its disabled-register observation");
    assert!(matches!(observed.vis, Visibility::Inherited));
    assert!(matches!(&observed.ty, Type::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == "DisabledRegisters")));
    let dma = fields
        .named
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|name| name == "dma"))
        .expect("stop evidence must borrow the exact DMA state it retires");
    assert!(matches!(dma.vis, Visibility::Inherited));
    assert!(matches!(&dma.ty, Type::Reference(reference)
        if reference.mutability.is_some()
            && matches!(&*reference.elem, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "DmaState"))));
    assert!(exact_derives(&evidence.attrs, &["Debug"]));
    assert!(exact_inherent_methods(
        &syntax,
        "ControllerStopped",
        &["quiesce"]
    ));
    assert_eq!(
        authority_alias_count(&syntax, &["ControllerStopped"]),
        0,
        "stop evidence gained a constructor alias"
    );

    let stop = function(&syntax, "stop_controller");
    assert_eq!(stop.sig.inputs.len(), 1);
    assert!(
        matches!(stop.sig.inputs.first(), Some(FnArg::Typed(argument))
        if matches!(&*argument.pat, Pat::Ident(name) if name.ident == "controller")
            && matches!(&*argument.ty, Type::Reference(reference)
                if reference.mutability.is_some()
                    && matches!(&*reference.elem, Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment| segment.ident == "Controller"))))
    );
    assert!(matches!(stop.sig.output, ReturnType::Type(_, ref ty)
        if matches!(&**ty, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == "Result"))));
    assert_eq!(
        stop.block.stmts.len(),
        8,
        "the evidence mint must stay auditable"
    );

    let Stmt::Local(parts) = &stop.block.stmts[0] else {
        panic!("the stop boundary must split one Controller borrow");
    };
    let Pat::Struct(parts) = &parts.pat else {
        panic!("the stop boundary must split one Controller borrow");
    };
    assert!(path_ends(&parts.path, &["Controller"]));
    assert!(parts.rest.is_some());
    assert_eq!(parts.fields.len(), 3);
    for name in ["registers", "capabilities", "dma"] {
        assert!(
            parts
                .fields
                .iter()
                .any(|field| member_is(&field.member, name)
                    && matches!(&*field.pat, Pat::Ident(binding) if binding.ident == name))
        );
    }

    let write = tried_call_is(&stop.block.stmts[2], "write").expect("first disable write");
    assert_eq!(write.args.len(), 3);
    assert!(path_expression_is(&write.args[0], &["registers"]));
    assert!(path_expression_is(&write.args[1], &["register", "CC"]));
    assert!(matches!(&write.args[2], Expr::Lit(literal)
        if matches!(&literal.lit, syn::Lit::Int(value)
            if matches!(value.base10_parse::<u32>(), Ok(0)))));

    let wait = tried_call_is(&stop.block.stmts[3], "wait_for_ready")
        .expect("disable must be followed by a readiness observation");
    assert_eq!(wait.args.len(), 3);
    assert!(path_expression_is(&wait.args[0], &["registers"]));
    assert!(dereferenced_path_is(&wait.args[1], &["capabilities"]));
    assert!(path_expression_is(
        &wait.args[2],
        &["Readiness", "NotReady"]
    ));

    for (index, binding, register_name) in [(4, "cc", "CC"), (5, "csts", "CSTS")] {
        let read = tried_local_initializer(&stop.block.stmts[index], binding)
            .and_then(|expression| call_is(expression, "read"))
            .unwrap_or_else(|| panic!("stop must freshly read {register_name}"));
        assert_eq!(read.args.len(), 2);
        assert!(path_expression_is(&read.args[0], &["registers"]));
        assert!(path_expression_is(
            &read.args[1],
            &["register", register_name]
        ));
    }

    let classify = tried_local_initializer(&stop.block.stmts[6], "observed")
        .expect("stop must classify its fresh CC/CSTS pair");
    let Expr::MethodCall(map_error) = classify else {
        panic!("stop classification must map an active pair to failure");
    };
    assert_eq!(map_error.method, "map_err");
    assert_eq!(map_error.args.len(), 1);
    assert!(
        matches!(map_error.args.first(), Some(Expr::Closure(closure))
        if matches!(&*closure.body, Expr::Path(path)
            if path_ends(&path.path, &["BringUpError", "NotReady"])))
    );
    let Some(from_bits) = call_is(&map_error.receiver, "from_bits") else {
        panic!("stop classification must use DisabledRegisters::from_bits");
    };
    assert!(path_expression_is(
        &from_bits.func,
        &["DisabledRegisters", "from_bits"]
    ));
    assert_eq!(from_bits.args.len(), 2);
    assert!(path_expression_is(&from_bits.args[0], &["cc"]));
    assert!(path_expression_is(&from_bits.args[1], &["csts"]));

    let Stmt::Expr(last, None) = &stop.block.stmts[7] else {
        panic!("the final expression must return stop evidence");
    };
    let ok = call_is(last, "Ok").expect("stop evidence must be returned through Result");
    assert_eq!(ok.args.len(), 1);
    assert!(matches!(ok.args.first(), Some(Expr::Struct(value))
        if path_ends(&value.path, &["ControllerStopped"])
            && value.fields.len() == 3
            && value.fields.iter().any(|field|
                member_is(&field.member, "_registers")
                    && path_expression_is(&field.expr, &["registers"]))
            && value.fields.iter().any(|field|
                member_is(&field.member, "_observed")
                    && path_expression_is(&field.expr, &["observed"]))
            && value.fields.iter().any(|field|
                member_is(&field.member, "dma")
                    && path_expression_is(&field.expr, &["dma"]))));

    let mut uses = StopUses::default();
    uses.visit_file(&syntax);
    assert_eq!(
        uses.stopped_trait_impls, 0,
        "stop evidence gained an explicit trait implementation"
    );
    assert_eq!(uses.constructors, 1, "stop evidence gained another mint");
}

#[test]
fn vtd_mapping_enforces_effective_coverage_before_the_walk() {
    let syntax = parse("wost/src/arch/x86_64/vtd.rs");
    let public = method(&syntax, "Vtd", "map_segment").expect("missing VT-d mapping seam");
    let Some(FnArg::Typed(segment)) = public.sig.inputs.iter().nth(1) else {
        panic!("the public VT-d mapping seam has no segment owner");
    };
    assert!(matches!(&*segment.ty, Type::Reference(reference)
        if reference.mutability.is_some()
            && matches!(&*reference.elem, Type::Path(path)
                if path_ends(&path.path, &["DmaSegment"]))));

    struct RetainsSegment(usize);
    impl<'ast> Visit<'ast> for RetainsSegment {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "retain" && path_expression_is(&call.receiver, &["segment"]) {
                self.0 += 1;
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    for name in ["map_segment", "unmap_segment"] {
        let boundary = method(&syntax, "Vtd", name).expect("missing owned VT-d segment boundary");
        let Some(FnArg::Typed(segment)) = boundary.sig.inputs.iter().nth(1) else {
            panic!("{name} has no DMA-segment owner");
        };
        assert!(matches!(&*segment.ty, Type::Reference(reference)
            if reference.mutability.is_some()
                && matches!(&*reference.elem, Type::Path(path)
                    if path_ends(&path.path, &["DmaSegment"]))));
        let mut retains = RetainsSegment(0);
        retains.visit_block(&boundary.block);
        assert_eq!(retains.0, 1, "{name} does not retain uncertain ownership");
    }

    let mapping = method(&syntax, "Vtd", "map_range").expect("missing private VT-d range walk");
    let Some(Stmt::Expr(Expr::If(gate), _)) = mapping.block.stmts.get(1) else {
        panic!("the mapping seam has no leading coverage refusal");
    };
    assert!(gate.else_branch.is_none());
    let Expr::Unary(refusal) = &*gate.cond else {
        panic!("coverage must refuse on the negative result");
    };
    assert!(matches!(refusal.op, syn::UnOp::Not(_)));
    let Expr::MethodCall(coverage) = &*refusal.expr else {
        panic!("coverage refusal does not call the checked policy");
    };
    assert_eq!(coverage.method, "covers_on");
    assert!(field_chain_is(&coverage.receiver, "self", &["width"]));
    assert_eq!(coverage.args.len(), 2);
    assert!(coverage.args.first().is_some_and(|argument| field_chain_is(
        argument,
        "self",
        &["capability"]
    )));
    assert!(
        coverage
            .args
            .last()
            .is_some_and(|argument| path_expression_is(argument, &["last"]))
    );
    let [Stmt::Expr(Expr::Return(returned), _)] = gate.then_branch.stmts.as_slice() else {
        panic!("coverage refusal must return before the table walk");
    };
    let refused = returned
        .expr
        .as_deref()
        .and_then(|expression| call_is(expression, "Err"))
        .and_then(|error| error.args.first())
        .and_then(|expression| call_is(expression, "Refused"))
        .and_then(|refused| refused.args.first())
        .expect("coverage refusal must return a typed clean failure");
    assert!(path_expression_is(
        refused,
        &["VtdError", "OutsideAddressWidth"]
    ));
}

#[test]
fn a_partial_queue_mapping_rolls_back_the_submission_queue() {
    let syntax = parse("kernel/src/nvme.rs");
    let mapping = method(&syntax, "QueuePair", "map_into").expect("queue mapping boundary");

    struct SubmissionRollbacks(usize);
    impl<'ast> Visit<'ast> for SubmissionRollbacks {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "unmap_segment"
                && path_expression_is(&call.receiver, &["isolation"])
                && matches!(call.args.first(), Some(Expr::Reference(reference))
                    if reference.mutability.is_some()
                        && field_chain_is(&reference.expr, "self", &["submission"]))
            {
                self.0 += 1;
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }

    let mut rollbacks = SubmissionRollbacks(0);
    rollbacks.visit_block(&mapping.block);
    assert_eq!(
        rollbacks.0, 1,
        "a completion-queue map refusal must roll back the submission queue"
    );
}

#[test]
fn rejected_controller_retirement_fences_and_unmaps_every_queue() {
    let syntax = parse("kernel/src/nvme.rs");
    let retirement = method(&syntax, "Controller", "retire").expect("controller retirement");

    #[derive(Default)]
    struct RetirementOrder(Vec<&'static str>);
    impl<'ast> Visit<'ast> for RetirementOrder {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            let named = if call.method == "disable_bus_master_for_retirement"
                && path_expression_is(&call.receiver, &["device"])
            {
                Some("fence")
            } else if call.method == "stop_dma" && path_expression_is(&call.receiver, &["self"]) {
                Some("stop")
            } else if call.method == "unmap_queues" && path_expression_is(&call.receiver, &["self"])
            {
                Some("queues")
            } else {
                None
            };
            if let Some(named) = named {
                self.0.push(named);
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if path_expression_is(&call.func, &["mem", "unmap_mapped_pci_bar_exact"]) {
                self.0.push("bar");
            }
            syn::visit::visit_expr_call(self, call);
        }
    }

    let mut order = RetirementOrder::default();
    order.visit_block(&retirement.block);
    assert_eq!(order.0.first(), Some(&"fence"));
    assert_eq!(order.0.iter().filter(|step| **step == "stop").count(), 2);
    let queues = order
        .0
        .iter()
        .position(|step| *step == "queues")
        .expect("retirement does not unmap its queues");
    let bar = order
        .0
        .iter()
        .position(|step| *step == "bar")
        .expect("retirement does not retire its BAR");
    assert!(
        order.0[..queues]
            .iter()
            .all(|step| matches!(*step, "fence" | "stop"))
            && queues < bar,
        "DMA fencing, stopping, queue invalidation and BAR retirement are out of order: {:?}",
        order.0
    );

    let unmap = method(&syntax, "QueuePair", "unmap_from").expect("queue unmap boundary");
    struct QueueUnmaps {
        submission: usize,
        completion: usize,
    }
    impl<'ast> Visit<'ast> for QueueUnmaps {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "unmap_segment" && path_expression_is(&call.receiver, &["isolation"])
            {
                if matches!(call.args.first(), Some(Expr::Reference(reference))
                    if reference.mutability.is_some()
                        && field_chain_is(&reference.expr, "self", &["submission"]))
                {
                    self.submission += 1;
                }
                if matches!(call.args.first(), Some(Expr::Reference(reference))
                    if reference.mutability.is_some()
                        && field_chain_is(&reference.expr, "self", &["completion"]))
                {
                    self.completion += 1;
                }
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut unmaps = QueueUnmaps {
        submission: 0,
        completion: 0,
    };
    unmaps.visit_block(&unmap.block);
    assert_eq!((unmaps.submission, unmaps.completion), (1, 1));

    let queue_drop = implementations(&syntax, "QueuePair")
        .into_iter()
        .find(|implementation| {
            implementation
                .trait_
                .as_ref()
                .is_some_and(|(_, path, _)| path_ends(path, &["Drop"]))
        })
        .expect("mapped queues have no drop backstop");
    let drop = queue_drop
        .items
        .iter()
        .find_map(|item| match item {
            ImplItem::Fn(method) if method.sig.ident == "drop" => Some(method),
            _ => None,
        })
        .expect("QueuePair's Drop implementation has no drop method");
    struct RetainMapped(usize);
    impl<'ast> Visit<'ast> for RetainMapped {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "retain_mapped" && path_expression_is(&call.receiver, &["self"]) {
                self.0 += 1;
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut retains = RetainMapped(0);
    retains.visit_block(&drop.block);
    assert_eq!(retains.0, 1, "mapped queue frames can recycle from Drop");
}

#[test]
fn uncertain_io_queue_mapping_stays_reachable_for_retirement() {
    let syntax = parse("kernel/src/nvme.rs");
    let creation =
        method(&syntax, "Controller", "create_io_queues").expect("I/O queue creation boundary");

    #[derive(Default)]
    struct QueueOwnershipOrder(Vec<&'static str>);
    impl<'ast> Visit<'ast> for QueueOwnershipOrder {
        fn visit_expr_assign(&mut self, assignment: &'ast syn::ExprAssign) {
            if field_chain_is(&assignment.left, "self", &["io"]) {
                self.0.push("install");
            }
            syn::visit::visit_expr_assign(self, assignment);
        }

        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "map_into" && path_expression_is(&call.receiver, &["pair"]) {
                self.0.push("map");
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }

    let mut order = QueueOwnershipOrder::default();
    order.visit_block(&creation.block);
    assert_eq!(
        order.0,
        ["install", "map"],
        "a fallible queue map must not precede installation in the controller owner"
    );
}

#[test]
fn queue_mapping_state_conserves_every_reachable_segment() {
    let syntax = parse("kernel/src/nvme.rs");
    let mapping = method(&syntax, "QueuePair", "map_into").expect("queue map boundary");
    let unmapping = method(&syntax, "QueuePair", "unmap_from").expect("queue unmap boundary");
    let retention =
        method(&syntax, "QueuePair", "retain_mapped").expect("mapped queue retention boundary");

    #[derive(Default)]
    struct FlagAssignments {
        submission_true: usize,
        completion_true: usize,
        submission_false: usize,
        completion_false: usize,
    }
    impl<'ast> Visit<'ast> for FlagAssignments {
        fn visit_expr_assign(&mut self, assignment: &'ast ExprAssign) {
            let value = match &*assignment.right {
                Expr::Lit(literal) => match &literal.lit {
                    syn::Lit::Bool(value) => Some(value.value),
                    _ => None,
                },
                _ => None,
            };
            let counter = if field_chain_is(&assignment.left, "self", &["submission_mapped"]) {
                value.map(|value| {
                    if value {
                        &mut self.submission_true
                    } else {
                        &mut self.submission_false
                    }
                })
            } else if field_chain_is(&assignment.left, "self", &["completion_mapped"]) {
                value.map(|value| {
                    if value {
                        &mut self.completion_true
                    } else {
                        &mut self.completion_false
                    }
                })
            } else {
                None
            };
            if let Some(counter) = counter {
                *counter += 1;
            }
            syn::visit::visit_expr_assign(self, assignment);
        }
    }

    let mut mapped = FlagAssignments::default();
    mapped.visit_block(&mapping.block);
    assert_eq!(
        (mapped.submission_true, mapped.completion_true),
        (2, 2),
        "successful and uncertain maps must both conservatively mark each segment"
    );
    assert_eq!((mapped.submission_false, mapped.completion_false), (1, 0));

    let mut unmapped = FlagAssignments::default();
    unmapped.visit_block(&unmapping.block);
    assert_eq!(
        (unmapped.submission_false, unmapped.completion_false),
        (1, 1),
        "only successful invalidation may clear either mapped flag"
    );
    assert_eq!((unmapped.submission_true, unmapped.completion_true), (0, 0));

    #[derive(Default)]
    struct Retention {
        submission_guard: usize,
        completion_guard: usize,
        submission_retain: usize,
        completion_retain: usize,
    }
    impl<'ast> Visit<'ast> for Retention {
        fn visit_expr_if(&mut self, branch: &'ast syn::ExprIf) {
            if field_chain_is(&branch.cond, "self", &["submission_mapped"]) {
                self.submission_guard += 1;
            }
            if field_chain_is(&branch.cond, "self", &["completion_mapped"]) {
                self.completion_guard += 1;
            }
            syn::visit::visit_expr_if(self, branch);
        }

        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "retain" {
                if field_chain_is(&call.receiver, "self", &["submission"]) {
                    self.submission_retain += 1;
                }
                if field_chain_is(&call.receiver, "self", &["completion"]) {
                    self.completion_retain += 1;
                }
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut retained = Retention::default();
    retained.visit_block(&retention.block);
    assert_eq!(
        (
            retained.submission_guard,
            retained.completion_guard,
            retained.submission_retain,
            retained.completion_retain,
        ),
        (1, 1, 1, 1),
        "every possibly mapped segment must be retained behind its own flag"
    );
}

#[test]
fn device_fault_reporting_is_bounded_and_follows_retirement() {
    let vtd = parse("wost/src/arch/x86_64/vtd.rs");
    let drain = method(&vtd, "Vtd", "drain_faults").expect("bounded VT-d fault drain");
    let Some(Stmt::Expr(Expr::While(drain_loop), None)) = drain.block.stmts.get(1) else {
        panic!("the VT-d fault drain has no explicit loop");
    };
    let Expr::Binary(condition) = &*drain_loop.cond else {
        panic!("the VT-d fault drain has no inventory comparison");
    };
    assert!(matches!(condition.op, syn::BinOp::And(_)));
    let Expr::Binary(bound) = &*condition.left else {
        panic!("the VT-d fault drain does not test its budget first");
    };
    assert!(
        matches!(bound.op, syn::BinOp::Lt(_)),
        "the live fault drain can inspect more records than the inventory"
    );
    assert!(
        path_expression_is(&bound.left, &["drained"]),
        "the VT-d fault bound does not count drained records"
    );
    let Expr::Call(limit) = &*bound.right else {
        panic!("the VT-d fault bound does not derive from its register inventory");
    };
    assert!(path_expression_is(&limit.func, &["usize", "from"]));
    assert!(matches!(limit.args.first(), Some(argument)
        if field_chain_is(argument, "self", &["fault_registers"])));

    struct DrainIncrement(usize);
    impl<'ast> Visit<'ast> for DrainIncrement {
        fn visit_expr_binary(&mut self, binary: &'ast syn::ExprBinary) {
            if matches!(binary.op, syn::BinOp::AddAssign(_))
                && path_expression_is(&binary.left, &["drained"])
                && matches!(&*binary.right, Expr::Lit(literal)
                    if matches!(&literal.lit, syn::Lit::Int(one) if one.base10_digits() == "1"))
            {
                self.0 += 1;
            }
            syn::visit::visit_expr_binary(self, binary);
        }
    }
    let mut increment = DrainIncrement(0);
    increment.visit_block(&drain_loop.body);
    assert_eq!(
        increment.0, 1,
        "the bounded VT-d drain does not spend exactly one unit per record"
    );

    let iommu = parse("kernel/src/iommu.rs");
    let report = function(&iommu, "report_faults");
    struct FaultReporter {
        drains: usize,
        takes: usize,
        loops: usize,
    }
    impl<'ast> Visit<'ast> for FaultReporter {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "drain_faults" {
                self.drains += 1;
            }
            if call.method == "take_fault" {
                self.takes += 1;
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_expr_while(&mut self, loop_: &'ast syn::ExprWhile) {
            self.loops += 1;
            syn::visit::visit_expr_while(self, loop_);
        }
    }
    let mut reporter = FaultReporter {
        drains: 0,
        takes: 0,
        loops: 0,
    };
    reporter.visit_block(&report.block);
    assert_eq!((reporter.drains, reporter.takes, reporter.loops), (1, 0, 0));

    let main = parse("kernel/src/main.rs");
    let start = function(&main, "start_candidate");
    struct FailureOrder(Vec<&'static str>);
    impl<'ast> Visit<'ast> for FailureOrder {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "retire" && path_expression_is(&call.receiver, &["controller"]) {
                self.0.push("retire");
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if path_expression_is(&call.func, &["iommu", "report_faults"]) {
                self.0.push("report");
            }
            syn::visit::visit_expr_call(self, call);
        }
    }
    let mut failure = FailureOrder(Vec::new());
    failure.visit_block(&start.block);
    let report = failure
        .0
        .iter()
        .position(|step| *step == "report")
        .expect("read failures never report VT-d faults");
    let last_retirement = failure
        .0
        .iter()
        .rposition(|step| *step == "retire")
        .expect("read failures do not retire their controller");
    assert!(
        last_retirement < report,
        "device-controlled reporting runs before the DMA fence: {:?}",
        failure.0
    );
}

#[test]
fn dma_disposition_cannot_move_between_controllers() {
    let syntax = parse("kernel/src/nvme.rs");
    let disposition = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Enum(item) if item.ident == "DmaDisposition" => Some(item),
            _ => None,
        })
        .expect("missing private DMA disposition");
    assert!(matches!(disposition.vis, Visibility::Inherited));
    assert!(exact_derives(
        &disposition.attrs,
        &["Debug", "Eq", "PartialEq"]
    ));
    assert!(implementations(&syntax, "DmaDisposition").is_empty());
    assert_eq!(
        authority_alias_count(&syntax, &["DmaDisposition", "DmaState"]),
        0,
        "DMA retirement state gained a transfer alias"
    );
    assert!(exact_inherent_methods(
        &syntax,
        "DmaState",
        &[
            "cancel_unpublished",
            "complete",
            "idle",
            "is_in_flight",
            "is_quiesced",
            "is_settled",
            "must_retain",
            "publish_with",
            "reserve",
            "retain_unquiesced"
        ]
    ));

    let mut uses = StopUses::default();
    uses.visit_file(&syntax);
    assert_eq!(
        uses.assignments
            .iter()
            .filter(|assignment| matches!(&*assignment.left, Expr::Field(field)
                if member_is(&field.member, "disposition")))
            .count(),
        2,
        "DMA disposition gained another assignment path"
    );
    assert_eq!(
        uses.disposition_fields, 8,
        "DMA disposition gained an unaudited read or transfer"
    );
    assert_eq!(
        uses.dma_transfer_calls, 0,
        "DMA state gained a swap/replace transfer path"
    );
    assert_eq!(
        uses.dma_assignments, 1,
        "DMA state gained a cross-controller assignment path"
    );
    assert_eq!(
        uses.dma_fields, 14,
        "DMA state gained an unaudited access or transfer alias"
    );

    let settled = method(&syntax, "DmaState", "is_settled").expect("settlement observer");
    assert!(matches!(sole_expression(settled), Some(expression)
        if matches_disposition(expression, "Active", true)));
    let quiesced = method(&syntax, "DmaState", "is_quiesced").expect("quiescence observer");
    assert!(matches!(sole_expression(quiesced), Some(expression)
        if matches_disposition(expression, "Quiesced", false)));
    let retained = method(&syntax, "DmaState", "must_retain").expect("retention observer");
    assert!(matches!(sole_expression(retained), Some(expression)
        if matches_disposition(expression, "Retained", false)));
}

#[test]
fn proven_stop_consumes_evidence_before_recycling() {
    let kernel = parse("kernel/src/nvme.rs");
    assert!(method(&kernel, "DmaState", "quiesce").is_none());
    let quiesce = method(&kernel, "ControllerStopped", "quiesce")
        .expect("stop evidence must own the quiescence transition");
    assert_eq!(quiesce.sig.inputs.len(), 1);
    assert!(
        matches!(quiesce.sig.inputs.first(), Some(FnArg::Receiver(receiver))
        if receiver.reference.is_none())
    );
    assert!(disposition_transition_is_exact(
        quiesce,
        &["dma", "disposition"],
        &["dma", "flight"],
        "Quiesced"
    ));

    let mut transition = StopUses::default();
    transition.visit_block(&quiesce.block);
    assert_eq!(transition.quiesced_values, 1);
    assert_eq!(transition.retained_values, 0);
    assert_eq!(
        transition
            .assignments
            .iter()
            .filter(
                |assignment| field_chain_is(&assignment.left, "self", &["dma", "disposition"])
                    && path_expression_is(&assignment.right, &["DmaDisposition", "Quiesced"])
            )
            .count(),
        1
    );
    assert!(transition.quiesce_calls.is_empty());
    assert!(transition.retain_calls.is_empty());
    assert!(
        quiesce
            .block
            .stmts
            .iter()
            .any(|statement| matches!(statement,
        Stmt::Expr(Expr::If(branch), None)
            if branch.then_branch.stmts.iter().any(|statement| matches!(statement,
                Stmt::Expr(Expr::MethodCall(call), Some(_))
                    if call.method == "close"
                        && call.args.is_empty()
                        && field_chain_is(&call.receiver, "self", &["dma", "flight"])))))
    );

    let mut uses = StopUses::default();
    uses.visit_file(&kernel);
    assert_eq!(
        uses.quiesce_calls.len(),
        1,
        "DMA gained another quiescence path"
    );
    assert_eq!(uses.quiesced_values, 1, "quiescence gained a proof bypass");
    let call = &uses.quiesce_calls[0];
    assert!(path_expression_is(&call.receiver, &["stopped"]));
    assert!(call.args.is_empty());

    let stop_dma = method(&kernel, "Controller", "stop_dma").expect("controller stop boundary");
    let mut matches = Matches::default();
    matches.visit_block(&stop_dma.block);
    assert_eq!(
        matches.expressions.len(),
        1,
        "stop decision must stay explicit"
    );
    let decision = &matches.expressions[0];
    assert_eq!(decision.arms.len(), 2);
    assert!(variant_binding(
        &decision.arms[0].pat,
        "Ok",
        Some("stopped")
    ));
    assert!(receiver_call(&decision.arms[0].body, "stopped", "quiesce"));
}

#[test]
fn unproven_stop_retains_every_reachable_run() {
    let kernel = parse("kernel/src/nvme.rs");
    let retain = method(&kernel, "DmaState", "retain_unquiesced")
        .expect("conservative retention transition");
    assert!(disposition_transition_is_exact(
        retain,
        &["disposition"],
        &["flight"],
        "Retained"
    ));
    let mut transition = StopUses::default();
    transition.visit_block(&retain.block);
    assert_eq!(transition.quiesced_values, 0);
    assert_eq!(transition.retained_values, 1);

    let mut uses = StopUses::default();
    uses.visit_file(&kernel);
    assert_eq!(
        uses.retain_calls.len(),
        1,
        "DMA gained another failed-stop transition"
    );
    assert_eq!(
        uses.retained_values, 1,
        "retention gained another constructor"
    );

    let stop_dma = method(&kernel, "Controller", "stop_dma").expect("controller stop boundary");
    let mut matches = Matches::default();
    matches.visit_block(&stop_dma.block);
    assert_eq!(
        matches.expressions.len(),
        1,
        "stop decision must stay explicit"
    );
    let decision = &matches.expressions[0];
    assert_eq!(decision.arms.len(), 2);
    assert!(variant_binding(&decision.arms[1].pat, "Err", None));
    assert!(dma_call(&decision.arms[1].body, "retain_unquiesced", None));

    let pure = parse("libs/nvme/src/lib.rs");
    for forbidden in ["quiesce", "is_quiesced", "must_retain", "retain_unquiesced"] {
        assert!(
            implemented_method(&pure, "DmaState", forbidden).is_none(),
            "pure DmaState recovered hardware authority through `{forbidden}`"
        );
    }
    assert!(method(&pure, "DmaState", "close").is_some());
}

#[test]
fn controller_retirement_returns_every_cleanup_owner() {
    let kernel = parse("kernel/src/nvme.rs");
    let retirement = kernel
        .items
        .iter()
        .find_map(|item| match item {
            Item::Enum(item) if item.ident == "ControllerRetirement" => Some(item),
            _ => None,
        })
        .expect("missing typed controller retirement");
    assert!(matches!(retirement.vis, Visibility::Restricted(_)));
    assert!(exact_derives(&retirement.attrs, &["Debug"]));
    assert_eq!(retirement.variants.len(), 2);
    assert!(
        retirement
            .variants
            .iter()
            .any(|variant| variant.ident == "Retired" && matches!(variant.fields, Fields::Unit))
    );
    assert!(
        retirement
            .variants
            .iter()
            .any(|variant| variant.ident == "Quarantined"
                && matches!(&variant.fields, Fields::Unnamed(fields)
                if fields.unnamed.len() == 1
                    && matches!(&fields.unnamed[0].ty, Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment|
                            segment.ident == "ControllerQuarantine"))))
    );

    let quarantine = kernel
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "ControllerQuarantine" => Some(item),
            _ => None,
        })
        .expect("missing controller quarantine owner");
    assert!(matches!(quarantine.vis, Visibility::Restricted(_)));
    assert!(exact_derives(&quarantine.attrs, &["Debug"]));
    assert!(matches!(&quarantine.fields, Fields::Named(fields)
        if fields.named.len() == 1
            && fields.named.iter().any(|field|
                field.ident.as_ref().is_some_and(|name| name == "_private")
                    && matches!(&field.ty, Type::Tuple(tuple) if tuple.elems.is_empty()))));
    let retained = kernel
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "RetainedController" => Some(item),
            _ => None,
        })
        .expect("missing controller quarantine storage");
    assert!(matches!(retained.vis, Visibility::Inherited));
    assert!(matches!(&retained.fields, Fields::Named(fields)
        if fields.named.len() == 2
            && fields.named.iter().any(|field|
                field.ident.as_ref().is_some_and(|name| name == "_controller")
                    && matches!(&field.ty, Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment|
                            segment.ident == "Controller")))
            && fields.named.iter().any(|field|
                field.ident.as_ref().is_some_and(|name| name == "device")
                    && matches!(&field.ty, Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment|
                            segment.ident == "Option")))));

    let failure = kernel
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "StartFailure" => Some(item),
            _ => None,
        })
        .expect("missing start failure");
    assert!(exact_derives(&failure.attrs, &["Debug"]));
    assert!(matches!(&failure.fields, Fields::Named(fields)
        if fields.named.len() == 2
            && fields.named.iter().any(|field|
                field.ident.as_ref().is_some_and(|name| name == "reason")
                    && matches!(&field.ty, Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment|
                            segment.ident == "BringUpError")))
            && fields.named.iter().any(|field|
                field.ident.as_ref().is_some_and(|name| name == "retirement")
                    && matches!(&field.ty, Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment|
                            segment.ident == "ControllerRetirement")))));
    assert!(exact_inherent_methods(
        &kernel,
        "StartFailure",
        &["into_retirement", "reason"]
    ));

    let retire = method(&kernel, "Controller", "retire").expect("controller retirement boundary");
    assert!(matches!(&retire.sig.output, ReturnType::Type(_, ty)
        if matches!(&**ty, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment|
                segment.ident == "ControllerRetirement"))));
    assert!(controller_retirement_is_exact(retire));

    let keep = function(&kernel, "keep_controller_quarantine");
    assert_eq!(keep.sig.inputs.len(), 2);
    assert!(by_value_named_input(
        keep.sig.inputs.first(),
        "ControllerQuarantine"
    ));
    assert!(by_value_named_input(
        keep.sig.inputs.iter().nth(1),
        "Device"
    ));
    assert!(
        matches!(keep.block.stmts.as_slice(), [Stmt::Local(consumed), Stmt::Local(locked), Stmt::Local(slot), assertion, Stmt::Expr(Expr::Assign(store), Some(_))]
        if matches!(&consumed.pat, Pat::Struct(pattern)
            if path_ends(&pattern.path, &["ControllerQuarantine"])
                && pattern.fields.len() == 1
                && member_is(&pattern.fields[0].member, "_private"))
            && matches!(&consumed.init, Some(initializer)
                if path_expression_is(&initializer.expr, &["quarantine"]))
            && matches!(&locked.pat, Pat::Ident(binding) if binding.ident == "retained")
            && matches!(&locked.init, Some(initializer)
                if matches!(&*initializer.expr, Expr::MethodCall(call)
                    if call.method == "lock"
                        && call.args.is_empty()
                        && path_expression_is(&call.receiver, &["CONTROLLER_QUARANTINE"])))
            && variant_binding(&slot.pat, "Some", Some("retained"))
            && matches!(assertion, Stmt::Macro(statement)
                if path_ends(&statement.mac.path, &["assert"]))
            && field_chain_is(&store.left, "retained", &["device"])
            && matches!(&*store.right, Expr::Call(some)
                if path_expression_is(&some.func, &["Some"])
                    && some.args.len() == 1
                    && path_expression_is(&some.args[0], &["device"])))
    );

    let quarantine_controller = function(&kernel, "quarantine_controller");
    assert!(by_value_named_input(
        quarantine_controller.sig.inputs.first(),
        "Controller"
    ));
    assert!(
        matches!(&quarantine_controller.sig.output, ReturnType::Type(_, ty)
        if matches!(&**ty, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment|
                segment.ident == "ControllerRetirement")))
    );
    assert!(
        matches!(quarantine_controller.block.stmts.as_slice(), [Stmt::Local(locked), assertion, Stmt::Expr(Expr::Assign(store), Some(_)), Stmt::Expr(result, None)]
        if matches!(&locked.pat, Pat::Ident(binding) if binding.ident == "retained")
            && matches!(&locked.init, Some(initializer)
                if matches!(&*initializer.expr, Expr::MethodCall(call)
                    if call.method == "lock"
                        && call.args.is_empty()
                        && path_expression_is(&call.receiver, &["CONTROLLER_QUARANTINE"])))
            && matches!(assertion, Stmt::Macro(statement)
                if path_ends(&statement.mac.path, &["assert"]))
            && matches!(&*store.left, Expr::Unary(dereference)
                if matches!(dereference.op, syn::UnOp::Deref(_))
                    && path_expression_is(&dereference.expr, &["retained"]))
            && matches!(&*store.right, Expr::Call(some)
                if path_expression_is(&some.func, &["Some"])
                    && some.args.len() == 1
                    && matches!(&some.args[0], Expr::Struct(value)
                        if path_ends(&value.path, &["RetainedController"])
                            && value.fields.iter().any(|field|
                                member_is(&field.member, "_controller")
                                    && path_expression_is(&field.expr, &["controller"]))
                            && value.fields.iter().any(|field|
                                member_is(&field.member, "device")
                                    && path_expression_is(&field.expr, &["None"]))))
            && matches!(call_is(result, "Quarantined"), Some(call)
                if path_expression_is(&call.func, &["ControllerRetirement", "Quarantined"])
                    && call.args.len() == 1
                    && matches!(&call.args[0], Expr::Struct(value)
                        if path_ends(&value.path, &["ControllerQuarantine"])
                            && value.fields.len() == 1
                            && value.fields.iter().any(|field|
                                member_is(&field.member, "_private")
                                    && matches!(&field.expr, Expr::Tuple(tuple)
                                        if tuple.elems.is_empty())))))
    );
}

#[test]
fn mapped_bar_cleanup_quarantine_cannot_be_reported_retired() {
    let kernel = parse("kernel/src/nvme.rs");
    let retire = function(&kernel, "retire_disabled");
    let [Stmt::Expr(Expr::Match(outcome), None)] = retire.block.stmts.as_slice() else {
        panic!("disabled preparation retirement must classify the mapped-BAR outcome");
    };
    assert_eq!(outcome.arms.len(), 3);
    let quarantined = &outcome.arms[2];
    assert!(matches!(&quarantined.pat, Pat::Path(path)
        if path_ends(&path.path, &["MappedPciBarRetirement", "Quarantined"])));
    let Some(retained) = call_is(&quarantined.body, "memory_quarantine") else {
        panic!("memory-owned BAR cleanup must remain a preparation quarantine");
    };
    assert!(path_expression_is(
        &retained.func,
        &["PrepareFailure", "memory_quarantine"]
    ));
    assert!(retained.args.len() == 1 && path_expression_is(&retained.args[0], &["reason"]));

    let constructor = method(&kernel, "PrepareFailure", "memory_quarantine")
        .expect("memory quarantine constructor");
    assert!(matches!(sole_expression(constructor), Some(expression)
        if matches!(call_is(expression, "Quarantined"), Some(call)
            if path_expression_is(&call.func, &["Self", "Quarantined"])
                && call.args.len() == 1
                && matches!(&call.args[0], Expr::Struct(value)
                    if path_ends(&value.path, &["PrepareQuarantine"])
                        && value.fields.iter().any(|field|
                            member_is(&field.member, "_registers")
                                && path_expression_is(&field.expr,
                                    &["QuarantinedRegisters", "MemoryQuarantine"]))))));
}

#[test]
fn every_failed_controller_retirement_keeps_its_controller_and_pci_lease() {
    let main = parse("kernel/src/main.rs");
    let mut arms = ControllerRetirementArms::default();
    arms.visit_file(&main);
    assert_eq!(arms.retired, 4);
    assert_eq!(arms.quarantined.len(), 4);
    assert!(
        arms.quarantined
            .iter()
            .all(|arm| controller_quarantine_arm_is_exact(arm))
    );

    let retire = function(&main, "retire_candidate");
    let Some(Stmt::Expr(Expr::If(fence), None)) = retire.block.stmts.first() else {
        panic!("candidate retirement must begin with its DMA fence");
    };
    assert!(matches!(&*fence.cond, Expr::Let(condition)
        if matches!(&*condition.expr, Expr::MethodCall(call)
            if call.method == "disable_bus_master_for_retirement"
                && call.args.is_empty()
                && path_expression_is(&call.receiver, &["device"]))));
    let Some(first) = retire.block.stmts.iter().find_map(|statement| {
        let Stmt::Expr(Expr::If(first), None) = statement else {
            return None;
        };
        matches!(&*first.cond, Expr::Let(condition)
            if matches!(&*condition.expr, Expr::MethodCall(into_retained)
                if into_retained.method == "into_retained"
                    && matches!(&*into_retained.receiver, Expr::MethodCall(release)
                        if release.method == "release"
                            && path_expression_is(&release.receiver, &["device"]))))
        .then_some(first)
    }) else {
        panic!("candidate retirement must release its fenced PCI lease");
    };
    let Some(Stmt::Expr(Expr::If(second), None)) = first.then_branch.stmts.get(1) else {
        panic!("candidate retirement must retain the owner after its second refusal");
    };
    assert!(matches!(&*second.cond, Expr::Let(condition)
        if matches!(&*condition.expr, Expr::MethodCall(into_retained)
            if into_retained.method == "into_retained"
                && into_retained.args.is_empty()
                && matches!(&*into_retained.receiver, Expr::MethodCall(release)
                    if release.method == "release"
                        && path_expression_is(&release.receiver, &["device"])
                        && release.args.len() == 1
                        && path_expression_is(&release.args[0], &["context"])))));
    let sinks = second
        .then_branch
        .stmts
        .iter()
        .filter(|statement| {
            matches!(statement,
            Stmt::Expr(Expr::Call(call), Some(_))
                if path_expression_is(&call.func, &["pci", "retain_failed_release"])
                    && call.args.len() == 1
                    && path_expression_is(&call.args[0], &["device"]))
        })
        .count();
    assert_eq!(sinks, 1);
    assert!(matches!(second.then_branch.stmts.last(),
        Some(Stmt::Expr(Expr::Return(returned), Some(_)))
            if returned.expr.as_deref().is_some_and(|expression|
                path_expression_is(expression, &["StorageSelection", "Halted"]))));
}

#[test]
fn only_the_candidate_sequencer_calls_its_stages() {
    let main = parse("kernel/src/main.rs");
    let mut calls = CalledFunctions::default();
    calls.visit_block(&function(&main, "try_controller").block);
    let stages: Vec<String> = main
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(stage)
                if calls.names.contains(&stage.sig.ident.to_string())
                    && matches!(&stage.sig.output, ReturnType::Type(_, returned)
                        if matches!(&**returned, Type::Path(path)
                            if path.path.is_ident("CandidateStage"))) =>
            {
                Some(stage.sig.ident.to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        stages.len(),
        3,
        "try_controller must sequence its owner-moving claim, admit and start stages: {stages:?}"
    );

    let mut watched = stages;
    watched.push("isolate_candidate".to_owned());
    watched.push("retire_candidate".to_owned());
    let mut references = StageReferences {
        watched: &watched,
        enclosing: Vec::new(),
        found: Vec::new(),
    };
    references.visit_file(&main);
    for (caller, stage) in &references.found {
        assert_eq!(
            caller, "try_controller",
            "{caller} calls {stage}, so two candidate frames share the boot stack (D-359)"
        );
    }
    for stage in &watched {
        assert_eq!(
            references
                .found
                .iter()
                .filter(|(_, named)| named == stage)
                .count(),
            1,
            "try_controller must call {stage} exactly once"
        );
    }
}

#[test]
fn device_unmap_retry_returns_the_exact_remaining_owner() {
    let memory = parse("kernel/src/mem.rs");
    let retirement = memory
        .items
        .iter()
        .find_map(|item| match item {
            Item::Enum(item) if item.ident == "DeviceRetirement" => Some(item),
            _ => None,
        })
        .expect("missing typed device retirement");
    assert!(matches!(retirement.vis, Visibility::Restricted(_)));
    assert!(exact_derives(&retirement.attrs, &["Debug"]));
    assert_eq!(retirement.variants.len(), 2);
    assert!(
        retirement
            .variants
            .iter()
            .any(|variant| variant.ident == "Retired" && matches!(variant.fields, Fields::Unit))
    );
    assert!(
        retirement
            .variants
            .iter()
            .any(|variant| variant.ident == "Retained"
                && matches!(&variant.fields, Fields::Unnamed(fields)
                if fields.unnamed.len() == 1
                    && matches!(&fields.unnamed[0].ty, Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment|
                            segment.ident == "ReleasableDevice"))))
    );

    let unmap = function(&memory, "unmap_device_exact");
    let mut uses = DeviceRetirementUses::default();
    uses.visit_block(&unmap.block);
    assert_eq!(uses.initial_unmaps, 1);
    assert_eq!(uses.retry_unmaps, 1);
    assert_eq!(uses.retired, 2);
    assert_eq!(uses.retained_region, 2);
    assert_eq!(uses.retained_second_region, 1);
}

#[test]
fn routing_rollback_retains_ownership_before_early_refusal() {
    let main = parse("kernel/src/main.rs");
    let rollback = function(&main, "rollback_keyboard_route");
    let Some(Stmt::Expr(Expr::If(mask), None)) = rollback.block.stmts.get(1) else {
        panic!("routing rollback must handle a refused mask before mapping cleanup");
    };
    assert!(routing_retention_block_is_exact(&mask.then_branch));

    let Some(Stmt::Local(context)) = rollback.block.stmts.get(2) else {
        panic!("routing rollback must retain ownership when no thread context exists");
    };
    let Some((_, otherwise)) = &context.init.as_ref().and_then(|init| init.diverge.as_ref()) else {
        panic!("thread-context acquisition must have an ownership-preserving refusal branch");
    };
    let Expr::Block(otherwise) = &**otherwise else {
        panic!("thread-context refusal must be an explicit retention block");
    };
    assert!(routing_retention_block_is_exact(&otherwise.block));

    for caller in ["route_keyboard", "commit_keyboard_route"] {
        let mut panics = PanicMacros::default();
        <PanicMacros as Visit<'_>>::visit_item_fn(&mut panics, function(&main, caller));
        assert_eq!(
            panics.count, 0,
            "{caller} must keep the polled fallback live"
        );
    }
}

#[test]
fn bar_admission_quarantine_keeps_every_exact_owner() {
    let main = parse("kernel/src/main.rs");
    let mut quarantine_arms = WindowQuarantineArms::default();
    <WindowQuarantineArms as Visit<'_>>::visit_item_fn(
        &mut quarantine_arms,
        function(&main, "claim_candidate"),
    );
    assert_eq!(quarantine_arms.found.len(), 1);
    assert!(window_quarantine_arm_is_exact(quarantine_arms.found[0]));

    let mut admission = WindowAdmissionUses::default();
    <WindowAdmissionUses as Visit<'_>>::visit_item_fn(
        &mut admission,
        function(&main, "admit_controller_window"),
    );
    assert_eq!(admission.release_admitted, 1);
    assert_eq!(admission.retain_authority, 1);
    assert_eq!(admission.retain_mapping, 1);
    assert_eq!(admission.retirement_quarantines, 1);
    assert_eq!(admission.retirement_to_window_quarantine, 1);

    let memory = parse("kernel/src/mem.rs");
    assert!(fixed_retention_sink_is_exact(
        function(&memory, "retain_pci_bar"),
        "RETAINED_PCI_BARS",
        "quarantine",
    ));
    assert!(pci_bar_retention_wrapper_is_exact(
        function(&memory, "retain_pci_bar_authority"),
        "Authority",
        "_failure",
        "failure",
    ));
    assert!(pci_bar_retention_wrapper_is_exact(
        function(&memory, "retain_pci_bar_mapping"),
        "Mapping",
        "_mapping",
        "mapping",
    ));

    let pci = parse("kernel/src/pci.rs");
    assert!(fixed_retention_sink_is_exact(
        function(&pci, "retain_failed_release"),
        "RETAINED_RELEASES",
        "device",
    ));
}

#[test]
fn failed_stop_retention_and_retirement_sinks_are_exact() {
    let kernel = parse("kernel/src/nvme.rs");
    let stop_dma = method(&kernel, "Controller", "stop_dma").expect("controller stop boundary");
    assert!(stop_dma_is_exact(stop_dma));

    let queue_retain = method(&kernel, "QueuePair", "retain").expect("queue retention sink");
    assert!(queue_retention_is_exact(queue_retain));

    let retire = method(&kernel, "Controller", "retire").expect("controller retirement boundary");
    assert!(controller_retirement_is_exact(retire));
    assert!(drop_stops_dma_exactly(&kernel));
}

#[test]
fn controller_stop_waits_for_the_requested_hardware_state() {
    let kernel = parse("kernel/src/nvme.rs");
    assert!(readiness_poll_is_exact(function(&kernel, "wait_for_ready")));
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

#[derive(Default)]
struct DomainStatusGuard {
    translation_checks: usize,
    none_returns: usize,
}

impl Visit<'_> for DomainStatusGuard {
    fn visit_expr_call(&mut self, call: &ExprCall) {
        if path_expression_is(&call.func, &["translation_enabled"]) {
            self.translation_checks += 1;
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_return(&mut self, returned: &syn::ExprReturn) {
        if returned
            .expr
            .as_deref()
            .is_some_and(|expression| path_expression_is(expression, &["None"]))
        {
            self.none_returns += 1;
        }
        syn::visit::visit_expr_return(self, returned);
    }
}

fn call_position(calls: &CallNames, name: &str) -> usize {
    calls
        .0
        .iter()
        .position(|call| call == name)
        .unwrap_or_else(|| panic!("missing `{name}` call from unit transition"))
}

#[test]
fn unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving() {
    let iommu = parse("kernel/src/iommu.rs");
    let installed = iommu
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "InstalledUnit" => Some(item),
            _ => None,
        })
        .expect("unit-wide installed owner");
    let Fields::Named(installed_fields) = &installed.fields else {
        panic!("InstalledUnit must name both inseparable owners");
    };
    let names = installed_fields
        .named
        .iter()
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert_eq!(names, ["isolation", "fences"]);

    let mut establish_calls = CallNames::default();
    establish_calls.visit_item_fn(function(&iommu, "establish"));
    let fences = call_position(&establish_calls, "acquire_fences");
    let mapping = call_position(&establish_calls, "map_device");
    let transition = call_position(&establish_calls, "establish");
    assert!(fences < mapping && mapping < transition);

    let kernel = parse("kernel/src/main.rs");
    let mut retirement_calls = CallNames::default();
    retirement_calls.visit_item_fn(function(&kernel, "retire_candidate"));
    let selected_fence = call_position(&retirement_calls, "settle_bus_master_fence_for_retirement");
    let unit_release = call_position(&retirement_calls, "release_candidate");
    let selected_release = call_position(&retirement_calls, "release");
    assert!(selected_fence < unit_release && unit_release < selected_release);

    let vtd = parse("wost/src/arch/x86_64/vtd.rs");
    assert!(method(&vtd, "Vtd", "disable_translation_for_test").is_none());
    assert!(method(&vtd, "Vtd", "enable_translation_for_test").is_none());

    let mut preflight = CallNames::default();
    preflight.visit_impl_item_fn(method(&vtd, "Plan", "read").expect("VT-d preflight"));
    assert_eq!(
        preflight
            .0
            .iter()
            .filter(|call| call.as_str() == "dma_draining")
            .count(),
        1,
        "read-only preflight must require the complete DMA drain capability"
    );

    let mut transition_calls = CallNames::default();
    transition_calls
        .visit_impl_item_fn(method(&vtd, "Vtd", "transition").expect("VT-d transition"));
    assert!(
        call_position(&transition_calls, "quiesce") < call_position(&transition_calls, "drain_dma")
    );
    assert!(
        call_position(&transition_calls, "drain_dma")
            < call_position(&transition_calls, "set_root_table")
    );
    assert!(
        call_position(&transition_calls, "set_root_table")
            < call_position(&transition_calls, "invalidate")
    );
    assert!(
        call_position(&transition_calls, "invalidate")
            < call_position(&transition_calls, "switch_translation")
    );

    let establishment = vtd
        .items
        .iter()
        .find_map(|item| match item {
            Item::Enum(item) if item.ident == "Establishment" => Some(item),
            _ => None,
        })
        .expect("ownership-carrying establishment outcome");
    let retained = establishment
        .variants
        .iter()
        .find(|variant| variant.ident == "Retained")
        .expect("post-write retention outcome");
    let Fields::Named(retained_fields) = &retained.fields else {
        panic!("retained establishment must name its whole owner");
    };
    assert!(retained_fields.named.iter().any(|field| {
        field.ident.as_ref().is_some_and(|name| name == "isolation")
            && matches!(&field.ty, Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "Vtd"))
    }));

    let mut domain = DomainStatusGuard::default();
    domain.visit_impl_item_fn(method(&vtd, "Vtd", "domain").expect("DMA-domain mint"));
    assert_eq!(domain.translation_checks, 1);
    assert_eq!(domain.none_returns, 2);
}

#[derive(Default)]
struct DmarTopologyArguments {
    inventory: usize,
    reserved: usize,
}

#[derive(Default)]
struct PciTopologyConstructions(usize);

impl Visit<'_> for PciTopologyConstructions {
    fn visit_expr_call(&mut self, call: &ExprCall) {
        if matches!(&*call.func, Expr::Path(path)
            if path_ends(&path.path, &["PciTopology", "new"]))
        {
            self.0 += 1;
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn for_loop_named<'a>(block: &'a syn::Block, name: &str) -> &'a syn::ExprForLoop {
    block
        .stmts
        .iter()
        .find_map(|statement| match statement {
            Stmt::Expr(Expr::ForLoop(loop_), None)
                if matches!(&*loop_.pat, Pat::Ident(binding) if binding.ident == name) =>
            {
                Some(loop_)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing complete topology `{name}` loop"))
}

fn range_from_zero_to(expression: &Expr, end: &[&str], inclusive: bool) -> bool {
    matches!(expression, Expr::Range(range)
        if matches!(range.start.as_deref(), Some(Expr::Lit(literal))
            if matches!(&literal.lit, syn::Lit::Int(value) if value.base10_digits() == "0"))
            && range.end.as_deref().is_some_and(|end_expression|
                path_expression_is(end_expression, end))
            && matches!((&range.limits, inclusive),
                (syn::RangeLimits::Closed(_), true)
                    | (syn::RangeLimits::HalfOpen(_), false)))
}

impl Visit<'_> for DmarTopologyArguments {
    fn visit_expr_method_call(&mut self, call: &ExprMethodCall) {
        if call.method == "requester_inventory"
            && matches!(call.args.iter().collect::<Vec<_>>().as_slice(),
                [bdf, bridge, hierarchy, present]
                    if path_expression_is(bdf, &["bdf"])
                        && path_expression_is(bridge, &["bridge_buses"])
                        && path_expression_is(hierarchy, &["hierarchy_contains"])
                        && path_expression_is(present, &["requester_is_present"]))
        {
            self.inventory += 1;
        }
        if call.method == "reserved_regions_for"
            && matches!(call.args.iter().collect::<Vec<_>>().as_slice(),
                [bdf, bridge, hierarchy]
                    if path_expression_is(bdf, &["bdf"])
                        && path_expression_is(bridge, &["bridge_buses"])
                        && path_expression_is(hierarchy, &["hierarchy_contains"]))
        {
            self.reserved += 1;
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

#[test]
fn dmar_authority_uses_one_complete_sealed_pci_topology() {
    let main = parse("kernel/src/main.rs");
    let mut boot_calls = CallNames::default();
    boot_calls.visit_item_fn(function(&main, "survey_the_bus"));
    assert!(
        call_position(&boot_calls, "snapshot_topology") < call_position(&boot_calls, "enumerate"),
        "the complete topology must seal before enumeration can select a DMA candidate"
    );

    let pci = parse("kernel/src/pci.rs");
    let topology = pci
        .items
        .iter()
        .find_map(|item| match item {
            Item::Static(item) if item.ident == "TOPOLOGY" => Some(item),
            _ => None,
        })
        .expect("the complete topology must live in static storage");
    assert!(matches!(&*topology.ty, Type::Path(path)
        if path.path.segments.last().is_some_and(|segment|
            segment.ident == "SpinLock"
                && matches!(&segment.arguments, syn::PathArguments::AngleBracketed(arguments)
                    if matches!(arguments.args.first(), Some(syn::GenericArgument::Type(Type::Path(inner)))
                        if inner.path.segments.last().is_some_and(|segment| segment.ident == "PciTopology"))))));

    let mut snapshot_calls = CallNames::default();
    snapshot_calls.visit_item_fn(function(&pci, "snapshot_topology"));
    for required in ["reset", "read_function", "observe", "seal"] {
        assert!(
            snapshot_calls.0.iter().any(|call| call == required),
            "topology snapshot lost its `{required}` stage"
        );
    }
    let snapshot = function(&pci, "snapshot_topology");
    let mut local_topologies = PciTopologyConstructions::default();
    local_topologies.visit_item_fn(snapshot);
    assert_eq!(
        local_topologies.0, 0,
        "the complete topology must not consume the fixed boot stack"
    );
    let buses = for_loop_named(&snapshot.block, "bus");
    let devices = for_loop_named(&buses.body, "device");
    let functions = for_loop_named(&devices.body, "function");
    assert!(range_from_zero_to(&buses.expr, &["u8", "MAX"], true));
    assert!(range_from_zero_to(
        &devices.expr,
        &["pci_config", "DEVICES"],
        false
    ));
    assert!(range_from_zero_to(
        &functions.expr,
        &["pci_config", "FUNCTIONS"],
        false
    ));

    let iommu = parse("kernel/src/iommu.rs");
    let lookup = function(&iommu, "look_up");
    let Some(Stmt::Expr(Expr::If(sealed), None)) = lookup.block.stmts.first() else {
        panic!("DMAR lookup must begin with a sealed-topology refusal");
    };
    assert!(matches!(&*sealed.cond, Expr::Unary(unary)
        if matches!(unary.op, syn::UnOp::Not(_))
            && matches!(&*unary.expr, Expr::Call(call)
                if path_expression_is(&call.func, &["pci", "topology_is_sealed"])
                    && call.args.is_empty())));
    assert!(matches!(sealed.then_branch.stmts.as_slice(),
        [Stmt::Expr(Expr::Return(returned), Some(_))]
            if returned.expr.as_deref().is_some_and(|expression|
                matches!(expression, Expr::Call(call)
                    if path_expression_is(&call.func, &["Err"])
                        && matches!(call.args.first(), Some(Expr::Path(path))
                            if path_ends(&path.path, &["LookupError", "UnsafeTopology"]))))));
    let mut lookup_calls = CallNames::default();
    lookup_calls.visit_item_fn(lookup);
    assert!(
        call_position(&lookup_calls, "topology_is_sealed")
            < call_position(&lookup_calls, "requester_inventory"),
        "DMAR lookup must fail closed before deriving requester authority"
    );
    let mut arguments = DmarTopologyArguments::default();
    arguments.visit_item_fn(function(&iommu, "look_up"));
    assert_eq!(arguments.inventory, 1);
    assert_eq!(arguments.reserved, 1);

    for (boundary, query) in [
        ("bridge_buses", "topology_bridge_range"),
        ("hierarchy_contains", "topology_hierarchy_contains"),
        ("requester_is_present", "topology_contains_function"),
    ] {
        let mut paths = ReferencedPaths::default();
        paths.visit_item_fn(function(&iommu, boundary));
        assert!(paths.names.iter().any(|name| name == query));
        assert!(
            paths.names.iter().all(|name| name != "read"),
            "{boundary} must not mix live configuration reads into the sealed snapshot"
        );
    }
}
