//! Source contracts for controlled page-fault recovery.

use std::collections::BTreeSet;
use std::path::PathBuf;

use syn::visit::{self, Visit};
use syn::{
    Expr, ExprCall, ExprField, ExprMethodCall, ExprStruct, FnArg, ImplItem, Item, ItemFn, ItemImpl,
    ItemStruct, Member, Pat, PointerMutability, RangeLimits, ReturnType, Stmt, Type, Visibility,
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

fn type_is(ty: &Type, name: &str) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

fn type_is_exact(ty: &Type, segments: &[&str]) -> bool {
    matches!(ty, Type::Path(path)
        if path.qself.is_none() && path_is_exact(&path.path, segments))
}

fn returns_type(output: &ReturnType, name: &str) -> bool {
    matches!(output, ReturnType::Type(_, ty) if type_is(ty, name))
}

fn path_has_suffix(path: &syn::Path, suffix: &[&str]) -> bool {
    path.segments.len() >= suffix.len()
        && path
            .segments
            .iter()
            .rev()
            .zip(suffix.iter().rev())
            .all(|(segment, expected)| segment.ident == expected)
}

fn path_is_exact(path: &syn::Path, segments: &[&str]) -> bool {
    path.leading_colon.is_none()
        && path.segments.len() == segments.len()
        && path
            .segments
            .iter()
            .zip(segments)
            .all(|(segment, expected)| segment.ident == expected)
}

fn typed_parameter<'function>(function: &'function ItemFn, name: &str) -> Option<&'function Type> {
    function
        .sig
        .inputs
        .iter()
        .find_map(|argument| match argument {
            FnArg::Typed(argument)
                if matches!(&*argument.pat, Pat::Ident(ident)
                if ident.ident == name
                    && ident.by_ref.is_none()
                    && ident.mutability.is_none()
                    && ident.subpat.is_none()) =>
            {
                Some(&*argument.ty)
            }
            _ => None,
        })
}

fn shared_reference_to(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Reference(reference)
        if reference.mutability.is_none() && type_is(&reference.elem, name))
}

fn is_pub_super(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Restricted(restricted) if restricted.path.is_ident("super"))
}

fn tokens_name(tokens: &proc_macro2::TokenStream, wanted: &str) -> bool {
    tokens
        .to_string()
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|token| token == wanted)
}

#[derive(Default)]
struct InstructionPointerExpressions {
    instruction_pointer_fields: usize,
    mutable_addresses: usize,
}

impl<'ast> Visit<'ast> for InstructionPointerExpressions {
    fn visit_item(&mut self, _item: &'ast Item) {}

    fn visit_expr_field(&mut self, expression: &'ast ExprField) {
        if matches!(&expression.member, Member::Named(name) if name == "instruction_pointer") {
            self.instruction_pointer_fields += 1;
        }
        visit::visit_expr_field(self, expression);
    }

    fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
        if path_has_suffix(&expression.mac.path, &["addr_of_mut"])
            && tokens_name(&expression.mac.tokens, "instruction_pointer")
        {
            self.mutable_addresses += 1;
        }
        self.instruction_pointer_fields +=
            usize::from(tokens_name(&expression.mac.tokens, "instruction_pointer"));
        visit::visit_expr_macro(self, expression);
    }

    fn visit_expr_raw_addr(&mut self, expression: &'ast syn::ExprRawAddr) {
        if matches!(expression.mutability, PointerMutability::Mut(_))
            && expression_mentions_instruction_pointer(&expression.expr)
        {
            self.mutable_addresses += 1;
        }
        visit::visit_expr_raw_addr(self, expression);
    }

    fn visit_expr_reference(&mut self, expression: &'ast syn::ExprReference) {
        if expression.mutability.is_some()
            && expression_mentions_instruction_pointer(&expression.expr)
        {
            self.mutable_addresses += 1;
        }
        visit::visit_expr_reference(self, expression);
    }
}

fn expression_shape(expression: &Expr) -> InstructionPointerExpressions {
    let mut shape = InstructionPointerExpressions::default();
    <InstructionPointerExpressions as Visit<'_>>::visit_expr(&mut shape, expression);
    shape
}

fn expression_mentions_instruction_pointer(expression: &Expr) -> bool {
    expression_shape(expression).instruction_pointer_fields != 0
}

fn expression_contains_alias(expression: &Expr, aliases: &BTreeSet<String>) -> bool {
    struct AliasUse<'aliases> {
        aliases: &'aliases BTreeSet<String>,
        found: bool,
    }

    impl<'ast> Visit<'ast> for AliasUse<'_> {
        fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
            if expression.qself.is_none()
                && expression.path.segments.len() == 1
                && self
                    .aliases
                    .contains(&expression.path.segments[0].ident.to_string())
            {
                self.found = true;
            }
            visit::visit_expr_path(self, expression);
        }
    }

    let mut usage = AliasUse {
        aliases,
        found: false,
    };
    <AliasUse<'_> as Visit<'_>>::visit_expr(&mut usage, expression);
    usage.found
}

fn instruction_pointer_aliases(block: &syn::Block) -> BTreeSet<String> {
    struct Initializers<'ast> {
        values: Vec<(String, &'ast Expr)>,
    }

    impl<'ast> Visit<'ast> for Initializers<'ast> {
        fn visit_item(&mut self, _item: &'ast Item) {}

        fn visit_local(&mut self, local: &'ast syn::Local) {
            if let (Pat::Ident(binding), Some(initializer)) = (&local.pat, &local.init)
                && binding.by_ref.is_none()
                && binding.subpat.is_none()
            {
                self.values
                    .push((binding.ident.to_string(), &initializer.expr));
            }
            visit::visit_local(self, local);
        }
    }

    let mut initializers = Initializers { values: Vec::new() };
    initializers.visit_block(block);
    let mut aliases = BTreeSet::new();
    loop {
        let before = aliases.len();
        for (name, initializer) in &initializers.values {
            let shape = expression_shape(initializer);
            if shape.mutable_addresses != 0 || expression_contains_alias(initializer, &aliases) {
                aliases.insert(name.clone());
            }
        }
        if aliases.len() == before {
            return aliases;
        }
    }
}

fn target_address(expression: &Expr) -> bool {
    matches!(expression, Expr::MethodCall(address)
        if address.method == "address"
            && address.args.is_empty()
            && matches!(&*address.receiver, Expr::Path(path) if path.path.is_ident("target")))
}

fn dereference(expression: &Expr) -> bool {
    matches!(expression, Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)))
}

#[derive(Default)]
struct WriterShape {
    instruction_pointer_fields: usize,
    mutable_instruction_pointer_addresses: usize,
    writes: usize,
    instruction_pointer_writes: usize,
    target_address_writes: usize,
}

struct WriteSinks<'aliases> {
    aliases: &'aliases BTreeSet<String>,
    writes: usize,
    instruction_pointer_writes: usize,
    target_address_writes: usize,
}

impl WriteSinks<'_> {
    fn record(&mut self, destination: &Expr, value: &Expr) {
        self.writes += 1;
        let shape = expression_shape(destination);
        if shape.instruction_pointer_fields != 0
            || shape.mutable_addresses != 0
            || expression_contains_alias(destination, self.aliases)
        {
            self.instruction_pointer_writes += 1;
            self.target_address_writes += usize::from(target_address(value));
        }
    }
}

impl<'ast> Visit<'ast> for WriteSinks<'_> {
    fn visit_item(&mut self, _item: &'ast Item) {}

    fn visit_expr_method_call(&mut self, expression: &'ast ExprMethodCall) {
        if matches!(
            expression.method.to_string().as_str(),
            "write" | "write_volatile"
        ) && expression.args.len() == 1
        {
            self.record(&expression.receiver, &expression.args[0]);
        }
        visit::visit_expr_method_call(self, expression);
    }

    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if matches!(&*expression.func, Expr::Path(path)
                if path_has_suffix(&path.path, &["ptr", "write"])
                    || path_has_suffix(&path.path, &["ptr", "write_volatile"]))
            && expression.args.len() == 2
        {
            self.record(&expression.args[0], &expression.args[1]);
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_assign(&mut self, expression: &'ast syn::ExprAssign) {
        let shape = expression_shape(&expression.left);
        if dereference(&expression.left)
            || shape.instruction_pointer_fields != 0
            || shape.mutable_addresses != 0
            || expression_contains_alias(&expression.left, self.aliases)
        {
            self.record(&expression.left, &expression.right);
        }
        visit::visit_expr_assign(self, expression);
    }
}

fn writer_shape_in(block: &syn::Block) -> WriterShape {
    let mut expressions = InstructionPointerExpressions::default();
    expressions.visit_block(block);
    let aliases = instruction_pointer_aliases(block);
    let mut sinks = WriteSinks {
        aliases: &aliases,
        writes: 0,
        instruction_pointer_writes: 0,
        target_address_writes: 0,
    };
    sinks.visit_block(block);
    WriterShape {
        instruction_pointer_fields: expressions.instruction_pointer_fields,
        mutable_instruction_pointer_addresses: expressions.mutable_addresses,
        writes: sinks.writes,
        instruction_pointer_writes: sinks.instruction_pointer_writes,
        target_address_writes: sinks.target_address_writes,
    }
}

fn writer_shape(function: &ItemFn) -> WriterShape {
    writer_shape_in(&function.block)
}

#[derive(Default)]
struct WriterFunctions<'ast> {
    free: Vec<&'ast ItemFn>,
    associated: Vec<&'ast syn::ImplItemFn>,
    trait_defaults: Vec<&'ast syn::TraitItemFn>,
}

impl<'ast> Visit<'ast> for WriterFunctions<'ast> {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let shape = writer_shape(function);
        if shape.mutable_instruction_pointer_addresses != 0 || shape.instruction_pointer_writes != 0
        {
            self.free.push(function);
        }
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        let shape = writer_shape_in(&function.block);
        if shape.mutable_instruction_pointer_addresses != 0 || shape.instruction_pointer_writes != 0
        {
            self.associated.push(function);
        }
        visit::visit_impl_item_fn(self, function);
    }

    fn visit_trait_item_fn(&mut self, function: &'ast syn::TraitItemFn) {
        if let Some(block) = &function.default {
            let shape = writer_shape_in(block);
            if shape.mutable_instruction_pointer_addresses != 0
                || shape.instruction_pointer_writes != 0
            {
                self.trait_defaults.push(function);
            }
        }
        visit::visit_trait_item_fn(self, function);
    }
}

fn calls_live_frame_on_frame(expression: &Expr) -> bool {
    let Expr::Unary(not) = expression else {
        return false;
    };
    if !matches!(not.op, syn::UnOp::Not(_)) {
        return false;
    }
    let Expr::Call(ExprCall { func, args, .. }) = &*not.expr else {
        return false;
    };
    matches!(&**func, Expr::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "is_live_frame"))
        && args.len() == 1
        && matches!(args.first(), Some(Expr::Path(path)) if path.path.is_ident("frame"))
}

fn returns_false(statements: &[Stmt]) -> bool {
    statements.len() == 1
        && matches!(&statements[0], Stmt::Expr(Expr::Return(returned), _)
            if matches!(returned.expr.as_deref(), Some(Expr::Lit(literal))
                if matches!(&literal.lit, syn::Lit::Bool(value) if !value.value)))
}

fn starts_with_live_frame_refusal(function: &ItemFn) -> bool {
    matches!(function.block.stmts.first(), Some(Stmt::Expr(Expr::If(branch), _))
        if calls_live_frame_on_frame(&branch.cond) && returns_false(&branch.then_branch.stmts))
}

fn compares_the_live_frame_origin(function: &ItemFn) -> bool {
    matches!(function.block.stmts.as_slice(), [Stmt::Expr(Expr::Binary(comparison), _) ]
        if matches!(comparison.op, syn::BinOp::Eq(_))
            && matches!(&*comparison.left, Expr::MethodCall(load)
                if load.method == "load"
                    && path_is(&load.receiver, "LIVE_FRAME")
                    && load.args.len() == 1
                    && load.args.first().is_some_and(|ordering| path_ends_in(ordering, "Relaxed")))
            && matches!(&*comparison.right, Expr::Field(field)
                if path_is(&field.base, "frame")
                    && matches!(&field.member, Member::Named(name) if name == "origin")))
}

fn binding_count(block: &syn::Block, name: &str) -> usize {
    struct Bindings<'name> {
        name: &'name str,
        count: usize,
    }

    impl<'ast> Visit<'ast> for Bindings<'_> {
        fn visit_item(&mut self, _item: &'ast Item) {}

        fn visit_pat_ident(&mut self, pattern: &'ast syn::PatIdent) {
            if pattern.ident == self.name {
                self.count += 1;
            }
            visit::visit_pat_ident(self, pattern);
        }
    }

    let mut bindings = Bindings { name, count: 0 };
    bindings.visit_block(block);
    bindings.count
}

fn immutable_local_expression<'function>(
    function: &'function ItemFn,
    name: &str,
) -> Option<&'function Expr> {
    if binding_count(&function.block, name) != 1 {
        return None;
    }
    function
        .block
        .stmts
        .iter()
        .find_map(|statement| match statement {
            Stmt::Local(local)
                if matches!(&local.pat, Pat::Ident(pattern)
                    if pattern.ident == name
                        && pattern.by_ref.is_none()
                        && pattern.mutability.is_none()
                        && pattern.subpat.is_none())
                    && local
                        .init
                        .as_ref()
                        .is_some_and(|init| init.diverge.is_none()) =>
            {
                local.init.as_ref().map(|init| &*init.expr)
            }
            _ => None,
        })
}

fn immutable_some_local_expression<'function>(
    function: &'function ItemFn,
    name: &str,
) -> Option<&'function Expr> {
    if binding_count(&function.block, name) != 1 {
        return None;
    }
    function
        .block
        .stmts
        .iter()
        .find_map(|statement| match statement {
            Stmt::Local(local)
                if matches!(&local.pat, Pat::TupleStruct(pattern)
                    if path_is_exact(&pattern.path, &["Some"])
                        && pattern.elems.len() == 1
                        && matches!(pattern.elems.first(), Some(Pat::Ident(binding))
                            if binding.ident == name
                                && binding.by_ref.is_none()
                                && binding.mutability.is_none()
                                && binding.subpat.is_none()))
                    && local.init.as_ref().is_some_and(|init| {
                        init.diverge.as_ref().is_some_and(|(_, expression)| {
                            matches!(&**expression, Expr::Block(block)
                                if block.label.is_none()
                                    && returns_false(&block.block.stmts))
                        })
                    }) =>
            {
                local.init.as_ref().map(|init| &*init.expr)
            }
            _ => None,
        })
}

fn address_of_symbol(expression: &Expr, symbol: &str) -> bool {
    matches!(expression, Expr::Cast(cast)
        if type_is_exact(&cast.ty, &["u64"])
            && matches!(&*cast.expr, Expr::Macro(address)
                if path_is_exact(&address.mac.path, &["core", "ptr", "addr_of"])
                    && address.mac.tokens.to_string() == symbol))
}

fn checked_add_one(expression: &Expr, receiver: &str) -> bool {
    matches!(expression, Expr::MethodCall(call)
        if call.method == "checked_add"
            && path_is(&call.receiver, receiver)
            && call.args.len() == 1
            && call.args.first().is_some_and(is_one))
}

fn local_struct<'function>(
    function: &'function ItemFn,
    binding: &str,
    type_name: &str,
) -> &'function ExprStruct {
    immutable_local_expression(function, binding)
        .and_then(|expression| match expression {
            Expr::Struct(initializer)
                if initializer
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == type_name) =>
            {
                Some(initializer)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing `{binding}: {type_name}` initializer"))
}

fn initialized_field<'initializer>(
    initializer: &'initializer ExprStruct,
    name: &str,
) -> &'initializer Expr {
    initializer
        .fields
        .iter()
        .find_map(|field| {
            matches!(&field.member, Member::Named(member) if member == name).then_some(&field.expr)
        })
        .unwrap_or_else(|| panic!("missing `{name}` field"))
}

fn has_exact_fields(initializer: &ExprStruct, expected: &[&str]) -> bool {
    initializer.rest.is_none()
        && initializer.fields.len() == expected.len()
        && expected.iter().all(|expected| {
            initializer
                .fields
                .iter()
                .any(|field| matches!(&field.member, Member::Named(name) if name == expected))
        })
}

fn path_is(expression: &Expr, name: &str) -> bool {
    matches!(expression, Expr::Path(path) if path.path.is_ident(name))
}

fn reference_to(expression: &Expr, name: &str) -> bool {
    matches!(expression, Expr::Reference(reference) if path_is(&reference.expr, name))
}

fn method_on(expression: &Expr, receiver: &str, method: &str) -> bool {
    matches!(expression, Expr::MethodCall(call)
        if call.method == method && call.args.is_empty() && path_is(&call.receiver, receiver))
}

fn call_path(expression: &Expr, segments: &[&str]) -> bool {
    matches!(expression, Expr::Call(call)
        if call.args.is_empty()
            && matches!(&*call.func, Expr::Path(path)
                if path.qself.is_none() && path_is_exact(&path.path, segments)))
}

fn path_ends_in(expression: &Expr, name: &str) -> bool {
    matches!(expression, Expr::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == name))
}

fn half_open_range(
    expression: &Expr,
    start: impl Fn(&Expr) -> bool,
    end: impl Fn(&Expr) -> bool,
) -> bool {
    matches!(expression, Expr::Range(range)
        if matches!(range.limits, RangeLimits::HalfOpen(_))
            && range.start.as_deref().is_some_and(start)
            && range.end.as_deref().is_some_and(end))
}

fn is_zero(expression: &Expr) -> bool {
    matches!(expression, Expr::Lit(literal)
        if matches!(&literal.lit, syn::Lit::Int(value)
            if value.base10_parse::<u64>().is_ok_and(|value| value == 0)))
}

fn is_one(expression: &Expr) -> bool {
    matches!(expression, Expr::Lit(literal)
        if matches!(&literal.lit, syn::Lit::Int(value)
            if value.base10_parse::<u64>().is_ok_and(|value| value == 1)))
}

fn refuses_unadmitted_attempt(function: &ItemFn) -> bool {
    function.block.stmts.iter().any(|statement| {
        matches!(statement, Stmt::Expr(Expr::If(branch), _)
            if matches!(&*branch.cond, Expr::Unary(not)
                if matches!(not.op, syn::UnOp::Not(_))
                    && matches!(&*not.expr, Expr::MethodCall(call)
                    if call.method == "is_admitted_by"
                        && path_is(&call.receiver, "attempt")
                        && call.args.len() == 1
                        && call.args.first().is_some_and(|argument| reference_to(argument, "rule"))))
                && returns_false(&branch.then_branch.stmts))
    })
}

#[derive(Default)]
struct AuthoritySurface<'ast> {
    implementations: Vec<&'ast ItemImpl>,
    module_constructors: Vec<&'ast ItemFn>,
    foreign_methods: Vec<&'ast syn::ImplItemFn>,
    named_constructions: usize,
    fixup_references: usize,
    type_references: usize,
    use_aliases: usize,
}

struct AuthorityCollector<'ast, 'name> {
    name: &'name str,
    surface: AuthoritySurface<'ast>,
}

impl<'ast> Visit<'ast> for AuthorityCollector<'ast, '_> {
    fn visit_item_use(&mut self, import: &'ast syn::ItemUse) {
        fn aliases(tree: &syn::UseTree, name: &str) -> usize {
            match tree {
                syn::UseTree::Path(path) => aliases(&path.tree, name),
                syn::UseTree::Rename(rename) => usize::from(rename.ident == name),
                syn::UseTree::Group(group) => {
                    group.items.iter().map(|tree| aliases(tree, name)).sum()
                }
                _ => 0,
            }
        }

        self.surface.use_aliases += aliases(&import.tree, self.name);
        visit::visit_item_use(self, import);
    }

    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        let owns_authority = type_is(&implementation.self_ty, self.name);
        if owns_authority {
            self.surface.implementations.push(implementation);
        }
        for item in &implementation.items {
            if let ImplItem::Fn(method) = item
                && !owns_authority
                && returns_type(&method.sig.output, self.name)
            {
                self.surface.foreign_methods.push(method);
            }
        }
        visit::visit_item_impl(self, implementation);
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        if returns_type(&function.sig.output, self.name) {
            self.surface.module_constructors.push(function);
        }
        visit::visit_item_fn(self, function);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if matches!(&*call.func, Expr::Path(path) if path_has_suffix(&path.path, &[self.name])) {
            self.surface.named_constructions += 1;
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path_has_suffix(&path.path, &[self.name, "fixup"]) {
            self.surface.fixup_references += 1;
        }
        visit::visit_expr_path(self, path);
    }

    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path.qself.is_none() && path_has_suffix(&path.path, &[self.name]) {
            self.surface.type_references += 1;
        }
        visit::visit_type_path(self, path);
    }
}

struct FixupReferences<'name> {
    name: &'name str,
    count: usize,
}

impl<'ast> Visit<'ast> for FixupReferences<'_> {
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path_has_suffix(&path.path, &[self.name, "fixup"]) {
            self.count += 1;
        }
        visit::visit_expr_path(self, path);
    }
}

fn fixup_references(function: &ItemFn, name: &str) -> usize {
    let mut references = FixupReferences { name, count: 0 };
    Visit::visit_item_fn(&mut references, function);
    references.count
}

fn sole_expression(method: &syn::ImplItemFn) -> Option<&Expr> {
    match method.block.stmts.as_slice() {
        [Stmt::Expr(expression, None)] => Some(expression),
        _ => None,
    }
}

fn fixup_names_symbol(method: &syn::ImplItemFn, symbol: &str) -> bool {
    let Some(Expr::Call(call)) = sole_expression(method) else {
        return false;
    };
    let Some(argument) = call.args.first() else {
        return false;
    };
    call.args.len() == 1
        && matches!(&*call.func, Expr::Path(path) if path.path.is_ident("Self"))
        && address_of_symbol(argument, symbol)
}

fn address_returns_private_field(method: &syn::ImplItemFn) -> bool {
    matches!(sole_expression(method), Some(Expr::Field(field))
        if matches!(&*field.base, Expr::Path(path) if path.path.is_ident("self"))
            && matches!(&field.member, Member::Unnamed(index) if index.index == 0))
}

fn has_derive(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("derive")
            || matches!(&attribute.meta, syn::Meta::List(list)
                if list.path.is_ident("cfg_attr") && tokens_name(&list.tokens, "derive"))
    })
}

fn has_plain_signature(signature: &syn::Signature) -> bool {
    signature.constness.is_none()
        && signature.asyncness.is_none()
        && signature.unsafety.is_none()
        && signature.abi.is_none()
        && signature.variadic.is_none()
        && signature.generics.params.is_empty()
        && signature.generics.where_clause.is_none()
}

fn is_exact_fixup(method: &syn::ImplItemFn, symbol: &str) -> bool {
    matches!(method.vis, Visibility::Inherited)
        && has_plain_signature(&method.sig)
        && method.sig.inputs.is_empty()
        && matches!(&method.sig.output, ReturnType::Type(_, ty)
            if type_is_exact(ty, &["Self"]))
        && fixup_names_symbol(method, symbol)
}

fn is_exact_address_observer(method: &syn::ImplItemFn) -> bool {
    is_pub_super(&method.vis)
        && method.sig.constness.is_some()
        && method.sig.asyncness.is_none()
        && method.sig.unsafety.is_none()
        && method.sig.abi.is_none()
        && method.sig.variadic.is_none()
        && method.sig.generics.params.is_empty()
        && method.sig.generics.where_clause.is_none()
        && matches!(method.sig.inputs.first(), Some(FnArg::Receiver(receiver))
            if method.sig.inputs.len() == 1
                && receiver.reference.as_ref().is_some_and(|(_, lifetime)| lifetime.is_none())
                && receiver.mutability.is_none()
                && receiver.colon_token.is_none())
        && matches!(&method.sig.output, ReturnType::Type(_, ty)
            if type_is_exact(ty, &["u64"]))
        && address_returns_private_field(method)
}

fn authority<'a>(syntax: &'a syn::File, name: &str) -> (&'a ItemStruct, AuthoritySurface<'a>) {
    let structure = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(structure) if structure.ident == name => Some(structure),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing authority `{name}`"));
    let mut collector = AuthorityCollector {
        name,
        surface: AuthoritySurface::default(),
    };
    collector.visit_file(syntax);
    assert!(
        !collector.surface.implementations.is_empty(),
        "missing implementation for `{name}`"
    );
    (structure, collector.surface)
}

#[test]
fn each_recovery_adapter_binds_the_live_architectural_values() {
    let usercopy = parse("wost/src/arch/x86_64/usercopy.rs");
    let usercopy = function(&usercopy, "recover");
    assert!(
        typed_parameter(usercopy, "frame").is_some_and(|ty| shared_reference_to(ty, "TrapFrame"))
            && typed_parameter(usercopy, "fault").is_some_and(|ty| type_is(ty, "PageFault"))
            && binding_count(&usercopy.block, "frame") == 0
            && binding_count(&usercopy.block, "fault") == 0
            && binding_count(&usercopy.block, "faulting_address") == 0,
        "user-copy recovery no longer uses its immutable architectural inputs"
    );
    assert!(
        immutable_local_expression(usercopy, "at").is_some_and(|expression| method_on(
            expression,
            "frame",
            "instruction_pointer"
        )),
        "user-copy recovery no longer binds `at` immutably from the live frame"
    );
    assert!(
        immutable_local_expression(usercopy, "start")
            .is_some_and(|expression| address_of_symbol(expression, "wost_copy_fault_start")),
        "user-copy recovery no longer binds its exact fault start"
    );
    assert!(
        immutable_local_expression(usercopy, "end")
            .is_some_and(|expression| address_of_symbol(expression, "wost_copy_fault_end")),
        "user-copy recovery no longer binds its exact fault end"
    );
    let attempt = local_struct(usercopy, "attempt", "RecoveryAttempt");
    assert!(has_exact_fields(
        attempt,
        &[
            "vector",
            "frame_error_code",
            "code_selector",
            "instruction_pointer",
            "page_fault",
            "faulting_address",
        ]
    ));
    assert!(method_on(
        initialized_field(attempt, "vector"),
        "frame",
        "vector"
    ));
    assert!(method_on(
        initialized_field(attempt, "frame_error_code"),
        "frame",
        "error_code"
    ));
    assert!(method_on(
        initialized_field(attempt, "code_selector"),
        "frame",
        "code_selector"
    ));
    assert!(path_is(
        initialized_field(attempt, "instruction_pointer"),
        "at"
    ));
    assert!(path_is(initialized_field(attempt, "page_fault"), "fault"));
    assert!(call_path(
        initialized_field(attempt, "faulting_address"),
        &["cpu", "faulting_address"]
    ));
    let rule = local_struct(usercopy, "rule", "RecoveryRule");
    assert!(has_exact_fields(
        rule,
        &["fault", "fault_addresses", "instruction_addresses"]
    ));
    assert!(path_ends_in(
        initialized_field(rule, "fault"),
        "SupervisorData"
    ));
    assert!(half_open_range(
        initialized_field(rule, "fault_addresses"),
        is_zero,
        |end| path_is(end, "USER_END")
    ));
    assert!(half_open_range(
        initialized_field(rule, "instruction_addresses"),
        |start| path_is(start, "start"),
        |end| path_is(end, "end")
    ));
    assert!(refuses_unadmitted_attempt(usercopy));

    let cpu = parse("wost/src/arch/x86_64/cpu.rs");
    let write_protection = function(&cpu, "recover_write_protection_probe");
    assert!(
        typed_parameter(write_protection, "frame")
            .is_some_and(|ty| shared_reference_to(ty, "TrapFrame"))
            && typed_parameter(write_protection, "fault")
                .is_some_and(|ty| type_is(ty, "PageFault"))
            && binding_count(&write_protection.block, "frame") == 0
            && binding_count(&write_protection.block, "fault") == 0
            && binding_count(&write_protection.block, "faulting_address") == 0,
        "write-protection recovery no longer uses its immutable architectural inputs"
    );
    assert!(
        immutable_local_expression(write_protection, "at").is_some_and(|expression| method_on(
            expression,
            "frame",
            "instruction_pointer"
        )),
        "write-protection recovery no longer binds `at` immutably from the live frame"
    );
    assert!(
        immutable_local_expression(write_protection, "start").is_some_and(|expression| {
            address_of_symbol(expression, "wost_write_protection_fault")
        }),
        "write-protection recovery no longer binds its exact fault start"
    );
    assert!(
        immutable_local_expression(write_protection, "end").is_some_and(|expression| {
            address_of_symbol(expression, "wost_write_protection_fault_end")
        }),
        "write-protection recovery no longer binds its exact fault end"
    );
    assert!(
        immutable_local_expression(write_protection, "target").is_some_and(|expression| {
            address_of_symbol(expression, "wost_write_protection_target")
        }),
        "write-protection recovery no longer binds its exact target byte"
    );
    assert!(
        immutable_some_local_expression(write_protection, "target_end")
            .is_some_and(|expression| checked_add_one(expression, "target")),
        "write-protection recovery no longer checks the target byte's exclusive end"
    );
    let attempt = local_struct(write_protection, "attempt", "RecoveryAttempt");
    assert!(has_exact_fields(
        attempt,
        &[
            "vector",
            "frame_error_code",
            "code_selector",
            "instruction_pointer",
            "page_fault",
            "faulting_address",
        ]
    ));
    assert!(method_on(
        initialized_field(attempt, "vector"),
        "frame",
        "vector"
    ));
    assert!(method_on(
        initialized_field(attempt, "frame_error_code"),
        "frame",
        "error_code"
    ));
    assert!(method_on(
        initialized_field(attempt, "code_selector"),
        "frame",
        "code_selector"
    ));
    assert!(path_is(
        initialized_field(attempt, "instruction_pointer"),
        "at"
    ));
    assert!(path_is(initialized_field(attempt, "page_fault"), "fault"));
    assert!(call_path(
        initialized_field(attempt, "faulting_address"),
        &["faulting_address"]
    ));
    let rule = local_struct(write_protection, "rule", "RecoveryRule");
    assert!(has_exact_fields(
        rule,
        &["fault", "fault_addresses", "instruction_addresses"]
    ));
    assert!(path_ends_in(
        initialized_field(rule, "fault"),
        "SupervisorWriteProtection"
    ));
    assert!(half_open_range(
        initialized_field(rule, "fault_addresses"),
        |start| path_is(start, "target"),
        |end| path_is(end, "target_end")
    ));
    assert!(half_open_range(
        initialized_field(rule, "instruction_addresses"),
        |start| path_is(start, "start"),
        |end| path_is(end, "end")
    ));
    assert!(refuses_unadmitted_attempt(write_protection));
}

#[test]
fn every_instruction_pointer_writer_requires_its_opaque_authority() {
    let syntax = parse("wost/src/arch/x86_64/interrupt.rs");
    let expected = [
        ("resume_usercopy", "RecoveryTarget"),
        ("resume_write_protection", "WriteProtectionRecoveryTarget"),
    ];

    let mut writers = WriterFunctions::default();
    writers.visit_file(&syntax);
    assert!(
        writers.associated.is_empty(),
        "an associated function can rewrite an instruction pointer"
    );
    assert!(
        writers.trait_defaults.is_empty(),
        "a trait default can rewrite an instruction pointer"
    );
    assert_eq!(writers.free.len(), expected.len());

    for (name, authority) in expected {
        let writer = writers
            .free
            .iter()
            .find(|writer| writer.sig.ident == name)
            .unwrap_or_else(|| panic!("missing instruction-pointer writer `{name}`"));
        assert!(is_pub_super(&writer.vis), "`{name}` escaped its module");
        assert!(
            typed_parameter(writer, "target").is_some_and(|ty| type_is(ty, authority)),
            "`{name}` accepts an untyped recovery target"
        );
        assert!(
            writer
                .sig
                .inputs
                .iter()
                .all(|argument| !matches!(argument, FnArg::Typed(argument) if type_is(&argument.ty, "u64"))),
            "`{name}` accepts a raw resume address"
        );
        let shape = writer_shape(writer);
        assert_eq!(shape.instruction_pointer_fields, 1, "`{name}` writer field");
        assert_eq!(shape.writes, 1, "`{name}` write count");
        assert_eq!(shape.target_address_writes, 1, "`{name}` target write");
    }
}

fn assert_writer_fixture(
    source: &str,
    expected_free: &[&str],
    expected_associated: &[&str],
    expected_trait_defaults: &[&str],
    exact_free_sinks: bool,
) {
    let syntax = syn::parse_file(source).expect("valid instruction-pointer writer witness");
    let mut writers = WriterFunctions::default();
    writers.visit_file(&syntax);
    let mut free: Vec<_> = writers
        .free
        .iter()
        .map(|function| function.sig.ident.to_string())
        .collect();
    let mut associated: Vec<_> = writers
        .associated
        .iter()
        .map(|function| function.sig.ident.to_string())
        .collect();
    let mut trait_defaults: Vec<_> = writers
        .trait_defaults
        .iter()
        .map(|function| function.sig.ident.to_string())
        .collect();
    free.sort();
    associated.sort();
    trait_defaults.sort();
    assert_eq!(free, expected_free);
    assert_eq!(associated, expected_associated);
    assert_eq!(trait_defaults, expected_trait_defaults);
    if exact_free_sinks {
        for function in writers.free {
            let shape = writer_shape(function);
            assert_eq!(
                shape.instruction_pointer_writes, 1,
                "`{}` instruction-pointer sink",
                function.sig.ident
            );
            assert_eq!(
                shape.target_address_writes, 1,
                "`{}` opaque-target sink",
                function.sig.ident
            );
        }
    }
}

#[test]
fn raw_pointer_write_functions_remain_visible() {
    assert_writer_fixture(
        r#"
            struct Frame { instruction_pointer: u64 }
            struct Target;
            impl Target { fn address(&self) -> u64 { 0 } }
            fn raw(frame: *mut Frame, target: Target) {
                core::ptr::write(
                    core::ptr::addr_of_mut!((*frame).instruction_pointer),
                    target.address(),
                );
            }
            fn volatile(frame: *mut Frame, target: Target) {
                core::ptr::write_volatile(
                    core::ptr::addr_of_mut!((*frame).instruction_pointer),
                    target.address(),
                );
            }
        "#,
        &["raw", "volatile"],
        &[],
        &[],
        true,
    );
}

#[test]
fn direct_instruction_pointer_writes_remain_visible() {
    assert_writer_fixture(
        r#"
            struct Frame { instruction_pointer: u64 }
            struct Target;
            impl Target { fn address(&self) -> u64 { 0 } }
            fn method(frame: *mut Frame, target: Target) {
                core::ptr::addr_of_mut!((*frame).instruction_pointer)
                    .write(target.address());
            }
            fn assignment(frame: *mut Frame, target: Target) {
                (*frame).instruction_pointer = target.address();
            }
        "#,
        &["assignment", "method"],
        &[],
        &[],
        true,
    );
}

#[test]
fn split_instruction_pointer_writes_remain_visible() {
    assert_writer_fixture(
        r#"
            struct Frame { instruction_pointer: u64 }
            struct Target;
            impl Target { fn address(&self) -> u64 { 0 } }
            fn method(frame: *mut Frame, target: Target) {
                let slot = core::ptr::addr_of_mut!((*frame).instruction_pointer);
                let destination = slot;
                destination.write(target.address());
            }
            fn raw(frame: *mut Frame, target: Target) {
                let slot = core::ptr::addr_of_mut!((*frame).instruction_pointer);
                core::ptr::write(slot, target.address());
            }
            fn assignment(frame: *mut Frame, target: Target) {
                let slot = core::ptr::addr_of_mut!((*frame).instruction_pointer);
                *slot = target.address();
            }
        "#,
        &["assignment", "method", "raw"],
        &[],
        &[],
        true,
    );
}

#[test]
fn nested_and_associated_instruction_pointer_writers_remain_visible() {
    assert_writer_fixture(
        r#"
            struct Frame { instruction_pointer: u64 }
            mod nested {
                fn nested(frame: *mut super::Frame) {
                    core::ptr::addr_of_mut!((*frame).instruction_pointer).write(0);
                }
            }
            struct Adapter;
            impl Adapter {
                fn associated(frame: *mut Frame) {
                    core::ptr::addr_of_mut!((*frame).instruction_pointer).write(0);
                }
            }
            trait AdapterTrait {
                fn default(frame: *mut Frame) {
                    core::ptr::addr_of_mut!((*frame).instruction_pointer).write(0);
                }
            }
        "#,
        &["nested"],
        &["associated"],
        &["default"],
        false,
    );
}

#[test]
fn every_instruction_pointer_writer_refuses_a_non_live_frame_first() {
    let syntax = parse("wost/src/arch/x86_64/interrupt.rs");
    assert!(
        compares_the_live_frame_origin(function(&syntax, "is_live_frame")),
        "live-frame authority is not the exact saved-frame origin"
    );
    for name in ["resume_usercopy", "resume_write_protection"] {
        assert!(
            starts_with_live_frame_refusal(function(&syntax, name)),
            "`{name}` can reach a stale trap frame"
        );
    }
}

#[test]
fn recovery_authority_cannot_be_built_from_a_raw_address() {
    let cases = [
        (
            "wost/src/arch/x86_64/usercopy.rs",
            "RecoveryTarget",
            "recover",
            "wost_copy_fixup",
        ),
        (
            "wost/src/arch/x86_64/cpu.rs",
            "WriteProtectionRecoveryTarget",
            "recover_write_protection_probe",
            "wost_write_protection_fixup",
        ),
    ];

    for (relative, name, recovery, symbol) in cases {
        let syntax = parse(relative);
        let (structure, surface) = authority(&syntax, name);
        assert!(is_pub_super(&structure.vis), "`{name}` has the wrong scope");
        assert!(
            !has_derive(&structure.attrs),
            "`{name}` gained derived duplication or construction"
        );
        assert!(
            structure.generics.params.is_empty() && structure.generics.where_clause.is_none(),
            "`{name}` gained generic construction state"
        );
        let fields = match &structure.fields {
            syn::Fields::Unnamed(fields) => &fields.unnamed,
            _ => panic!("`{name}` is not an opaque tuple authority"),
        };
        assert_eq!(fields.len(), 1, "`{name}` field count");
        assert!(matches!(fields[0].vis, Visibility::Inherited));
        assert!(type_is_exact(&fields[0].ty, &["u64"]));

        assert_eq!(
            surface.implementations.len(),
            1,
            "`{name}` must have one exact inherent implementation"
        );
        let implementation = surface.implementations[0];
        assert!(
            implementation.trait_.is_none()
                && implementation.generics.params.is_empty()
                && implementation.generics.where_clause.is_none(),
            "`{name}` gained a trait or generic implementation"
        );
        assert_eq!(
            implementation.items.len(),
            2,
            "`{name}` gained an associated construction surface"
        );
        let mut methods: Vec<_> = implementation
            .items
            .iter()
            .map(|item| match item {
                ImplItem::Fn(method) => method,
                _ => panic!("`{name}` gained a non-function implementation item"),
            })
            .collect();
        methods.sort_by_key(|method| method.sig.ident.to_string());
        assert_eq!(
            methods
                .iter()
                .map(|method| method.sig.ident.to_string())
                .collect::<Vec<_>>(),
            ["address", "fixup"]
        );
        let fixup = methods
            .iter()
            .find(|method| method.sig.ident == "fixup")
            .expect("private fixup constructor");
        assert!(
            is_exact_fixup(fixup, symbol),
            "`{name}` no longer has the exact private linker-fixup constructor"
        );
        let address = methods
            .iter()
            .find(|method| method.sig.ident == "address")
            .expect("private-field observer");
        assert!(
            is_exact_address_observer(address),
            "`{name}` no longer has the exact private-field observer"
        );
        assert!(
            methods.iter().all(|method| method
                .sig
                .inputs
                .iter()
                .all(|argument| !matches!(argument, FnArg::Typed(argument)
                    if type_is(&argument.ty, "u64")))),
            "`{name}` regained a raw-address constructor"
        );
        assert!(
            surface.module_constructors.is_empty(),
            "`{name}` regained a module-level constructor"
        );
        assert!(
            surface.foreign_methods.is_empty(),
            "`{name}` can be returned by another type's method"
        );
        assert_eq!(
            surface.named_constructions, 0,
            "`{name}` was constructed outside its private `Self` constructor"
        );
        assert_eq!(
            surface.type_references, 1,
            "`{name}` gained another typed construction surface"
        );
        assert_eq!(
            surface.use_aliases, 0,
            "`{name}` gained a constructor import alias"
        );
        assert_eq!(
            surface.fixup_references, 1,
            "`{name}` fixup authority gained another caller"
        );
        assert_eq!(
            fixup_references(function(&syntax, recovery), name),
            1,
            "`{name}` fixup authority escaped its validated recovery adapter"
        );
    }
}
