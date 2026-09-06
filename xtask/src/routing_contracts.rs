//! Source contracts for external-interrupt mask authority.

use std::path::PathBuf;

use syn::visit::{self, Visit};
use syn::{
    Expr, ExprCall, ExprStruct, Fields, FnArg, ImplItem, Item, ItemFn, ItemImpl, Member, Pat,
    PathArguments, ReturnType, Stmt, Type, Visibility,
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

fn rust_sources_below(relative: &str) -> Vec<(PathBuf, syn::File)> {
    fn collect(directory: &std::path::Path, paths: &mut Vec<PathBuf>) {
        let mut entries = crate::paths::directory_entries(directory)
            .unwrap_or_else(|error| panic!("reading {}: {error}", directory.display()));
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if entry.is_dir() {
                collect(&path, paths);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                paths.push(path);
            }
        }
    }

    let directory = root().join(relative);
    let mut paths = Vec::new();
    collect(&directory, &mut paths);
    paths
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
            let syntax = syn::parse_file(&source)
                .unwrap_or_else(|error| panic!("parsing {}: {error}", path.display()));
            (path, syntax)
        })
        .collect()
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

fn type_is(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(path)
        if path.qself.is_none() && path_has_suffix(&path.path, &[name]))
}

fn option_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    if segment.ident != "Option" || arguments.args.len() != 1 {
        return None;
    }
    match arguments.args.first()? {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

fn option_of(ty: &Type, inner: &str) -> bool {
    option_inner(ty).is_some_and(|ty| type_is(ty, inner))
}

fn option_of_shared_reference(ty: &Type, inner: &str) -> bool {
    matches!(option_inner(ty), Some(Type::Reference(reference))
        if reference.mutability.is_none() && type_is(&reference.elem, inner))
}

fn is_pub_crate(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Restricted(restricted) if restricted.path.is_ident("crate"))
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

fn parameter<'a>(function: &'a ItemFn, name: &str) -> &'a Type {
    function
        .sig
        .inputs
        .iter()
        .find_map(|argument| match argument {
            FnArg::Typed(argument)
                if matches!(&*argument.pat, Pat::Ident(binding) if binding.ident == name) =>
            {
                Some(&*argument.ty)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing parameter `{name}` on `{}`", function.sig.ident))
}

struct MethodCollector<'ast, 'name> {
    owner: &'name str,
    name: &'name str,
    methods: Vec<&'ast syn::ImplItemFn>,
}

impl<'ast> Visit<'ast> for MethodCollector<'ast, '_> {
    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        if implementation.trait_.is_none() && type_is(&implementation.self_ty, self.owner) {
            for item in &implementation.items {
                if let ImplItem::Fn(method) = item
                    && method.sig.ident == self.name
                {
                    self.methods.push(method);
                }
            }
        }
        visit::visit_item_impl(self, implementation);
    }
}

fn method<'a>(syntax: &'a syn::File, owner: &str, name: &str) -> &'a syn::ImplItemFn {
    let mut collector = MethodCollector {
        owner,
        name,
        methods: Vec::new(),
    };
    collector.visit_file(syntax);
    assert_eq!(collector.methods.len(), 1, "`{owner}::{name}` count");
    collector.methods[0]
}

fn optional_method<'a>(
    syntax: &'a syn::File,
    owner: &str,
    name: &str,
) -> Option<&'a syn::ImplItemFn> {
    let mut collector = MethodCollector {
        owner,
        name,
        methods: Vec::new(),
    };
    collector.visit_file(syntax);
    assert!(collector.methods.len() <= 1, "`{owner}::{name}` count");
    collector.methods.first().copied()
}

struct Implementations<'ast, 'name> {
    owner: &'name str,
    items: Vec<&'ast ItemImpl>,
}

impl<'ast> Visit<'ast> for Implementations<'ast, '_> {
    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        if type_is(&implementation.self_ty, self.owner) {
            self.items.push(implementation);
        }
        visit::visit_item_impl(self, implementation);
    }
}

fn implementations<'a>(syntax: &'a syn::File, owner: &str) -> Vec<&'a ItemImpl> {
    let mut implementations = Implementations {
        owner,
        items: Vec::new(),
    };
    implementations.visit_file(syntax);
    implementations.items
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

fn exact_derives(structure: &syn::ItemStruct, expected: &[&str]) -> bool {
    let mut derives = Vec::new();
    for attribute in structure
        .attrs
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
    fn renamed_authorities(tree: &syn::UseTree, names: &[&str]) -> usize {
        match tree {
            syn::UseTree::Path(path) => renamed_authorities(&path.tree, names),
            syn::UseTree::Rename(rename) if names.iter().any(|name| rename.ident == *name) => 1,
            syn::UseTree::Group(group) => group
                .items
                .iter()
                .map(|tree| renamed_authorities(tree, names))
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
            if self.names.iter().any(|name| type_is(&alias.ty, name)) {
                self.count += 1;
            }
            visit::visit_item_type(self, alias);
        }

        fn visit_item_use(&mut self, import: &'ast syn::ItemUse) {
            self.count += renamed_authorities(&import.tree, self.names);
            visit::visit_item_use(self, import);
        }
    }

    let mut aliases = Aliases { names, count: 0 };
    aliases.visit_file(syntax);
    aliases.count
}

struct Constructions<'name> {
    name: &'name str,
    count: usize,
}

impl<'ast> Visit<'ast> for Constructions<'_> {
    fn visit_expr_struct(&mut self, expression: &'ast ExprStruct) {
        if path_has_suffix(&expression.path, &[self.name]) {
            self.count += 1;
        }
        visit::visit_expr_struct(self, expression);
    }
}

fn constructions(name: &str, node: impl FnOnce(&mut Constructions<'_>)) -> usize {
    let mut constructions = Constructions { name, count: 0 };
    node(&mut constructions);
    constructions.count
}

fn receiver_only(method: &syn::ImplItemFn) -> bool {
    matches!(method.sig.inputs.first(), Some(FnArg::Receiver(receiver))
        if receiver.reference.is_none())
        && method.sig.inputs.len() == 1
}

fn returns_option(method: &syn::ImplItemFn, inner: &str) -> bool {
    matches!(&method.sig.output, ReturnType::Type(_, ty) if option_of(ty, inner))
}

fn transition_is_exact_mask_gate(method: &syn::ImplItemFn) -> bool {
    let [
        Stmt::Expr(Expr::If(refusal), _),
        Stmt::Expr(Expr::Call(admitted), None),
    ] = method.block.stmts.as_slice()
    else {
        return false;
    };
    matches!(&*refusal.cond, Expr::Unary(not)
        if matches!(not.op, syn::UnOp::Not(_))
            && matches!(&*not.expr, Expr::Field(field)
                if matches!(&*field.base, Expr::Path(path) if path.path.is_ident("self"))
                    && matches!(&field.member, Member::Named(name)
                        if name == "controllers_masked")))
        && matches!(refusal.then_branch.stmts.as_slice(),
            [Stmt::Expr(Expr::Return(returned), _)]
                if matches!(returned.expr.as_deref(), Some(Expr::Path(path))
                    if path.path.is_ident("None")))
        && matches!(&*admitted.func, Expr::Path(path) if path.path.is_ident("Some"))
        && matches!(admitted.args.first(), Some(Expr::Struct(value))
            if path_has_suffix(&value.path, &["ExternalControllersMasked"])
                && value.fields.len() == 1
                && matches!(value.fields.first(), Some(field)
                    if matches!(&field.member, Member::Named(name) if name == "routing")
                        && matches!(&field.expr, Expr::Field(routing)
                            if matches!(&*routing.base, Expr::Path(path)
                                if path.path.is_ident("self"))
                                && matches!(&routing.member, Member::Named(name)
                                    if name == "routing"))))
}

fn sole_expression(method: &syn::ImplItemFn) -> Option<&Expr> {
    match method.block.stmts.as_slice() {
        [Stmt::Expr(expression, None)] => Some(expression),
        _ => None,
    }
}

fn preparation_constructor_is(
    method: &syn::ImplItemFn,
    has_routing: bool,
    controllers_masked: bool,
) -> bool {
    let Some(Expr::Struct(construction)) = sole_expression(method) else {
        return false;
    };
    if !construction.path.is_ident("Self") || construction.fields.len() != 2 {
        return false;
    }
    let field = |name: &str| {
        construction
            .fields
            .iter()
            .find(|field| matches!(&field.member, Member::Named(member) if member == name))
    };
    let Some(routing) = field("routing") else {
        return false;
    };
    let Some(masked) = field("controllers_masked") else {
        return false;
    };
    let routing_matches = if has_routing {
        matches!(&routing.expr, Expr::Call(call)
            if matches!(&*call.func, Expr::Path(path) if path.path.is_ident("Some"))
                && call.args.len() == 1
                && matches!(call.args.first(), Some(Expr::Path(path))
                    if path.path.is_ident("routing")))
    } else {
        matches!(&routing.expr, Expr::Path(path) if path.path.is_ident("None"))
    };
    routing_matches
        && matches!(&masked.expr, Expr::Lit(literal)
            if matches!(&literal.lit, syn::Lit::Bool(value)
                if value.value == controllers_masked))
}

fn unit_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn polled_safe_requires_release(method: &syn::ImplItemFn) -> bool {
    method.sig.inputs.len() == 1
        && matches!(method.sig.inputs.first(), Some(FnArg::Typed(argument))
        if matches!(&*argument.pat, Pat::Ident(binding)
            if binding.ident == "_released" && binding.mutability.is_none())
            && type_is(&argument.ty, "ControllersReleased"))
}

fn constructs_release(expression: &Expr) -> bool {
    matches!(expression, Expr::Call(call)
        if matches!(&*call.func, Expr::Path(path) if path.path.is_ident("Some"))
            && call.args.len() == 1
            && matches!(call.args.first(), Some(Expr::Struct(construction))
                if path_has_suffix(&construction.path, &["ControllersReleased"])
                    && construction.fields.len() == 1
                    && matches!(construction.fields.first(), Some(field)
                        if matches!(&field.member, Member::Named(name) if name == "_private")
                            && matches!(&field.expr, Expr::Tuple(tuple)
                                if tuple.elems.is_empty()))))
}

fn release_tail_is_exact(function: &ItemFn) -> bool {
    let [
        Stmt::Expr(Expr::While(releases), None),
        Stmt::Expr(success, None),
    ] = function.block.stmts.as_slice()
    else {
        return false;
    };
    if !matches!(&*releases.cond, Expr::Let(binding)
        if matches!(&*binding.pat, Pat::TupleStruct(pattern)
            if pattern.path.is_ident("Some")
                && matches!(pattern.elems.first(), Some(Pat::Ident(name))
                    if name.ident == "controller"))
            && matches!(&*binding.expr, Expr::MethodCall(call)
                if call.method == "pop"
                    && call.args.is_empty()
                    && matches!(&*call.receiver, Expr::Path(path)
                        if path.path.is_ident("controllers"))))
    {
        return false;
    }
    let [Stmt::Expr(Expr::If(failure), None)] = releases.body.stmts.as_slice() else {
        return false;
    };
    if failure.else_branch.is_some()
        || !matches!(&*failure.cond, Expr::Let(binding)
            if matches!(&*binding.pat, Pat::TupleStruct(pattern)
                if path_has_suffix(&pattern.path, &["DeviceRetirement", "Retained"])
                    && matches!(pattern.elems.first(), Some(Pat::Ident(name))
                        if name.ident == "registers"))
                && matches!(&*binding.expr, Expr::Call(call)
                    if matches!(&*call.func, Expr::Path(path)
                        if path.path.is_ident("unmap_device_exact"))
                        && call.args.len() == 2
                        && matches!(call.args.first(), Some(Expr::Path(path))
                            if path.path.is_ident("context"))
                        && matches!(call.args.last(), Some(Expr::MethodCall(method))
                            if method.method == "into_releasable"
                                && method.args.is_empty()
                                && matches!(&*method.receiver, Expr::Path(path)
                                    if path.path.is_ident("controller")))))
    {
        return false;
    }
    let [
        warning,
        Stmt::Expr(Expr::Call(retain), Some(_)),
        Stmt::Expr(Expr::Return(returned), Some(_)),
    ] = failure.then_branch.stmts.as_slice()
    else {
        return false;
    };
    matches!(warning, Stmt::Macro(statement) if statement.mac.path.is_ident("warn"))
        && matches!(&*retain.func, Expr::Path(path) if path.path.is_ident("retain_routing_registers"))
        && retain.args.len() == 2
        && matches!(retain.args.first(), Some(Expr::Path(path))
            if path.path.is_ident("registers"))
        && matches!(retain.args.last(), Some(Expr::Call(take))
            if matches!(&*take.func, Expr::Path(path)
                if path_has_suffix(&path.path, &["mem", "take"]))
                && matches!(take.args.first(), Some(Expr::Path(path))
                    if path.path.is_ident("controllers")))
        && matches!(returned.expr.as_deref(), Some(Expr::Path(path))
            if path.path.is_ident("None"))
        && constructs_release(success)
}

#[derive(Default)]
struct CallCount {
    interrupt_enable: usize,
    interrupt_enable_references: usize,
    interrupt_enable_imports: usize,
    interrupts_enabled_query: usize,
}

impl<'ast> Visit<'ast> for CallCount {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if matches!(&*call.func, Expr::Path(path)
            if path_has_suffix(&path.path, &["interrupt", "enable"]))
        {
            self.interrupt_enable += 1;
        }
        if matches!(&*call.func, Expr::Path(path)
            if path_has_suffix(&path.path, &["interrupt", "are_enabled"]))
        {
            self.interrupts_enabled_query += 1;
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        if path_has_suffix(&expression.path, &["interrupt", "enable"]) {
            self.interrupt_enable_references += 1;
        }
        visit::visit_expr_path(self, expression);
    }

    fn visit_item_use(&mut self, import: &'ast syn::ItemUse) {
        fn count(tree: &syn::UseTree, prefix: &mut Vec<String>) -> usize {
            match tree {
                syn::UseTree::Path(path) => {
                    prefix.push(path.ident.to_string());
                    let found = count(&path.tree, prefix);
                    prefix.pop();
                    found
                }
                syn::UseTree::Name(name) => usize::from(
                    name.ident == "enable"
                        && prefix.last().is_some_and(|segment| segment == "interrupt"),
                ),
                syn::UseTree::Rename(rename) => usize::from(
                    rename.ident == "enable"
                        && prefix.last().is_some_and(|segment| segment == "interrupt"),
                ),
                syn::UseTree::Glob(_) => {
                    usize::from(prefix.last().is_some_and(|segment| segment == "interrupt"))
                }
                syn::UseTree::Group(group) => {
                    group.items.iter().map(|tree| count(tree, prefix)).sum()
                }
            }
        }

        self.interrupt_enable_imports += count(&import.tree, &mut Vec::new());
        visit::visit_item_use(self, import);
    }
}

fn route_uses_live_interrupt_guard(function: &ItemFn) -> bool {
    let Some(Stmt::Local(binding)) = function.block.stmts.get(1) else {
        return false;
    };
    let Pat::Ident(name) = &binding.pat else {
        return false;
    };
    if name.ident != "routing" || name.mutability.is_some() {
        return false;
    }
    let Some(initializer) = &binding.init else {
        return false;
    };
    let Expr::Match(routing_match) = &*initializer.expr else {
        return false;
    };
    if !matches!(&*routing_match.expr, Expr::MethodCall(call)
        if call.method == "into_routing"
            && call.args.is_empty()
            && matches!(&*call.receiver, Expr::Path(path) if path.path.is_ident("routing")))
    {
        return false;
    }
    let Some(first) = routing_match.arms.first() else {
        return false;
    };
    matches!(&first.pat, Pat::TupleStruct(pattern)
        if pattern.path.is_ident("Some")
            && matches!(pattern.elems.first(), Some(Pat::Ident(binding))
                if binding.ident == "routing"))
        && matches!(&first.guard, Some((_, guard))
            if matches!(&**guard, Expr::Call(call)
            if matches!(&*call.func, Expr::Path(path)
                if path_has_suffix(&path.path, &["interrupt", "are_enabled"]))
                && call.args.is_empty()))
        && matches!(&*first.body, Expr::Path(path) if path.path.is_ident("routing"))
}

fn mask_guard(statement: &Stmt) -> bool {
    matches!(statement, Stmt::Expr(Expr::If(branch), _)
        if matches!(&*branch.cond, Expr::Unary(not)
            if matches!(not.op, syn::UnOp::Not(_))
                && matches!(&*not.expr, Expr::Call(call)
                    if matches!(&*call.func, Expr::Path(path)
                        if path.path.is_ident("external_interrupts_are_admitted"))
                        && call.args.len() == 1
                        && matches!(call.args.first(), Some(Expr::Path(path))
                            if path.path.is_ident("external_controllers"))))
            && branch.then_branch.stmts.iter().any(|statement|
                matches!(statement, Stmt::Expr(Expr::Return(_), _))))
}

#[test]
fn external_controller_mask_authority_is_private_linear_and_exact() {
    let syntax = parse("kernel/src/mem.rs");
    let preparation = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(structure) if structure.ident == "RoutingPreparation" => Some(structure),
            _ => None,
        })
        .expect("missing routing preparation state");
    let proof = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(structure) if structure.ident == "ExternalControllersMasked" => {
                Some(structure)
            }
            _ => None,
        })
        .expect("missing external-controller mask authority");
    let released = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(structure) if structure.ident == "ControllersReleased" => Some(structure),
            _ => None,
        })
        .expect("missing controller-release evidence");
    assert!(is_pub_crate(&preparation.vis));
    assert!(is_pub_crate(&proof.vis));
    assert!(matches!(released.vis, Visibility::Inherited));
    let Fields::Named(preparation_fields) = &preparation.fields else {
        panic!("routing preparation must name its state");
    };
    assert_eq!(preparation_fields.named.len(), 2);
    let routing = preparation_fields
        .named
        .iter()
        .find(|field| matches!(&field.ident, Some(name) if name == "routing"))
        .expect("routing preparation owner");
    assert!(matches!(routing.vis, Visibility::Inherited));
    assert!(option_of(&routing.ty, "Routing"));
    let masked = preparation_fields
        .named
        .iter()
        .find(|field| matches!(&field.ident, Some(name) if name == "controllers_masked"))
        .expect("routing mask state");
    assert!(matches!(masked.vis, Visibility::Inherited));
    assert!(type_is(&masked.ty, "bool"));
    let Fields::Named(fields) = &proof.fields else {
        panic!("external-controller mask authority must name its routing owner");
    };
    assert_eq!(fields.named.len(), 1);
    let field = fields.named.first().expect("routing field");
    assert!(matches!(field.vis, Visibility::Inherited));
    assert!(matches!(&field.ident, Some(name) if name == "routing"));
    assert!(option_of(&field.ty, "Routing"));
    let Fields::Named(released_fields) = &released.fields else {
        panic!("controller-release evidence must have one private field");
    };
    assert_eq!(released_fields.named.len(), 1);
    let released_field = released_fields
        .named
        .first()
        .expect("release evidence field");
    assert!(matches!(released_field.vis, Visibility::Inherited));
    assert!(matches!(&released_field.ident, Some(name) if name == "_private"));
    assert!(unit_type(&released_field.ty));
    assert!(exact_derives(preparation, &["Debug"]));
    assert!(exact_derives(proof, &["Debug"]));
    assert!(exact_derives(released, &[]));
    assert!(exact_inherent_methods(
        &syntax,
        "RoutingPreparation",
        &["into_masked", "polled_safe", "ready", "unproven"]
    ));
    assert!(exact_inherent_methods(
        &syntax,
        "ExternalControllersMasked",
        &["into_routing"]
    ));
    assert!(implementations(&syntax, "ControllersReleased").is_empty());

    assert!(preparation_constructor_is(
        method(&syntax, "RoutingPreparation", "ready"),
        true,
        true
    ));
    let polled_safe = method(&syntax, "RoutingPreparation", "polled_safe");
    assert!(polled_safe_requires_release(polled_safe));
    assert!(preparation_constructor_is(polled_safe, false, true));
    assert!(preparation_constructor_is(
        method(&syntax, "RoutingPreparation", "unproven"),
        false,
        false
    ));

    let into_masked = method(&syntax, "RoutingPreparation", "into_masked");
    assert!(receiver_only(into_masked));
    assert!(returns_option(into_masked, "ExternalControllersMasked"));
    assert!(transition_is_exact_mask_gate(into_masked));
    assert!(optional_method(&syntax, "RoutingPreparation", "permits_interrupts").is_none());
    assert!(optional_method(&syntax, "RoutingPreparation", "into_routing").is_none());

    let into_routing = method(&syntax, "ExternalControllersMasked", "into_routing");
    assert!(receiver_only(into_routing));
    assert!(returns_option(into_routing, "Routing"));
    assert!(
        matches!(sole_expression(into_routing), Some(Expr::Field(field))
        if matches!(&*field.base, Expr::Path(path) if path.path.is_ident("self"))
            && matches!(&field.member, Member::Named(name) if name == "routing"))
    );

    assert_eq!(
        constructions("ExternalControllersMasked", |visitor| visitor
            .visit_file(&syntax)),
        1
    );
    assert_eq!(
        constructions("ExternalControllersMasked", |visitor| {
            Visit::visit_impl_item_fn(visitor, into_masked)
        }),
        1,
        "mask authority escaped its checked state transition"
    );
    assert_eq!(
        constructions("RoutingPreparation", |visitor| visitor.visit_file(&syntax)),
        0,
        "routing preparation gained a raw constructor"
    );
    assert_eq!(
        authority_alias_count(
            &syntax,
            &[
                "RoutingPreparation",
                "ExternalControllersMasked",
                "ControllersReleased"
            ]
        ),
        0,
        "routing authority gained a constructor alias"
    );

    let release = function(&syntax, "release_io_apics");
    assert!(matches!(&release.sig.output, ReturnType::Type(_, ty)
        if option_of(ty, "ControllersReleased")));
    assert!(release_tail_is_exact(release));
    assert_eq!(
        constructions("ControllersReleased", |visitor| visitor.visit_file(&syntax)),
        1
    );
    assert_eq!(
        constructions("ControllersReleased", |visitor| {
            Visit::visit_item_fn(visitor, release)
        }),
        1,
        "controller-release evidence escaped exact cleanup"
    );
}

#[test]
fn interrupt_enable_and_routing_require_the_mask_authority() {
    let syntax = parse("kernel/src/main.rs");
    assert!(syntax.items.iter().all(|item| !matches!(item,
        Item::Enum(item) if item.ident == "ExternalControllerMask")));
    assert!(syntax.items.iter().all(|item| !matches!(item,
        Item::Struct(item) if item.ident == "InterruptsStarted")));
    assert_eq!(
        constructions("ExternalControllersMasked", |visitor| visitor
            .visit_file(&syntax)),
        0
    );

    let start = function(&syntax, "start_interrupts");
    assert!(option_of_shared_reference(
        parameter(start, "external_controllers"),
        "ExternalControllersMasked"
    ));
    assert!(matches!(start.sig.output, ReturnType::Default));
    let guard = start
        .block
        .stmts
        .iter()
        .position(mask_guard)
        .expect("missing mask-authority refusal");
    let mut calls = CallCount::default();
    Visit::visit_item_fn(&mut calls, start);
    assert_eq!(calls.interrupt_enable, 1);
    let enable = start
        .block
        .stmts
        .iter()
        .position(|statement| {
            let mut calls = CallCount::default();
            calls.visit_stmt(statement);
            calls.interrupt_enable == 1
        })
        .expect("missing interrupt enable");
    assert!(
        guard < enable,
        "interrupt enable moved before mask admission"
    );

    let admitted = function(&syntax, "external_interrupts_are_admitted");
    assert!(option_of_shared_reference(
        parameter(admitted, "mask"),
        "ExternalControllersMasked"
    ));
    assert!(matches!(admitted.block.stmts.as_slice(),
        [Stmt::Expr(Expr::MethodCall(call), None)]
            if call.method == "is_some"
                && call.args.is_empty()
                && matches!(&*call.receiver, Expr::Path(path) if path.path.is_ident("mask"))));

    let route = function(&syntax, "route_keyboard");
    assert!(option_of(
        parameter(route, "routing"),
        "ExternalControllersMasked"
    ));
    let mut route_calls = CallCount::default();
    Visit::visit_item_fn(&mut route_calls, route);
    assert_eq!(route_calls.interrupts_enabled_query, 1);
    assert!(route_uses_live_interrupt_guard(route));

    let mut admission_sites = Vec::new();
    let mut admission_references = 0;
    let mut admission_imports = 0;
    for (path, syntax) in rust_sources_below("kernel/src")
        .into_iter()
        .chain(rust_sources_below("wost/src"))
    {
        let mut calls = CallCount::default();
        calls.visit_file(&syntax);
        admission_references += calls.interrupt_enable_references;
        admission_imports += calls.interrupt_enable_imports;
        if calls.interrupt_enable != 0 {
            let relative = path
                .strip_prefix(root())
                .expect("source below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            admission_sites.push((relative, calls.interrupt_enable));
        }
    }
    assert_eq!(
        admission_sites,
        [
            ("kernel/src/main.rs".to_owned(), 1),
            ("wost/src/arch/x86_64/task.rs".to_owned(), 1)
        ],
        "interrupt admission escaped its two audited authority paths"
    );
    assert_eq!(
        admission_references, 2,
        "interrupt admission gained an indirect function reference"
    );
    assert_eq!(
        admission_imports, 0,
        "interrupt admission gained an import alias or glob"
    );
}
