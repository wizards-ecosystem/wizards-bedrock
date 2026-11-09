use syn::{
    Expr, ExprAssign, ExprCall, ExprMethodCall, Fields, FnArg, Item, Pat, Stmt, Type, visit::Visit,
};

use crate::test_support::function_after;

use super::{
    CallNames, ReferencedPaths, call_is, call_position, field_chain_is, function,
    integer_literal_is, member_is, method, module_source, parse, path_ends, path_expression_is,
    source,
};

fn bool_literal_is(expression: &Expr, value: bool) -> bool {
    matches!(expression, Expr::Lit(literal)
        if matches!(&literal.lit, syn::Lit::Bool(boolean) if boolean.value == value))
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
    let memory = source("wost/src/mm.rs");
    let memory_syntax = parse("wost/src/mm.rs");
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
        memory.contains("let word = destination.cast::<u64>();")
            && memory.contains("unsafe { word.write_volatile(value.to_le()) };"),
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
    let nvme = module_source("kernel/src/nvme.rs");
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
    let early = function_after(&iommu_source, "pub(crate) fn establish_fabric_boundary(")
        .expect("early VT-d establishment");
    let retained_takeover = early
        .split_once("Establishment::Retained { reason, isolation } => {")
        .expect("retained early takeover")
        .1
        .split_once("            return false;")
        .expect("retained early takeover end")
        .0;
    assert!(retained_takeover.contains("state: InstalledState::Quarantined"));
    assert!(!retained_takeover.contains("state: InstalledState::Ready"));

    let candidate = function_after(
        &iommu_source,
        "pub(crate) fn establish(device: &pci::Device)",
    )
    .expect("candidate VT-d establishment");
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

    let release = function_after(&iommu_source, "pub(crate) fn release_candidate(")
        .expect("candidate release");
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
    let lookup = function_after(&iommu_source, "fn look_up(").expect("DMAR lookup");
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

#[test]
fn vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps() {
    use syn::parse::Parser;

    fn local_value<'a>(statement: &'a Stmt, name: &str) -> &'a Expr {
        let Stmt::Local(local) = statement else {
            panic!("missing {name} local");
        };
        assert!(matches!(&local.pat, Pat::Ident(binding) if binding.ident == name));
        &local.init.as_ref().expect("owner initializer").expr
    }
    fn segment_call(expression: &Expr, name: &str) -> bool {
        matches!(expression, Expr::MethodCall(call)
            if call.method == name && path_expression_is(&call.receiver, &["segment"])
                && call.args.len() == 1
                && path_expression_is(call.args.first().expect("owner argument"), &["owner"]))
    }
    fn owner_is_root(statement: &Stmt) {
        let Expr::Call(call) = local_value(statement, "owner") else {
            panic!("mapping owner mint");
        };
        assert!(path_expression_is(&call.func, &["DmaMappingOwner", "new"]));
        assert_eq!(call.args.len(), 1);
        assert!(matches!(call.args.first(), Some(Expr::MethodCall(address))
            if address.method == "address" && address.args.is_empty()
                && field_chain_is(&address.receiver, "self", &["root"])));
    }
    fn owner_refusal(statement: &Stmt, name: &str) {
        let Stmt::Expr(Expr::If(gate), _) = statement else {
            panic!("mapping owner gate");
        };
        assert!(matches!(&*gate.cond, Expr::Unary(negative)
            if matches!(negative.op, syn::UnOp::Not(_)) && segment_call(&negative.expr, name)));
        assert!(gate.else_branch.is_none());
        let [Stmt::Expr(Expr::Return(returned), _)] = gate.then_branch.stmts.as_slice() else {
            panic!("owner mismatch must return before any table access");
        };
        let reason = returned
            .expr
            .as_deref()
            .and_then(|expr| call_is(expr, "Err"))
            .and_then(|call| call.args.first())
            .and_then(|expr| call_is(expr, "Refused"))
            .and_then(|call| call.args.first())
            .expect("clean owner refusal");
        assert!(path_expression_is(reason, &["VtdError", "InvalidRange"]));
    }
    fn release(statement: &Stmt) {
        let Stmt::Macro(assertion) = statement else {
            panic!("exact mapping release assertion");
        };
        assert!(path_ends(&assertion.mac.path, &["assert"]));
        let arguments = syn::punctuated::Punctuated::<Expr, syn::Token![,]>::parse_terminated
            .parse2(assertion.mac.tokens.clone())
            .expect("release assertion arguments");
        assert_eq!(arguments.len(), 2);
        assert!(segment_call(
            arguments.first().expect("release expression"),
            "release_mapping"
        ));
    }
    fn retains(statement: &Stmt) {
        assert!(
            matches!(statement, Stmt::Expr(Expr::MethodCall(call), Some(_))
            if call.method == "retain" && call.args.is_empty()
                && path_expression_is(&call.receiver, &["segment"]))
        );
    }
    fn error_kind(arm: &syn::Arm, name: &str) -> bool {
        let Pat::TupleStruct(error) = &arm.pat else {
            return false;
        };
        if !path_ends(&error.path, &["Err"]) || error.elems.len() != 1 {
            return false;
        }
        let Some(Pat::Ident(binding)) = error.elems.first() else {
            return false;
        };
        let Some((_, pattern)) = &binding.subpat else {
            return false;
        };
        binding.ident == "error"
            && matches!(&**pattern, Pat::TupleStruct(kind)
            if path_ends(&kind.path, &["SegmentChangeError", name]))
    }
    fn error_tail(statement: &Stmt) {
        assert!(matches!(statement, Stmt::Expr(expression, None)
            if call_is(expression, "Err").is_some_and(|call|
                call.args.len() == 1 && call.args.first().is_some_and(|arg| path_expression_is(arg, &["error"])) )));
    }
    fn ok_tail(statement: &Stmt) {
        assert!(matches!(statement, Stmt::Expr(expression, None)
            if call_is(expression, "Ok").is_some_and(|call|
                call.args.len() == 1 && matches!(call.args.first(), Some(Expr::Tuple(unit)) if unit.elems.is_empty()))));
    }
    fn refuses_false(gate: &syn::ExprIf) {
        assert!(gate.else_branch.is_none());
        assert!(
            matches!(gate.then_branch.stmts.as_slice(), [Stmt::Expr(Expr::Return(returned), _)]
            if returned.expr.as_deref().is_some_and(|expr| bool_literal_is(expr, false)))
        );
    }
    fn owner_assignment(statement: &Stmt, present: bool) {
        let Stmt::Expr(Expr::Assign(assignment), Some(_)) = statement else {
            panic!("mapping lease state change");
        };
        assert!(field_chain_is(&assignment.left, "self", &["mapping_owner"]));
        if present {
            assert!(call_is(&assignment.right, "Some").is_some_and(|call| {
                call.args.len() == 1
                    && call
                        .args
                        .first()
                        .is_some_and(|arg| path_expression_is(arg, &["owner"]))
            }));
        } else {
            assert!(path_expression_is(&assignment.right, &["None"]));
        }
    }
    #[derive(Default)]
    struct TableTypes {
        bare: usize,
        published: usize,
    }
    impl<'ast> Visit<'ast> for TableTypes {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            self.bare += usize::from(path_ends(&path.path, &["IommuTable"]));
            self.published += usize::from(path_ends(&path.path, &["PublishedIommuTable"]));
            syn::visit::visit_type_path(self, path);
        }
    }
    let vtd = parse("wost/src/arch/x86_64/vtd.rs");
    let mm = parse("wost/src/mm.rs");
    let begin = method(&mm, "DmaSegment", "begin_mapping").expect("single-owner lease acquisition");
    assert_eq!(begin.block.stmts.len(), 3);
    let Stmt::Expr(Expr::If(gate), _) = &begin.block.stmts[0] else {
        panic!("lease acquisition gate");
    };
    refuses_false(gate);
    assert!(matches!(&*gate.cond, Expr::Binary(either)
        if matches!(either.op, syn::BinOp::Or(_))
            && matches!(&*either.left, Expr::MethodCall(call) if call.method == "is_none"
                && call.args.is_empty() && field_chain_is(&call.receiver, "self", &["segment"]))
            && matches!(&*either.right, Expr::MethodCall(call) if call.method == "is_some"
                && call.args.is_empty() && field_chain_is(&call.receiver, "self", &["mapping_owner"]))));
    owner_assignment(&begin.block.stmts[1], true);
    assert!(
        matches!(&begin.block.stmts[2], Stmt::Expr(value, None) if bool_literal_is(value, true))
    );
    let released = method(&mm, "DmaSegment", "release_mapping").expect("exact-owner lease release");
    assert_eq!(released.block.stmts.len(), 3);
    let Stmt::Expr(Expr::If(gate), _) = &released.block.stmts[0] else {
        panic!("lease release gate");
    };
    refuses_false(gate);
    assert!(matches!(&*gate.cond, Expr::Unary(negative)
        if matches!(negative.op, syn::UnOp::Not(_)) && matches!(&*negative.expr, Expr::MethodCall(call)
            if call.method == "is_mapped_by" && path_expression_is(&call.receiver, &["self"])
                && call.args.len() == 1 && path_expression_is(&call.args[0], &["owner"]))));
    owner_assignment(&released.block.stmts[1], false);
    assert!(
        matches!(&released.block.stmts[2], Stmt::Expr(value, None) if bool_literal_is(value, true))
    );
    let owner = vtd
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "Vtd" => Some(item),
            _ => None,
        })
        .expect("VT-d owner");
    let mut types = TableTypes::default();
    types.visit_item_struct(owner);
    assert_eq!(
        (types.bare, types.published),
        (0, 5),
        "unreviewed or recyclable hardware table owner"
    );
    let Fields::Named(fields) = &owner.fields else {
        panic!("VT-d fields");
    };
    let establishment = method(&vtd, "Vtd", "establish_deny_all").expect("VT-d establishment");
    let construction = establishment.block.stmts.iter().find_map(|statement| match statement {
        Stmt::Local(local) if matches!(&local.pat, Pat::Ident(binding) if binding.ident == "unit") => {
            match &*local.init.as_ref()?.expr { Expr::Struct(value) => Some(value), _ => None }
        }
        _ => None,
    }).expect("actual unit construction");
    for name in [
        "root",
        "context",
        "interrupts",
        "invalidation_queue",
        "tables",
    ] {
        let field = fields
            .named
            .iter()
            .find(|field| field.ident.as_ref().is_some_and(|ident| ident == name))
            .expect("table owner");
        assert!(
            matches!(&field.ty, Type::Path(path) if path_ends(&path.path, &["PublishedIommuTable"]))
        );
        let value = &construction
            .fields
            .iter()
            .find(|field| member_is(&field.member, name))
            .expect("published field")
            .expr;
        assert!(
            matches!(value, Expr::Call(call) if path_expression_is(&call.func, &["PublishedIommuTable", "new"])
            && call.args.len() == 1 && call.args.first().is_some_and(|arg| path_expression_is(arg, &[name])))
        );
    }
    let mapping = method(&vtd, "Vtd", "map_segment_with").expect("mapping lease boundary");
    assert_eq!(mapping.block.stmts.len(), 4);
    owner_is_root(&mapping.block.stmts[1]);
    owner_refusal(&mapping.block.stmts[2], "begin_mapping");
    let Stmt::Expr(Expr::Match(outcome), None) = &mapping.block.stmts[3] else {
        panic!("mapping outcome");
    };
    assert!(
        matches!(&*outcome.expr, Expr::MethodCall(call) if call.method == "map_range"
        && path_expression_is(&call.receiver, &["self"]) && call.args.len() == 2
        && path_expression_is(&call.args[0], &["paddr"]) && path_expression_is(&call.args[1], &["len"]))
    );
    assert_eq!(outcome.arms.len(), 3);
    for (index, kind) in [(0, "Retained"), (1, "Refused")] {
        let arm = &outcome.arms[index];
        assert!(arm.guard.is_none() && error_kind(arm, kind));
        let Expr::Block(body) = &*arm.body else {
            panic!("mapping refusal body");
        };
        assert_eq!(body.block.stmts.len(), 2);
        if index == 0 {
            retains(&body.block.stmts[0]);
        } else {
            release(&body.block.stmts[0]);
        }
        error_tail(&body.block.stmts[1]);
    }
    assert!(matches!(&outcome.arms[2].pat, Pat::TupleStruct(ok) if path_ends(&ok.path, &["Ok"])));
    assert!(outcome.arms[2].guard.is_none());
    ok_tail(&Stmt::Expr((*outcome.arms[2].body).clone(), None));

    let revocation = method(&vtd, "Vtd", "unmap_segment").expect("revocation lease boundary");
    assert_eq!(revocation.block.stmts.len(), 9);
    owner_is_root(&revocation.block.stmts[0]);
    owner_refusal(&revocation.block.stmts[1], "is_mapped_by");
    assert!(matches!(
        &revocation.block.stmts[6],
        Stmt::Expr(Expr::While(_), _)
    ));
    let Stmt::Expr(Expr::If(drain), _) = &revocation.block.stmts[7] else {
        panic!("drain outcome");
    };
    assert!(matches!(&*drain.cond, Expr::Let(binding)
        if matches!(&*binding.pat, Pat::TupleStruct(error) if path_ends(&error.path, &["Err"]))
            && matches!(&*binding.expr, Expr::MethodCall(call) if call.method == "drain_dma"
                && path_expression_is(&call.receiver, &["self"]) && call.args.is_empty())));
    assert!(drain.else_branch.is_none());
    let [Stmt::Expr(Expr::MethodCall(record), Some(_))] = drain.then_branch.stmts.as_slice() else {
        panic!("drain failure is discarded");
    };
    assert!(record.method == "get_or_insert" && path_expression_is(&record.receiver, &["failure"]));
    let Stmt::Expr(Expr::If(outcome), None) = &revocation.block.stmts[8] else {
        panic!("revocation outcome");
    };
    assert!(matches!(&*outcome.cond, Expr::Let(binding)
        if matches!(&*binding.pat, Pat::TupleStruct(present) if path_ends(&present.path, &["Some"]))
            && path_expression_is(&binding.expr, &["failure"])));
    assert_eq!(outcome.then_branch.stmts.len(), 2);
    retains(&outcome.then_branch.stmts[0]);
    let (_, clean) = outcome
        .else_branch
        .as_ref()
        .expect("proved revocation branch");
    let Expr::Block(clean) = &**clean else {
        panic!("proved revocation block");
    };
    assert_eq!(clean.block.stmts.len(), 2);
    release(&clean.block.stmts[0]);
    ok_tail(&clean.block.stmts[1]);
    let mut calls = CallNames::default();
    calls.visit_impl_item_fn(revocation);
    assert!(call_position(&calls, "is_mapped_by") < call_position(&calls, "unmap_page"));
    assert!(call_position(&calls, "unmap_page") < call_position(&calls, "drain_dma"));
}
