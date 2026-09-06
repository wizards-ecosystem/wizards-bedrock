//! Source contracts for state-selecting production arguments.

use std::path::PathBuf;

use syn::visit::{self, Visit};
use syn::{
    FnArg, GenericArgument, ImplItem, Item, ItemImpl, Pat, PathArguments, ReturnType, Signature,
    Type,
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

fn function<'a>(syntax: &'a syn::File, name: &str) -> &'a Signature {
    syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == name => Some(&function.sig),
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing function `{name}`"))
}

fn method<'a>(syntax: &'a syn::File, owner: &str, name: &str) -> &'a Signature {
    optional_method(syntax, owner, name).unwrap_or_else(|| panic!("missing `{owner}::{name}`"))
}

struct MethodCollector<'ast, 'name> {
    owner: &'name str,
    name: &'name str,
    methods: Vec<&'ast Signature>,
}

impl<'ast> Visit<'ast> for MethodCollector<'ast, '_> {
    fn visit_item_impl(&mut self, implementation: &'ast ItemImpl) {
        if implementation.trait_.is_none() && type_is(&implementation.self_ty, self.owner) {
            for item in &implementation.items {
                if let ImplItem::Fn(method) = item
                    && method.sig.ident == self.name
                {
                    self.methods.push(&method.sig);
                }
            }
        }
        visit::visit_item_impl(self, implementation);
    }
}

fn optional_method<'a>(syntax: &'a syn::File, owner: &str, name: &str) -> Option<&'a Signature> {
    let mut collector = MethodCollector {
        owner,
        name,
        methods: Vec::new(),
    };
    collector.visit_file(syntax);
    assert!(collector.methods.len() <= 1, "`{owner}::{name}` count");
    collector.methods.first().copied()
}

fn type_is(ty: &Type, expected: &str) -> bool {
    matches!(ty, Type::Path(path)
        if path.qself.is_none()
            && path.path.segments.last().is_some_and(|segment| segment.ident == expected))
}

fn one_parameter_type_is(ty: &Type, outer: &str, inner: &str) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    segment.ident == outer
        && arguments.args.len() == 1
        && matches!(arguments.args.first(), Some(GenericArgument::Type(ty)) if type_is(ty, inner))
}

fn one_parameter_shared_reference_type_is(ty: &Type, outer: &str, inner: &str) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return false;
    };
    segment.ident == outer
        && arguments.args.len() == 1
        && matches!(arguments.args.first(), Some(GenericArgument::Type(Type::Reference(reference)))
            if reference.mutability.is_none() && type_is(&reference.elem, inner))
}

fn parameter<'a>(signature: &'a Signature, name: &str) -> &'a Type {
    signature
        .inputs
        .iter()
        .find_map(|argument| match argument {
            FnArg::Typed(argument)
                if matches!(&*argument.pat, Pat::Ident(identifier) if identifier.ident == name) =>
            {
                Some(&*argument.ty)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing `{name}` parameter on `{}`", signature.ident))
}

fn assert_named(signature: &Signature, parameter_name: &str, type_name: &str) {
    let ty = parameter(signature, parameter_name);
    assert!(
        type_is(ty, type_name),
        "`{}::{parameter_name}` must use `{type_name}`",
        signature.ident
    );
    assert!(!type_is(ty, "bool"));
}

fn assert_no_typed_parameters(signature: &Signature) {
    assert!(
        signature
            .inputs
            .iter()
            .all(|argument| matches!(argument, FnArg::Receiver(_))),
        "`{}` must expose only its receiver",
        signature.ident
    );
}

#[test]
fn selected_state_selectors_use_semantic_types() {
    let pci_config = parse("libs/pci-config/src/lib.rs");
    assert_named(
        function(&pci_config, "memory_window_size"),
        "width",
        "MemoryWidth",
    );

    let nvme = parse("libs/nvme/src/lib.rs");
    assert_no_typed_parameters(method(&nvme, "DmaState", "close"));
    for forbidden in ["quiesce", "retain_unquiesced", "is_quiesced", "must_retain"] {
        assert!(
            optional_method(&nvme, "DmaState", forbidden).is_none(),
            "pure DmaState recovered hardware authority through `{forbidden}`"
        );
    }
    assert_named(function(&nvme, "doorbell"), "doorbell", "Doorbell");

    let xstate = parse("libs/x86-cpuid/src/xstate.rs");
    assert_named(
        method(&xstate, "Component", "new"),
        "alignment",
        "Alignment",
    );

    let sync = parse("wost/src/sync.rs");
    assert_named(method(&sync, "SpinLock", "acquire"), "kind", "LockKind");
    let percpu = parse("wost/src/arch/x86_64/percpu.rs");
    assert_named(method(&percpu, "PerCpu", "took_lock"), "kind", "LockKind");
    assert_named(
        method(&percpu, "PerCpu", "released_lock"),
        "kind",
        "LockKind",
    );

    let pci = parse("kernel/src/pci.rs");
    assert_named(function(&pci, "claim_as"), "lifetime", "ClaimLifetime");

    let main = parse("kernel/src/main.rs");
    let start = function(&main, "start_interrupts");
    assert!(one_parameter_shared_reference_type_is(
        parameter(start, "external_controllers"),
        "Option",
        "ExternalControllersMasked",
    ));
    assert!(matches!(start.output, ReturnType::Default));
    assert!(one_parameter_shared_reference_type_is(
        parameter(function(&main, "external_interrupts_are_admitted"), "mask"),
        "Option",
        "ExternalControllersMasked",
    ));
    let route = function(&main, "route_keyboard");
    assert!(
        one_parameter_type_is(
            parameter(route, "routing"),
            "Option",
            "ExternalControllersMasked",
        ),
        "`route_keyboard::routing` must consume masked-controller authority"
    );
    assert_eq!(route.inputs.len(), 1);

    let input = parse("kernel/src/input.rs");
    assert_named(
        function(&input, "set_translation"),
        "translation",
        "Translation",
    );
    let i8042 = parse("kernel/src/arch/x86_64/i8042.rs");
    assert_named(
        method(&i8042, "Controller", "set_translation"),
        "translation",
        "Translation",
    );

    let kernel_nvme = parse("kernel/src/nvme.rs");
    assert_named(
        function(&kernel_nvme, "wait_for_ready"),
        "wanted",
        "Readiness",
    );

    let thread = parse("kernel/src/thread.rs");
    assert_named(
        method(&thread, "RunQueue", "choose"),
        "eligibility",
        "CurrentEligibility",
    );

    let console = parse("kernel/src/console.rs");
    assert_no_typed_parameters(method(&console, "Console", "show_cursor"));
    assert_no_typed_parameters(method(&console, "Console", "hide_cursor"));
    assert!(
        optional_method(&console, "Console", "set_cursor").is_none(),
        "cursor visibility must remain two named operations"
    );
}

#[test]
fn trait_methods_do_not_satisfy_inherent_api_contracts() {
    let syntax = syn::parse_file(
        "struct Boundary; trait Cosmetic { fn close(&mut self); } \
         impl Cosmetic for Boundary { fn close(&mut self) {} }",
    )
    .expect("valid trait-method witness");
    assert!(optional_method(&syntax, "Boundary", "close").is_none());
}

#[test]
fn nested_inherent_methods_remain_visible_to_api_contracts() {
    let syntax = syn::parse_file(
        "struct Boundary; mod nested { impl super::Boundary { fn close(&mut self) {} } }",
    )
    .expect("valid nested inherent-method witness");
    assert!(optional_method(&syntax, "Boundary", "close").is_some());
}
