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

fn source(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("reading {relative}: {error}"))
}

fn parse(relative: &str) -> syn::File {
    let source = source(relative);
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

fn integer_literal_is(expression: &Expr, value: u64) -> bool {
    matches!(expression, Expr::Lit(literal)
        if matches!(&literal.lit, syn::Lit::Int(integer)
            if integer.base10_parse::<u64>().is_ok_and(|parsed| parsed == value)))
}

fn bool_literal_is(expression: &Expr, value: bool) -> bool {
    matches!(expression, Expr::Lit(literal)
        if matches!(&literal.lit, syn::Lit::Bool(boolean) if boolean.value == value))
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

#[test]
fn memory_initialization_requires_the_exhaustive_entry_dma_boundary() {
    let main = source("kernel/src/main.rs");
    let boot = main
        .split_once("fn kernel_main(boot: BootInfo) -> ! {")
        .expect("kernel entry")
        .1
        .split_once("struct Handoff")
        .expect("kernel entry end")
        .0;
    let proof = boot
        .find("let early_dma = iommu::secure_handoff(&boot);")
        .expect("entry DMA proof");
    let console = boot
        .find("let serial = dev::init_uart();")
        .expect("UART setup");
    let memory = boot
        .find("mem::init(boot.memory_map, boot.direct_map, early_dma);")
        .expect("proof-consuming memory initialization");
    assert!(proof < console && console < memory);

    let iommu = source("kernel/src/iommu.rs");
    let boundary = iommu
        .split_once("pub(crate) fn secure_handoff")
        .expect("secure handoff")
        .1
        .split_once("static DMAR")
        .expect("secure handoff end")
        .0;
    let platform = boundary
        .find("cpu::dma_handoff_platform().ok_or(EarlyDmaError::UnauthenticatedPlatform)?")
        .expect("requester-inaccessible platform evidence");
    assert!(boundary.contains("cpu::DmaHandoffPlatform::QemuTcg => HandoffFencePolicy::strict()"));
    assert!(
        boundary.contains("cpu::DmaHandoffPlatform::Vmware => HandoffFencePolicy::vmware_440bx()")
    );
    let table = boundary
        .find("acpi::find_unique_early")
        .expect("unique early handoff table");
    let ports = boundary.find("platform_pci::init()").expect("port claim");
    let fence = boundary
        .find("platform_pci::fence_dma_handoff(policy)")
        .expect("exhaustive requester fence");
    let ordering = boundary
        .find("compiler_fence(Ordering::SeqCst)")
        .expect("compiler ordering after requester fence");
    assert!(platform < table && table < ports && ports < fence && fence < ordering);
    let iommu_syntax = parse("kernel/src/iommu.rs");
    let token = iommu_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "EarlyDmaBoundary" => Some(item),
            _ => None,
        })
        .expect("entry DMA proof token");
    assert!(matches!(token.vis, Visibility::Restricted(_)));
    assert!(exact_derives(&token.attrs, &["Debug"]));
    assert!(!iommu.contains("impl Clone for EarlyDmaBoundary"));
    assert!(!iommu.contains("impl Copy for EarlyDmaBoundary"));

    let acpi = source("kernel/src/acpi.rs");
    let unique = acpi
        .split_once("pub(crate) fn find_unique_early")
        .expect("allocation-free unique table lookup")
        .1
        .split_once("pub(crate) type Interrupts")
        .expect("allocation-free unique table lookup end")
        .0;
    assert!(unique.contains("let mut root_table = [0u8; EARLY_ROOT_BYTES];"));
    assert!(unique.contains("return Err(EarlyTableError::RootEntry);"));
    assert!(unique.contains("if found.replace(table).is_some()"));
    assert!(unique.contains("return Err(EarlyTableError::Duplicate);"));

    let memory = source("kernel/src/mem.rs");
    let init = memory
        .split_once("pub(crate) fn init(")
        .expect("memory initialization")
        .1
        .split_once(") {")
        .expect("memory initialization signature")
        .0;
    assert!(init.contains("_dma: EarlyDmaBoundary"));

    let pci = source("libs/pci-config/src/lib.rs");
    let fence = pci
        .split_once("pub fn fence_dma_handoff(")
        .expect("pure entry fence")
        .1
        .split_once("pub struct CommandState")
        .expect("pure entry fence end")
        .0;
    assert!(fence.contains("for bus in 0..=u8::MAX"));
    assert!(fence.contains("for device in 0..DEVICES"));
    assert!(fence.contains("for function in 0..FUNCTIONS"));
    assert_eq!(fence.matches("HandoffAccess::ReadIdentity").count(), 4);
    assert_eq!(fence.matches("HandoffAccess::ReadClass").count(), 4);
    assert!(fence.contains("let expected = before & !command::BUS_MASTER;"));
    assert!(fence.contains("if found != expected"));
    assert!(fence.contains(".classify_non_requester(at, identity, class, before, found)"));
    assert!(fence.contains("if current != identity"));
    assert!(fence.contains("if current_class != class"));

    let profile = pci
        .split_once("fn classify_non_requester(")
        .expect("qualified non-requester profile")
        .1
        .split_once("/// What an exhaustive entry-time DMA fence observed")
        .expect("qualified non-requester profile end")
        .0;
    for exact in [
        "at.bus() == 0",
        "at.device() == 0",
        "at.function() == 0",
        "identity == VMWARE_440BX_IDENTITY",
        "class & CLASS_WITHOUT_REVISION == HOST_BRIDGE_CLASS",
        "before == VMWARE_HOST_COMMAND",
        "found == VMWARE_HOST_COMMAND",
    ] {
        assert!(profile.contains(exact), "missing `{exact}`");
    }
    assert!(
        !pci.contains("pub fn classify_non_requester("),
        "only the exhaustive entry fence may mint non-requester authority"
    );
    assert!(pci.contains("pub fn take_non_requester(&mut self)"));
    let pci_syntax = parse("libs/pci-config/src/lib.rs");
    let non_requester = pci_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "QualifiedNonRequester" => Some(item),
            _ => None,
        })
        .expect("qualified non-requester authority");
    assert!(matches!(non_requester.vis, Visibility::Public(_)));
    assert!(exact_derives(&non_requester.attrs, &["Debug"]));
    assert!(
        non_requester
            .fields
            .iter()
            .all(|field| matches!(field.vis, Visibility::Inherited))
    );
    assert!(!pci.contains("impl Clone for QualifiedNonRequester"));
    assert!(!pci.contains("impl Copy for QualifiedNonRequester"));

    let kernel_pci = source("kernel/src/pci.rs");
    let snapshot = kernel_pci
        .split_once(
            "pub(crate) fn snapshot_topology(\n    mut non_requester: Option<QualifiedNonRequester>,\n    mut quiesced: Option<QuiescedIsaRequester>,\n)",
        )
        .expect("entry-proof-bound topology snapshot")
        .1
        .split_once("/// Whether the complete segment-zero topology passed its rooted-tree proof")
        .expect("policy-bound topology snapshot end")
        .0;
    assert!(!snapshot.contains("classify_non_requester("));
    assert!(snapshot.contains("authority.address() == entry.at"));
    assert!(snapshot.contains("topology.observe_non_requester("));
    assert!(snapshot.contains("quiesced.take_if(|authority| authority.address() == entry.at)"));
    assert!(snapshot.contains("topology.observe_quiesced_requester("));
    assert!(snapshot.contains("if let Some(authority) = quiesced {"));
    assert!(snapshot.contains("TopologyError::QuiescedRequesterMissing"));

    // D-426: the entry-quiesced ISA requester stays in every inventory, so a
    // unit-wide fence must hold it by exact identity, command and a fresh
    // 8237 remask rather than by a BME clear it cannot perform.
    let requester_fence = kernel_pci
        .split_once("pub(crate) fn fence_dma_requester(at: Address) -> DmaFenceOutcome {")
        .expect("sibling requester fence")
        .1
        .split_once("/// One uncertain BAR restoration")
        .expect("sibling requester fence end")
        .0;
    let quiesced_lookup = requester_fence
        .find("TOPOLOGY.lock_leaf().quiesced_requester(at)")
        .expect("quiesced requester lookup");
    let bus_master = requester_fence
        .find("pci::fence_bus_master(&function)")
        .expect("ordinary BME fence");
    assert!(quiesced_lookup < bus_master);
    for exact in [
        "return fence_quiesced_requester(function, record);",
        "identity == Some(record.identity())",
        "command == Some(record.command())",
        "pci::isa_dma_quiet()",
        "DmaFenceOutcome::Refused(DmaFenceError::Quiesce)",
    ] {
        assert!(requester_fence.contains(exact), "missing `{exact}`");
    }
    let isa = pci
        .split_once("fn classify_isa_requester(")
        .expect("quiesced ISA requester profile")
        .1
        .split_once("impl QualifiedNonRequester")
        .expect("quiesced ISA requester profile end")
        .0;
    for exact in [
        "self.quiesces_isa_dma()",
        "(at.bus(), at.device(), at.function()) == (0, 7, 0)",
        "identity == VMWARE_PIIX4_ISA_IDENTITY",
        "class & ISA_CLASS_MASK == ISA_BRIDGE_CLASS",
        "before == VMWARE_ISA_COMMAND",
        "found == VMWARE_ISA_COMMAND",
    ] {
        assert!(isa.contains(exact), "missing `{exact}`");
    }
    assert!(fence.contains(".classify_isa_requester(at, identity, class, before, found)"));
    assert!(fence.contains("status & ISA_DMA_PENDING_REQUESTS != 0"));
    let quiesced_authority = pci_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "QuiescedIsaRequester" => Some(item),
            _ => None,
        })
        .expect("quiesced ISA requester authority");
    assert!(exact_derives(&quiesced_authority.attrs, &["Debug"]));
    assert!(
        quiesced_authority
            .fields
            .iter()
            .all(|field| matches!(field.vis, Visibility::Inherited))
    );
    let wost_pci = source("wost/src/arch/x86_64/pci.rs");
    assert!(
        wost_pci.contains("HandoffAccess::QuiesceIsaDma => isa_dma.as_mut().map(IsaDma::quiesce),")
    );
    let claim = wost_pci
        .find("port::isa_dma_controllers()")
        .expect("8237 claim");
    let lock = wost_pci
        .find("let mut configuration = CONFIGURATION.lock_leaf();\n        if configuration.functions.len() != 0 {")
        .expect("fence configuration lock");
    assert!(
        claim < lock,
        "the 8237 claim precedes the configuration guard"
    );
    assert!(snapshot.contains("if let Some(authority) = non_requester {"));
    assert!(snapshot.contains("TopologyError::QualifiedNonRequesterMissing"));
    assert!(
        kernel_pci.contains(
            "topology_contains_function(at) && TOPOLOGY.lock_leaf().contains_requester(at)"
        )
    );

    let inventory = function(&iommu_syntax, "requester_is_present");
    let mut inventory_paths = ReferencedPaths::default();
    inventory_paths.visit_item_fn(inventory);
    assert!(
        inventory_paths
            .names
            .iter()
            .any(|name| name == "topology_contains_requester")
    );
    assert!(
        inventory_paths
            .names
            .iter()
            .all(|name| name != "topology_contains_function")
    );

    let coverage = function(&iommu_syntax, "covers_every_present_requester");
    let mut coverage_paths = ReferencedPaths::default();
    coverage_paths.visit_item_fn(coverage);
    assert!(
        coverage_paths
            .names
            .iter()
            .any(|name| name == "topology_contains_requester")
    );
    assert!(
        coverage_paths
            .names
            .iter()
            .all(|name| name != "topology_contains_function")
    );

    let main = source("kernel/src/main.rs");
    assert!(main.contains("let dma_non_requester = early_dma.take_non_requester();"));
    assert!(main.contains("let dma_quiesced = early_dma.take_quiesced_requester();"));
    assert!(main.contains("survey_the_bus(dma_non_requester, dma_quiesced);"));
    assert!(main.contains("pci::snapshot_topology(dma_non_requester, dma_quiesced)"));
}

fn isa_port_operation(
    expression: &Expr,
    controller: &str,
    operation: &str,
    values: &[u64],
) -> bool {
    let Expr::MethodCall(call) = expression else {
        return false;
    };
    let Some(types) = &call.turbofish else {
        return false;
    };
    field_chain_is(&call.receiver, "self", &[controller])
        && call.method == operation
        && types.args.len() == 1
        && matches!(types.args.first(), Some(syn::GenericArgument::Type(Type::Path(path)))
            if path.path.is_ident("u8"))
        && call.args.len() == values.len()
        && call
            .args
            .iter()
            .zip(values)
            .all(|(argument, value)| integer_literal_is(argument, *value))
}

fn isa_status_read(statement: &Stmt, controller: &str, offset: u64) -> bool {
    matches!(statement, Stmt::Local(local)
        if matches!(&local.pat, Pat::Ident(binding) if binding.ident == controller)
        && local.init.as_ref().is_some_and(|initializer|
            initializer.diverge.is_none()
                && isa_port_operation(&initializer.expr, controller, "read", &[offset])))
}

fn no_argument_method<'a>(expression: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::MethodCall(call) = expression else {
        return None;
    };
    (call.method == name && call.args.is_empty() && call.turbofish.is_none())
        .then_some(call.receiver.as_ref())
}

fn widened_status(expression: &Expr, name: &str) -> bool {
    matches!(expression, Expr::Call(call)
        if path_expression_is(&call.func, &["u32", "from"])
        && call.args.len() == 1
        && call.args.first().is_some_and(|argument| path_expression_is(argument, &[name])))
}

fn port_block_width(ty: &Type, width: u64) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    let Some(segment) = path.path.segments.last() else {
        return false;
    };
    matches!(&segment.arguments, syn::PathArguments::AngleBracketed(arguments)
        if segment.ident == "PortBlock" && arguments.args.len() == 1
        && matches!(arguments.args.first(), Some(syn::GenericArgument::Const(size))
            if integer_literal_is(size, width)))
}

#[test]
fn isa_dma_adapter_clears_masks_and_rechecks_both_controllers() {
    let pci = parse("wost/src/arch/x86_64/pci.rs");
    let quiesce = method(&pci, "IsaDma", "quiesce").expect("entry 8237 reset");
    let [
        Stmt::Expr(first_clear, Some(_)),
        Stmt::Expr(second_clear, Some(_)),
        Stmt::Expr(remask, None),
    ] = quiesce.block.stmts.as_slice()
    else {
        panic!("entry quiesce must reset both controllers before their remask and status reads");
    };
    assert!(isa_port_operation(
        first_clear,
        "first",
        "write",
        &[0x0d, 0]
    ));
    assert!(isa_port_operation(
        second_clear,
        "second",
        "write",
        &[0x1a, 0]
    ));
    assert!(
        no_argument_method(remask, "reassert")
            .is_some_and(|receiver| path_expression_is(receiver, &["self"]))
    );

    let reassert = method(&pci, "IsaDma", "reassert").expect("8237 remask and status reads");
    let [
        Stmt::Expr(first_mask, Some(_)),
        Stmt::Expr(second_mask, Some(_)),
        first_status,
        second_status,
        Stmt::Expr(Expr::Binary(combined), None),
    ] = reassert.block.stmts.as_slice()
    else {
        panic!("both byte-wide all-mask writes must precede both status reads");
    };
    assert!(isa_port_operation(
        first_mask,
        "first",
        "write",
        &[0x0f, 0x0f]
    ));
    assert!(isa_port_operation(
        second_mask,
        "second",
        "write",
        &[0x1e, 0x0f]
    ));
    assert!(isa_status_read(first_status, "first", 0x08));
    assert!(isa_status_read(second_status, "second", 0x10));
    assert!(matches!(combined.op, syn::BinOp::BitOr(_)));
    assert!(widened_status(&combined.left, "first"));
    assert!(matches!(&*combined.right, Expr::Binary(shift)
        if matches!(shift.op, syn::BinOp::Shl(_))
        && widened_status(&shift.left, "second")
        && integer_literal_is(&shift.right, 8)));

    let quiet = function(&pci, "isa_dma_quiet");
    let [Stmt::Expr(Expr::MethodCall(owned), None)] = quiet.block.stmts.as_slice() else {
        panic!("a quiet result must depend on the retained controller owner");
    };
    assert_eq!(owned.method, "is_some_and");
    let owner = no_argument_method(&owned.receiver, "as_mut")
        .and_then(|receiver| no_argument_method(receiver, "lock_leaf"))
        .expect("the permanent ISA controller owner is locked");
    assert!(path_expression_is(owner, &["ISA_DMA"]));
    assert_eq!(owned.args.len(), 1);
    let Some(Expr::Closure(check)) = owned.args.first() else {
        panic!("the retained owner must perform the fresh quiet check");
    };
    assert_eq!(check.inputs.len(), 1);
    assert!(matches!(check.inputs.first(), Some(Pat::Ident(binding))
        if binding.ident == "controllers"));
    let Expr::Block(body) = &*check.body else {
        panic!("the quiet check must remask before examining pending requests");
    };
    let [Stmt::Expr(Expr::Binary(clear), None)] = body.block.stmts.as_slice() else {
        panic!("the quiet check must return the pending-request predicate");
    };
    assert!(matches!(clear.op, syn::BinOp::Eq(_)));
    assert!(integer_literal_is(&clear.right, 0));
    let Expr::Binary(pending) = &*clear.left else {
        panic!("the quiet check must test both status words");
    };
    assert!(matches!(pending.op, syn::BinOp::BitAnd(_)));
    assert!(path_expression_is(
        &pending.right,
        &["pci_config", "ISA_DMA_PENDING_REQUESTS"]
    ));
    assert!(
        no_argument_method(&pending.left, "reassert")
            .is_some_and(|receiver| path_expression_is(receiver, &["controllers"]))
    );

    let fence = function(&pci, "fence_dma_handoff");
    let [
        Stmt::Local(_),
        Stmt::Local(_),
        Stmt::Expr(Expr::If(retention), None),
        Stmt::Expr(result, None),
    ] = fence.block.stmts.as_slice()
    else {
        panic!("the entry fence must retain its controllers before returning either outcome");
    };
    assert!(path_expression_is(result, &["fenced"]));
    assert!(retention.else_branch.is_none());
    assert!(matches!(&*retention.cond, Expr::Let(binding)
        if path_expression_is(&binding.expr, &["isa_dma"])
        && matches!(&*binding.pat, Pat::TupleStruct(pattern)
            if path_ends(&pattern.path, &["Some"])
            && pattern.elems.len() == 1
            && matches!(pattern.elems.first(), Some(Pat::Ident(name)) if name.ident == "controllers"))));
    let [Stmt::Expr(Expr::Assign(retain), Some(_))] = retention.then_branch.stmts.as_slice() else {
        panic!("both controller capabilities must move to the permanent owner");
    };
    assert!(matches!(&*retain.left, Expr::Unary(place)
        if matches!(place.op, syn::UnOp::Deref(_))
        && no_argument_method(&place.expr, "lock_leaf")
            .is_some_and(|receiver| path_expression_is(receiver, &["ISA_DMA"]))));
    assert!(matches!(&*retain.right, Expr::Call(value)
        if path_expression_is(&value.func, &["Some"])
        && value.args.len() == 1
        && value.args.first().is_some_and(|argument| path_expression_is(argument, &["controllers"]))));

    let ports = parse("wost/src/arch/x86_64/port.rs");
    let claim = function(&ports, "isa_dma_controllers");
    let [Stmt::Expr(Expr::Call(pair), None)] = claim.block.stmts.as_slice() else {
        panic!("both DMA controller windows must be claimed atomically");
    };
    assert!(path_expression_is(&pair.func, &["claim_block_pair"]));
    assert_eq!(pair.args.len(), 2);
    assert!(integer_literal_is(&pair.args[0], 0));
    assert!(integer_literal_is(&pair.args[1], 0xc0));
    let ReturnType::Type(_, output) = &claim.sig.output else {
        panic!("the paired claim must return both complete controller windows");
    };
    let Type::Path(optional) = &**output else {
        panic!("the paired claim must refuse a conflicting owner");
    };
    let wrapper = optional.path.segments.last().expect("the Option wrapper");
    assert_eq!(wrapper.ident, "Option");
    let syn::PathArguments::AngleBracketed(arguments) = &wrapper.arguments else {
        panic!("the paired claim must carry both windows");
    };
    assert_eq!(arguments.args.len(), 1);
    let Some(syn::GenericArgument::Type(Type::Tuple(windows))) = arguments.args.first() else {
        panic!("the paired claim must carry exactly two windows");
    };
    // The spans include DMA2's odd-port aliases as well as its active even ports.
    assert_eq!(windows.elems.len(), 2);
    assert!(port_block_width(&windows.elems[0], 16));
    assert!(port_block_width(&windows.elems[1], 32));

    let policy = parse("libs/pci-config/src/lib.rs");
    let mask = policy
        .items
        .iter()
        .find_map(|item| match item {
            Item::Const(item) if item.ident == "ISA_DMA_PENDING_REQUESTS" => Some(&*item.expr),
            _ => None,
        })
        .expect("the combined pending-request mask");
    assert!(integer_literal_is(mask, 0xf0f0));
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
            && call.args.len() == 3
            && path_expression_is(&call.args[0], &["authority"])
            && path_expression_is(&call.args[1], &["domain"])
            && path_expression_is(&call.args[2], &["interrupts"])
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
            && call.args.len() == 4
            && path_expression_is(&call.args[0], &["authority"])
            && path_expression_is(&call.args[1], &["domain"])
            && path_expression_is(&call.args[2], &["interrupts"])
            && path_expression_is(&call.args[3], &["state"])
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
    assert!(
        by_value_named_input(admission.sig.inputs.iter().nth(2), "InterruptIsolation"),
        "the hardware door admits bus mastering without an interrupt-isolation proof"
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
    assert!(
        by_value_named_input(
            device_admission.sig.inputs.iter().nth(3),
            "InterruptIsolation"
        ),
        "the device door admits bus mastering without an interrupt-isolation proof"
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
fn pci_interrupts_are_default_deny_before_bus_mastering_or_sti() {
    let vtd_source = source("wost/src/arch/x86_64/vtd.rs");
    let vtd = parse("wost/src/arch/x86_64/vtd.rs");
    let proof = vtd
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "InterruptIsolation" => Some(item),
            _ => None,
        })
        .expect("missing interrupt-isolation proof");
    assert!(matches!(proof.vis, Visibility::Public(_)));
    assert!(exact_derives(&proof.attrs, &["Debug"]));
    let Fields::Named(fields) = &proof.fields else {
        panic!("InterruptIsolation must name its exact function");
    };
    assert_eq!(fields.named.len(), 1);
    let function = fields.named.first().expect("interrupt function field");
    assert!(matches!(function.vis, Visibility::Inherited));
    assert!(matches!(&function.ty, Type::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == "Address")));
    assert_eq!(authority_alias_count(&vtd, &["InterruptIsolation"]), 0);

    for requirement in [
        "if !platform_interrupt_remapping {",
        "if !extended.interrupt_remapping() || !extended.queued_invalidation() {",
        "if !extended.extended_interrupt_mode() {",
        "VtdError::FirmwareFacilitiesActive { status }",
        "if !translation_enabled(status)",
        "|| !interrupt_remapping_enabled(status)",
        "|| compatibility_interrupts_enabled(status)",
        "|| status & gsts::QIES != 0",
        "|| !fault_event_control::masked(fault_control)",
        "|| root_table_address(pointer) != self.root.address()",
        "observed != expected",
        "|| !interrupt_table_has_two_entries(observed)",
        "if self.interrupts.read_u64(index) != Some(0) {",
        "if observed == pointer",
    ] {
        assert!(
            vtd_source.contains(requirement),
            "VT-d interrupt boundary lost `{requirement}`"
        );
    }

    let mut transition = CallNames::default();
    transition.visit_impl_item_fn(method(&vtd, "Vtd", "transition").expect("VT-d transition"));
    let fault_mask = call_position(&transition, "mask_fault_interrupt");
    let root = call_position(&transition, "set_root_table");
    let interrupt_table = call_position(&transition, "set_interrupt_table");
    let interrupt_cache = call_position(&transition, "invalidate_interrupt_cache");
    let interrupt_enable = call_position(&transition, "switch_interrupt_remapping");
    let translation_enable = call_position(&transition, "switch_translation");
    let drain = call_position(&transition, "drain_dma");
    assert!(
        fault_mask < root
            && root < interrupt_table
            && interrupt_table < interrupt_cache
            && interrupt_cache < interrupt_enable
            && interrupt_enable < translation_enable
            && translation_enable < drain
    );
    assert_eq!(
        vtd_source.matches("if !takeover_ready(status) {").count(),
        2,
        "read-only takeover preflight must be repeated after table allocation and before the first unit write"
    );
    let establishment = vtd_source
        .split_once("    pub fn establish_deny_all(")
        .expect("deny-all establishment")
        .1
        .split_once("    /// Build one selected context")
        .expect("deny-all establishment end")
        .0;
    let allocation = establishment
        .find("IommuTable::new(TABLE_LIMIT, walks)")
        .expect("final deny-all table allocation");
    let repeated_preflight = establishment
        .find("if !takeover_ready(status) {")
        .expect("post-allocation takeover preflight");
    let construction = establishment
        .find("let mut unit = Self {")
        .expect("first hardware-owning value construction");
    assert!(allocation < repeated_preflight && repeated_preflight < construction);
    assert!(!vtd_source.contains("fn quiesce("));
    let invalidation = vtd_source
        .split_once("fn invalidate_interrupt_cache(")
        .expect("interrupt-cache invalidation")
        .1
        .split_once("fn switch_queued_invalidation(")
        .expect("interrupt-cache invalidation end")
        .0;
    let queue_enable = invalidation
        .find("self.switch_queued_invalidation(true)?")
        .expect("QI enable");
    let queue_readback = invalidation
        .find("let observed_queue = read_u64(&self.registers, vtd::IQA as usize)?;")
        .expect("IQA readback");
    let tail = invalidation
        .find("write_u64(&mut self.registers, vtd::IQT as usize, INVALIDATION_TAIL)?;")
        .expect("QI tail publication");
    let queue_disable = invalidation
        .find("let disabled = self.switch_queued_invalidation(false);")
        .expect("QI disable");
    assert!(queue_enable < queue_readback && queue_readback < tail && tail < queue_disable);
    let fault_mask = vtd_source
        .split_once("fn mask_fault_interrupt(")
        .expect("fault-event interrupt mask")
        .1
        .split_once("fn switch_translation(")
        .expect("fault-event interrupt mask end")
        .0;
    assert!(fault_mask.contains("let prior = read_u32(&self.registers, vtd::FECTL as usize)?;"));
    assert!(fault_mask.contains("prior | fault_event_control::MASK"));
    assert!(fault_mask.contains("fault_event_control::masked(observed)"));

    let iommu = source("kernel/src/iommu.rs");
    for requirement in [
        "if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {",
        "if dmar.sole_unit_segment() != Some(SEGMENT) {",
        "if !covers_every_present_requester(&requesters) {",
        "for bus in 0..=u8::MAX {",
        "if !requesters.iter().any(|covered| covered == requester) {",
        "pub(crate) fn establish_fabric_boundary(context: &mut ThreadContext) -> bool {",
        "acpi::find_unique_table(firmware, root, Sdt::DMA_COVERAGE)",
        "acpi::find_unique_table(firmware, root, Sdt::DMAR)",
        "DmaCoverage::new(&table)",
        "dmar.sole_complete_unit_register_base(SEGMENT)",
        "dmar.sole_rmrr_free_unit_register_base(SEGMENT)",
        "Vtd::establish_deny_all(registers, true)",
        "pub(crate) fn ensure_interrupt_boundary() -> bool {",
        "pci::TopologyPresence::Empty | pci::TopologyPresence::Present => {}",
        ".filter(|owner| owner.state == InstalledState::Ready && owner.boundary_active())",
    ] {
        assert!(
            iommu.contains(requirement),
            "kernel interrupt boundary lost `{requirement}`"
        );
    }
    assert_eq!(
        iommu
            .matches("if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {")
            .count(),
        2,
        "both early fabric establishment and sealed-topology lookup must refuse missing interrupt-remapping authority or x2APIC opt-out"
    );
    assert_eq!(iommu.matches("Vtd::establish_deny_all(").count(), 1);
    let early_register = iommu
        .split_once("fn early_register_base(")
        .expect("static early VT-d authority")
        .1
        .split_once("/// The remapping unit firmware says")
        .expect("static early VT-d authority end")
        .0;
    assert!(
        early_register
            .contains("coverage.segment() == SEGMENT && coverage.register_base() == register_base")
    );
    assert!(iommu.contains("pci::TopologyPresence::Unsealed => return false,"));
    assert!(iommu.contains(
        "INSTALLED\n        .lock()\n        .as_ref()\n        .is_some_and(|unit| unit.state == InstalledState::Ready && unit.boundary_active())"
    ));
    let acpi = source("libs/acpi-tables/src/lib.rs");
    let early_unit = acpi
        .split_once("pub fn sole_rmrr_free_unit_register_base(")
        .expect("static whole-fabric unit query")
        .1
        .split_once("/// The DRHD unit that governs `bdf`")
        .expect("static whole-fabric unit query end")
        .0;
    assert!(early_unit.contains("DmarStructure::Rmrr { segment: reserved, .. }"));
    assert!(early_unit.contains("include_pci_all: true"));

    let acpi = source("kernel/src/acpi.rs");
    let unique = acpi
        .split_once("pub(crate) fn find_unique_table(")
        .expect("unique ACPI authority lookup")
        .1
        .split_once("/// A whole table at a physical address")
        .expect("unique ACPI authority lookup end")
        .0;
    assert!(unique.contains("if require_unique {"));
    assert!(unique.contains("return None;"));

    let pci = source("wost/src/arch/x86_64/pci.rs");
    assert!(pci.contains("if interrupts.function() != domain.function() {"));

    let kernel_pci = source("kernel/src/pci.rs");
    assert!(kernel_pci.contains(
        "pub(crate) enum TopologyPresence {\n    Unsealed,\n    Empty,\n    Present,\n}"
    ));
    assert!(
        kernel_pci.contains(
            "if !topology_is_sealed() {\n        return TopologyPresence::Unsealed;\n    }"
        )
    );

    let main = source("kernel/src/main.rs");
    assert!(main.contains("if !iommu::interrupt_boundary_active() {"));
    assert!(main.contains("pci::TopologyPresence::Empty"));
    assert!(main.contains("if let Err(reason) = apic::enable_after_pending_audit() {"));
    let survey = main
        .split_once(
            "fn survey_the_bus(\n    dma_non_requester: Option<pci_config::QualifiedNonRequester>,\n    dma_quiesced: Option<pci_config::QuiescedIsaRequester>,\n)",
        )
        .expect("PCI survey")
        .1
        .split_once("enum StorageSelection")
        .expect("PCI survey end")
        .0;
    let early = survey
        .find("iommu::establish_fabric_boundary(&mut context)")
        .expect("early VT-d owner");
    let snapshot = survey
        .find("pci::snapshot_topology(dma_non_requester, dma_quiesced)")
        .expect("PCI topology snapshot");
    let enumeration = survey.find("pci::enumerate(").expect("PCI enumeration");
    assert!(early < snapshot && snapshot < enumeration);
    let start = main
        .split_once("fn start_interrupts(")
        .expect("initial interrupt admission")
        .1
        .split_once("const fn external_interrupts_are_admitted")
        .expect("initial interrupt admission end")
        .0;
    let remapping = start
        .find("if !iommu::interrupt_boundary_active() {")
        .expect("PCI interrupt boundary before sti");
    let pending = start
        .find("apic::enable_after_pending_audit()")
        .expect("pending-vector audit before sti");
    let ready = start
        .find("MASKABLE_INTERRUPTS_READY.store(true")
        .expect("interrupt-ready publication");
    assert!(remapping < pending && pending < ready);

    let apic = source("wost/src/arch/x86_64/apic.rs");
    assert!(apic.contains(
        "for (bits, register) in requests.iter_mut().zip(register::INTERRUPT_REQUEST) {"
    ));
    assert!(apic.contains(
        "if let Some(vector) = pending_vector(&access) {\n        return Err(PendingInterruptError::Pending(vector));\n    }"
    ));
    let audit = apic
        .split_once("pub fn enable_after_pending_audit()")
        .expect("local APIC pending audit")
        .1
        .split_once("fn initialize_controller")
        .expect("local APIC pending audit end")
        .0;
    let irr = audit
        .find("pending_vector(&access)")
        .expect("all-bank IRR audit");
    let refusal = audit
        .find("PendingInterruptError::Pending")
        .expect("pending-vector refusal");
    let sti = audit.find("interrupt::enable()").expect("initial sti");
    assert!(irr < refusal && refusal < sti);
    let route = main
        .split_once("fn route_keyboard(")
        .expect("keyboard routing boundary")
        .1
        .split_once("enum KeyboardRouteError")
        .expect("keyboard routing boundary end")
        .0;
    assert!(
        route.contains("if !matches!(pci::topology_presence(), pci::TopologyPresence::Empty) {"),
        "PCI presence must take the live keyboard-routing rollback branch"
    );
    let boundary = route
        .find("pci::topology_presence()")
        .expect("PCI topology-state routing branch");
    let rollback = route[boundary..]
        .find("rollback_keyboard_route(routing)")
        .map(|position| boundary + position)
        .expect("PCI routing rollback");
    let returned = route[rollback..]
        .find("return;")
        .map(|position| rollback + position)
        .expect("PCI routing rollback return");
    let commit = route
        .rfind("commit_keyboard_route(routing)")
        .expect("non-PCI keyboard routing commit");
    assert!(boundary < rollback && rollback < returned && returned < commit);

    let input = source("kernel/src/input.rs");
    assert_eq!(
        input
            .matches("crate::time::is_calibrated() && interrupt::are_enabled()")
            .count(),
        2,
        "a poller may sleep only while timer interrupts can wake it"
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
    for name in ["map_segment_with", "unmap_segment"] {
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
fn vtd_revocation_drains_older_dma_before_recycling_frames() {
    let syntax = parse("wost/src/arch/x86_64/vtd.rs");

    let mut preflight_calls = CallNames::default();
    preflight_calls.visit_impl_item_fn(method(&syntax, "Plan", "read").expect("VT-d preflight"));
    assert!(
        preflight_calls
            .0
            .iter()
            .any(|call| call == "register_invalidation_supported"),
        "preflight must reject versions that cannot execute register invalidation"
    );

    for name in ["unmap_segment", "rollback_range"] {
        let boundary = method(&syntax, "Vtd", name).expect("missing VT-d revocation boundary");
        let mut calls = CallNames::default();
        calls.visit_impl_item_fn(boundary);
        assert_eq!(
            calls
                .0
                .iter()
                .filter(|call| call.as_str() == "drain_dma")
                .count(),
            1,
            "{name} must issue one drained invalidation after clearing its leaves"
        );
        assert!(
            !calls.0.iter().any(|call| call == "invalidate"),
            "{name} must not recycle frames after a non-draining invalidation"
        );
    }

    let drain = method(&syntax, "Vtd", "drain_dma").expect("VT-d DMA drain boundary");
    let mut drain_calls = CallNames::default();
    drain_calls.visit_impl_item_fn(drain);
    let preparation = drain_calls
        .0
        .iter()
        .position(|call| call == "prepare_invalidation")
        .expect("a DMA drain must prepare the invalidation first");
    let command = drain_calls
        .0
        .iter()
        .position(|call| call == "global_with_drain")
        .expect("a DMA drain must request both read and write draining");
    let completion = drain_calls
        .0
        .iter()
        .position(|call| call == "poll_u64")
        .expect("a DMA drain must wait for invalidation completion");
    assert!(preparation < command && command < completion);
    assert!(
        !drain_calls.0.iter().any(|call| call == "global"),
        "the DMA drain command must not regress to a plain global invalidation"
    );

    #[derive(Default)]
    struct AcceptedGlobalInvalidations {
        context: usize,
        iotlb: usize,
    }
    impl<'ast> Visit<'ast> for AcceptedGlobalInvalidations {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "poll_u64"
                && let Some(Expr::Path(completion)) = call.args.last()
            {
                self.context += usize::from(path_ends(
                    &completion.path,
                    &["context_command", "global_completed"],
                ));
                self.iotlb += usize::from(path_ends(
                    &completion.path,
                    &["iotlb_command", "global_completed"],
                ));
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut drain_completions = AcceptedGlobalInvalidations::default();
    drain_completions.visit_impl_item_fn(drain);
    assert_eq!(drain_completions.iotlb, 1);
    assert_eq!(drain_completions.context, 0);

    let ordinary = method(&syntax, "Vtd", "invalidate").expect("ordinary VT-d invalidation");
    let mut ordinary_completions = AcceptedGlobalInvalidations::default();
    ordinary_completions.visit_impl_item_fn(ordinary);
    assert_eq!(ordinary_completions.iotlb, 1);
    assert_eq!(ordinary_completions.context, 1);

    let preparation =
        method(&syntax, "Vtd", "prepare_invalidation").expect("invalidation preparation");
    let mut preparation_calls = CallNames::default();
    preparation_calls.visit_impl_item_fn(preparation);
    assert!(
        preparation_calls
            .0
            .iter()
            .any(|call| call == "flush_write_buffer"),
        "DMA draining must preserve the required write-buffer flush"
    );

    let mapping = method(&syntax, "Vtd", "map_range").expect("private VT-d range walk");
    let mut mapping_calls = CallNames::default();
    mapping_calls.visit_impl_item_fn(mapping);
    assert!(mapping_calls.0.iter().any(|call| call == "invalidate"));
    assert!(!mapping_calls.0.iter().any(|call| call == "drain_dma"));
}

#[test]
fn vtd_publication_and_nvme_buffers_keep_exact_dma_authority() {
    let memory = source("wost/src/mm/mod.rs");
    let memory_syntax = parse("wost/src/mm/mod.rs");
    let atomic = method(&memory_syntax, "DirectSpan", "write_u64_volatile_at")
        .expect("typed volatile qword publication primitive");
    let Some(FnArg::Typed(value)) = atomic.sig.inputs.iter().nth(2) else {
        panic!("qword publication has no typed value");
    };
    assert!(matches!(&*value.ty, Type::Path(path) if path_ends(&path.path, &["u64"])));
    let mut atomic_calls = CallNames::default();
    atomic_calls.visit_impl_item_fn(atomic);
    assert_eq!(
        atomic_calls
            .0
            .iter()
            .filter(|call| *call == "write_volatile")
            .count(),
        1,
        "the qword publication primitive must issue exactly one volatile store"
    );
    for forbidden in ["write_byte_volatile", "write_volatile_at"] {
        assert!(
            !atomic_calls.0.iter().any(|call| call == forbidden),
            "the qword publication primitive must not call {forbidden}"
        );
    }
    assert!(
        memory.contains("destination.cast::<u64>().write_volatile(value.to_le())"),
        "live VT-d entries must use one aligned qword publication"
    );
    let table_writer =
        method(&memory_syntax, "IommuTable", "write_u64").expect("IOMMU-table qword writer");
    assert_eq!(
        table_writer.block.stmts.len(),
        5,
        "the IOMMU-table writer gained an alternate publication step"
    );
    #[derive(Default)]
    struct TablePublications {
        span_writes: usize,
        exact_qword_writes: usize,
        free_writes: usize,
        assignments: usize,
        unsafe_blocks: usize,
    }
    impl<'ast> Visit<'ast> for TablePublications {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if path_expression_is(&call.receiver, &["span"])
                && call.method.to_string().contains("write")
            {
                self.span_writes += 1;
                self.exact_qword_writes += usize::from(
                    call.method == "write_u64_volatile_at"
                        && call.args.len() == 2
                        && path_expression_is(&call.args[0], &["offset"])
                        && path_expression_is(&call.args[1], &["value"]),
                );
            }
            syn::visit::visit_expr_method_call(self, call);
        }

        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if matches!(&*call.func, Expr::Path(path)
                if path.path.segments.last().is_some_and(|segment|
                    segment.ident.to_string().contains("write")))
            {
                self.free_writes += 1;
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_assign(&mut self, assignment: &'ast ExprAssign) {
            self.assignments += 1;
            syn::visit::visit_expr_assign(self, assignment);
        }

        fn visit_expr_unsafe(&mut self, block: &'ast syn::ExprUnsafe) {
            self.unsafe_blocks += 1;
            syn::visit::visit_expr_unsafe(self, block);
        }
    }
    let mut publications = TablePublications::default();
    publications.visit_impl_item_fn(table_writer);
    assert_eq!(
        (
            publications.span_writes,
            publications.exact_qword_writes,
            publications.free_writes,
            publications.assignments,
            publications.unsafe_blocks,
        ),
        (1, 1, 0, 0, 0),
        "the IOMMU-table writer must have one exact span qword publication and no alternate write"
    );

    let vtd = source("wost/src/arch/x86_64/vtd.rs");
    let vtd_syntax = parse("wost/src/arch/x86_64/vtd.rs");
    for (name, direction) in [
        (
            "map_segment",
            "self.map_segment_with::<true, true>(segment)",
        ),
        (
            "map_segment_for_device_read",
            "self.map_segment_with::<true, false>(segment)",
        ),
        (
            "map_segment_for_device_write",
            "self.map_segment_with::<false, true>(segment)",
        ),
    ] {
        assert!(
            vtd.contains(direction),
            "{name} lost its exact DMA direction"
        );
    }
    exact_self_method_call(
        method(&vtd_syntax, "Vtd", "map_segment_with").expect("directional mapping boundary"),
        "map_range",
        &["paddr", "len"],
    );
    exact_self_method_call(
        method(&vtd_syntax, "Vtd", "map_range").expect("directional mapping walk"),
        "map_page",
        &["page"],
    );
    assert!(vtd.contains("self.map_range::<DEVICE_READS, DEVICE_WRITES>(paddr, len)"));
    assert!(vtd.contains("self.map_page::<DEVICE_READS, DEVICE_WRITES>(page)"));
    let map_page = method(&vtd_syntax, "Vtd", "map_page").expect("leaf mapping boundary");
    struct LeafPublications<'ast>(Vec<&'ast ExprMethodCall>);
    impl<'ast> Visit<'ast> for LeafPublications<'ast> {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "write_u64" {
                self.0.push(call);
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut leaf_publications = LeafPublications(Vec::new());
    leaf_publications.visit_impl_item_fn(map_page);
    let [pointer, leaf] = leaf_publications.0.as_slice() else {
        panic!("map_page must have exactly one pointer and one leaf publication site");
    };
    assert!(
        field_chain_is(&pointer.receiver, "self", &["tables"])
            && pointer.args.len() == 2
            && path_expression_is(&pointer.args[0], &["slot"])
            && path_expression_is(&pointer.args[1], &["pointer"]),
        "map_page's intermediate pointer publication is not exact"
    );
    let Some(pte) = leaf.args.get(1).and_then(|entry| call_is(entry, "pte")) else {
        panic!("map_page's leaf publication does not derive one PTE");
    };
    assert!(
        field_chain_is(&leaf.receiver, "self", &["tables"])
            && leaf.args.len() == 2
            && path_expression_is(&leaf.args[0], &["slot"])
            && pte.args.len() == 3
            && path_expression_is(&pte.args[0], &["page"])
            && path_expression_is(&pte.args[1], &["DEVICE_READS"])
            && path_expression_is(&pte.args[2], &["DEVICE_WRITES"]),
        "map_page's leaf publication widened or changed identity"
    );
    let nvme = source("kernel/src/nvme.rs");
    for mapping in [
        "map_segment_for_device_read(&mut self.submission)",
        "map_segment_for_device_write(&mut self.completion)",
        "map_segment_for_device_write(buffer)",
    ] {
        assert!(
            nvme.contains(mapping),
            "NVMe mapping lost its exact DMA direction: {mapping}"
        );
    }

    let main = source("kernel/src/main.rs");
    let main_syntax = parse("kernel/src/main.rs");
    assert!(main.contains("#[inline(never)]\nfn allocate_cleared_startup_buffer()"));
    let preparation = function(&main_syntax, "allocate_cleared_startup_buffer");
    #[derive(Default)]
    struct StartupPreparation {
        events: Vec<&'static str>,
        exact_allocations: usize,
        exact_clears: usize,
        exact_wraps: usize,
    }
    impl<'ast> Visit<'ast> for StartupPreparation {
        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if path_expression_is(&call.func, &["allocate_contiguous"]) {
                self.events.push("allocate");
                self.exact_allocations += usize::from(
                    call.args.len() == 2
                        && integer_literal_is(&call.args[0], 1)
                        && integer_literal_is(&call.args[1], 1),
                );
            } else if matches!(&*call.func, Expr::Path(path)
                if path_ends(&path.path, &["DmaSegment", "from"]))
            {
                self.events.push("wrap");
                self.exact_wraps += usize::from(
                    call.args.len() == 1 && path_expression_is(&call.args[0], &["segment"]),
                );
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "zero" {
                self.events.push("zero");
                self.exact_clears += usize::from(
                    call.args.is_empty() && path_expression_is(&call.receiver, &["segment"]),
                );
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut preparation_trace = StartupPreparation::default();
    preparation_trace.visit_item_fn(preparation);
    assert!(
        preparation_trace.events == ["allocate", "zero", "wrap"]
            && preparation_trace.exact_allocations == 1
            && preparation_trace.exact_clears == 1
            && preparation_trace.exact_wraps == 1,
        "one exact startup Segment must be cleared before that same Segment gains DMA authority"
    );

    #[derive(Default)]
    struct StartupUses {
        helper_calls: usize,
        exact_reads: usize,
        reads: usize,
    }
    impl<'ast> Visit<'ast> for StartupUses {
        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if path_expression_is(&call.func, &["allocate_cleared_startup_buffer"]) {
                self.helper_calls += 1;
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "read_blocks" {
                self.reads += 1;
                self.exact_reads += usize::from(
                    path_expression_is(&call.receiver, &["controller"])
                        && call.args.len() == 3
                        && integer_literal_is(&call.args[0], 0)
                        && integer_literal_is(&call.args[1], 1)
                        && matches!(&call.args[2], Expr::Reference(reference)
                            if reference.mutability.is_some()
                                && path_expression_is(&reference.expr, &["buffer"])),
                );
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let mut all_startup_uses = StartupUses::default();
    all_startup_uses.visit_file(&main_syntax);
    assert_eq!(all_startup_uses.helper_calls, 1);
    let start = function(&main_syntax, "start_candidate");
    let mut start_uses = StartupUses::default();
    start_uses.visit_item_fn(start);
    assert_eq!(
        (
            start_uses.helper_calls,
            start_uses.reads,
            start_uses.exact_reads
        ),
        (1, 1, 1)
    );
    assert!(
        matches!(start.block.stmts.iter().find_map(|statement| match statement {
        Stmt::Local(local)
            if matches!(&local.pat, Pat::TupleStruct(some)
                if path_ends(&some.path, &["Some"])
                    && matches!(some.elems.first(), Some(Pat::Ident(binding))
                        if binding.ident == "buffer" && binding.mutability.is_some()))
                && local.init.as_ref().is_some_and(|initializer|
                    call_is(&initializer.expr, "allocate_cleared_startup_buffer").is_some()) =>
        {
            Some(local)
        }
        _ => None,
    }), Some(local) if local.init.as_ref().is_some_and(|initializer| initializer.diverge.is_some()))
    );
}

fn exact_self_method_call(boundary: &syn::ImplItemFn, called: &str, arguments: &[&str]) {
    struct Calls<'a> {
        called: &'a str,
        arguments: &'a [&'a str],
        total: usize,
        exact: usize,
    }

    impl<'ast> Visit<'ast> for Calls<'_> {
        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == self.called {
                self.total += 1;
                self.exact += usize::from(
                    path_expression_is(&call.receiver, &["self"])
                        && call.args.len() == self.arguments.len()
                        && call
                            .args
                            .iter()
                            .zip(self.arguments)
                            .all(|(argument, expected)| path_expression_is(argument, &[*expected])),
                );
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }

    let mut calls = Calls {
        called,
        arguments,
        total: 0,
        exact: 0,
    };
    calls.visit_impl_item_fn(boundary);
    assert_eq!(calls.total, 1, "{called} must have one forwarding call");
    assert_eq!(
        calls.exact, 1,
        "{called} does not forward the exact DMA authority arguments"
    );
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
    assert_eq!(names, ["register_base", "isolation", "fences", "state"]);

    let iommu_source = source("kernel/src/iommu.rs");
    let early = iommu_source
        .split_once("pub(crate) fn establish_fabric_boundary(")
        .expect("early VT-d establishment")
        .1
        .split_once("/// The remapping unit firmware says")
        .expect("early VT-d establishment end")
        .0;
    let retained_takeover = early
        .split_once("Establishment::Retained { reason, isolation } => {")
        .expect("retained early takeover")
        .1
        .split_once("            return false;")
        .expect("retained early takeover end")
        .0;
    assert!(retained_takeover.contains("state: InstalledState::Quarantined"));
    assert!(!retained_takeover.contains("state: InstalledState::Ready"));

    let candidate = iommu_source
        .split_once("pub(crate) fn establish(device: &pci::Device)")
        .expect("candidate VT-d establishment")
        .1
        .split_once("/// Verify every live PCI requester")
        .expect("candidate VT-d establishment end")
        .0;
    let quarantine_guard = candidate
        .find("if owner.state != InstalledState::Ready {")
        .expect("permanent early-owner quarantine");
    let retarget = candidate
        .find("owner.isolation.retarget(at)")
        .expect("candidate retarget");
    assert!(quarantine_guard < retarget);
    let retained_retarget = candidate
        .split_once("Retargeting::Retained(reason) => {")
        .expect("uncertain retarget")
        .1;
    assert!(retained_retarget.contains("owner.state = InstalledState::Quarantined;"));

    let mut early_calls = CallNames::default();
    early_calls.visit_item_fn(function(&iommu, "establish_fabric_boundary"));
    assert!(
        call_position(&early_calls, "map_device")
            < call_position(&early_calls, "establish_deny_all")
    );
    let mut establish_calls = CallNames::default();
    establish_calls.visit_item_fn(function(&iommu, "establish"));
    assert!(
        call_position(&establish_calls, "acquire_fences")
            < call_position(&establish_calls, "retarget")
    );

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

    let mut park_calls = CallNames::default();
    park_calls.visit_impl_item_fn(method(&vtd, "Vtd", "park").expect("VT-d parking"));
    assert!(!park_calls.0.iter().any(|call| call == "switch_translation"));
    assert!(
        !park_calls
            .0
            .iter()
            .any(|call| call == "switch_interrupt_remapping")
    );
    assert!(
        call_position(&park_calls, "write_entry") < call_position(&park_calls, "invalidate")
            && call_position(&park_calls, "invalidate") < call_position(&park_calls, "drain_dma")
            && call_position(&park_calls, "drain_dma")
                < call_position(&park_calls, "clear_dma_tables")
    );
    let mut retarget_calls = CallNames::default();
    retarget_calls.visit_impl_item_fn(method(&vtd, "Vtd", "retarget").expect("VT-d retargeting"));
    assert!(
        !retarget_calls
            .0
            .iter()
            .any(|call| call == "switch_interrupt_remapping")
    );
    assert!(
        !retarget_calls
            .0
            .iter()
            .any(|call| call == "switch_translation")
    );

    let release = iommu_source
        .split_once("pub(crate) fn release_candidate(")
        .expect("candidate release")
        .1
        .split_once("/// Clear one inventory-bounded fault batch")
        .expect("candidate release end")
        .0;
    assert!(release.contains("unit.isolation.park()"));
    assert!(!release.contains("unmap_device_exact"));

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
        call_position(&transition_calls, "set_root_table")
            < call_position(&transition_calls, "invalidate")
    );
    assert!(
        call_position(&transition_calls, "invalidate")
            < call_position(&transition_calls, "switch_translation")
    );
    assert!(
        call_position(&transition_calls, "switch_translation")
            < call_position(&transition_calls, "drain_dma")
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
        ("requester_is_present", "topology_contains_requester"),
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

#[test]
fn firmware_reserved_dma_regions_never_become_mapping_authority() {
    let iommu = parse("kernel/src/iommu.rs");
    let unit = iommu
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "Unit" => Some(item),
            _ => None,
        })
        .expect("looked-up VT-d unit authority");
    let Fields::Named(fields) = &unit.fields else {
        panic!("Unit must name its bounded authority");
    };
    let names = fields
        .named
        .iter()
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["register_base", "requesters"],
        "firmware-reserved regions must not survive lookup as mapping authority"
    );

    let policy = function(&iommu, "reserved_regions_are_supported");
    let [Stmt::Expr(Expr::MethodCall(is_none), None)] = policy.block.stmts.as_slice() else {
        panic!("reserved-region policy must be one exact emptiness predicate");
    };
    let Expr::MethodCall(next) = &*is_none.receiver else {
        panic!("reserved-region policy must inspect the first entry");
    };
    assert!(
        is_none.method == "is_none"
            && is_none.args.is_empty()
            && next.method == "next"
            && next.args.is_empty()
            && path_expression_is(&next.receiver, &["reserved"]),
        "any firmware-reserved region must be refused"
    );

    let iommu_source = source("kernel/src/iommu.rs");
    let lookup = iommu_source
        .split_once("fn look_up(")
        .expect("DMAR lookup")
        .1
        .split_once("/// Whether this one unit covers")
        .expect("DMAR lookup end")
        .0;
    let coverage = lookup
        .find("covers_every_present_requester(&requesters)")
        .expect("complete requester coverage");
    let refusal = lookup
        .find("reserved_regions_are_supported(dmar.reserved_regions_for(")
        .expect("reserved-region refusal");
    let construction = lookup
        .find("Ok(Unit {")
        .expect("unit authority construction");
    assert!(coverage < refusal && refusal < construction);
    assert!(lookup.contains("return Err(LookupError::UnsupportedReservedRegion);"));

    struct EmptyReservedMappings {
        boundary_calls: usize,
        exact_boundary_calls: usize,
        retarget_calls: usize,
        exact_retarget_calls: usize,
    }

    impl<'ast> Visit<'ast> for EmptyReservedMappings {
        fn visit_expr_call(&mut self, call: &'ast ExprCall) {
            if matches!(&*call.func, Expr::Path(path)
                if path_ends(&path.path, &["Vtd", "establish_deny_all"]))
            {
                self.boundary_calls += 1;
                self.exact_boundary_calls += usize::from(matches!(
                    call.args.iter().collect::<Vec<_>>().as_slice(),
                    [registers, interrupts]
                        if path_expression_is(registers, &["registers"])
                            && bool_literal_is(interrupts, true)
                ));
            }
            syn::visit::visit_expr_call(self, call);
        }

        fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
            if call.method == "retarget" {
                self.retarget_calls += 1;
                self.exact_retarget_calls += usize::from(
                    field_chain_is(&call.receiver, "owner", &["isolation"])
                        && matches!(call.args.iter().collect::<Vec<_>>().as_slice(),
                            [target] if path_expression_is(target, &["at"])),
                );
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }

    let mut storage = EmptyReservedMappings {
        boundary_calls: 0,
        exact_boundary_calls: 0,
        retarget_calls: 0,
        exact_retarget_calls: 0,
    };
    storage.visit_item_fn(function(&iommu, "establish"));
    assert_eq!(
        (
            storage.boundary_calls,
            storage.exact_boundary_calls,
            storage.retarget_calls,
            storage.exact_retarget_calls,
        ),
        (0, 0, 1, 1),
        "storage retargeting must expose no firmware mapping argument"
    );

    let mut boundary = EmptyReservedMappings {
        boundary_calls: 0,
        exact_boundary_calls: 0,
        retarget_calls: 0,
        exact_retarget_calls: 0,
    };
    boundary.visit_item_fn(function(&iommu, "establish_fabric_boundary"));
    assert_eq!(
        (
            boundary.boundary_calls,
            boundary.exact_boundary_calls,
            boundary.retarget_calls,
            boundary.exact_retarget_calls,
        ),
        (1, 1, 0, 0),
        "the early empty owner must expose no firmware mapping argument"
    );

    let vtd = source("wost/src/arch/x86_64/vtd.rs");
    assert!(!vtd.contains("reserved: &[(u64, u64)]"));
    assert!(!vtd.contains("VtdError::ReservedRegion"));
}
