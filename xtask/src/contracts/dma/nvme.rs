use syn::{
    Expr, ExprAssign, ExprCall, ExprMethodCall, ExprStruct, Fields, FnArg, Item, Stmt, Type,
    Visibility, visit::Visit,
};

use super::{
    by_value_named_input, exact_derives, field_chain_is, function, method, parse, path_ends,
    path_expression_is,
};

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
