use std::collections::BTreeMap;

use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprMatch, GenericArgument, Item, Pat, PathArguments, Type};

use crate::paths::Paths;
use crate::test_support::{function_after, source_through};

use super::sources::production_sources_below;
use super::{
    cfg_not_selftest, module_source, module_syntax, observer_expr_attributes, require_error,
    require_success, selftest_cfg,
};

#[derive(Default)]
struct BooleanType(bool);

impl<'ast> Visit<'ast> for BooleanType {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path.qself.is_none()
            && path.path.segments.len() == 1
            && path.path.segments[0].ident == "bool"
        {
            self.0 = true;
        }
        visit::visit_type_path(self, path);
    }
}

#[derive(Default)]
struct BooleanInputs(Vec<String>);

impl<'ast> Visit<'ast> for BooleanInputs {
    fn visit_signature(&mut self, signature: &'ast syn::Signature) {
        for argument in &signature.inputs {
            let syn::FnArg::Typed(argument) = argument else {
                continue;
            };
            let mut boolean = BooleanType::default();
            boolean.visit_type(&argument.ty);
            if boolean.0 {
                self.0.push(signature.ident.to_string());
            }
        }
        visit::visit_signature(self, signature);
    }
}

#[test]
fn guard_types_do_not_cross_cpus() {
    let paths = Paths::discover().expect("workspace paths");
    let interrupt_state = "#![no_std]\nfn requires_send<T: Send>() {}\n\
        fn probe() { requires_send::<wost::arch::current::interrupt::InterruptState>(); }\n";
    require_error(&paths, "interrupt-state-send", interrupt_state, "E0277")
        .unwrap_or_else(|diagnostics| panic!("InterruptState was Send:\n{diagnostics}"));

    for guard in ["SpinLockGuard", "ThreadLockGuard"] {
        let source = format!(
            "#![no_std]\nfn requires_send<T: Send>() {{}}\n\
             fn probe<'a>() {{ requires_send::<wost::sync::{guard}<'a, u8>>(); }}\n"
        );
        require_error(&paths, "guard-send", &source, "E0277")
            .unwrap_or_else(|diagnostics| panic!("{guard} was Send:\n{diagnostics}"));
    }
}

#[test]
fn apic_completion_and_disarm_authorities_are_linear() {
    let paths = Paths::discover().expect("workspace paths");
    for authority in ["EoiToken<'static>", "DisarmedTimer"] {
        for (bound, label) in [("Copy", "copy"), ("Clone", "clone")] {
            let source = format!(
                "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
                 fn probe() {{ requires::<wost::arch::current::apic::{authority}>(); }}\n"
            );
            require_error(&paths, &format!("apic-linear-{label}"), &source, "E0277")
                .unwrap_or_else(|diagnostics| {
                    panic!("{authority} satisfied {bound}:\n{diagnostics}")
                });
        }
    }

    let raw_claim = "#![no_std]\nfn probe(frame: &wost::arch::current::interrupt::TrapFrame) { \
        let _ = wost::arch::current::apic::claim_eoi(frame, 0x40); }\n";
    require_error(&paths, "apic-delivery-vector", raw_claim, "E0308")
        .unwrap_or_else(|diagnostics| panic!("raw EOI vector compiled:\n{diagnostics}"));
}

#[test]
fn interrupt_programming_doors_reject_raw_vectors() {
    let paths = Paths::discover().expect("workspace paths");
    for (label, source) in [
        (
            "raw-local-vector",
            "#![no_std]\nfn probe() { let _ = x86_apic::local_vector::Entry::fixed(0x40); }\n",
        ),
        (
            "raw-spurious-vector",
            "#![no_std]\nfn probe() { let _ = x86_apic::spurious::Spurious::enabled(0xff); }\n",
        ),
        (
            "raw-idt-vector",
            "#![no_std]\nfn handler(_: &wost::arch::current::interrupt::TrapFrame) {}\n\
             fn reject(_: &wost::arch::current::interrupt::TrapFrame, _: wost::arch::current::interrupt::UserReturnError) -> ! { loop {} }\n\
             static HANDLERS: wost::arch::current::interrupt::Handlers = wost::arch::current::interrupt::Handlers::new(handler, reject);\n\
             fn probe() { wost::arch::current::interrupt::init(&HANDLERS, Some(0x80)); }\n",
        ),
    ] {
        require_error(&paths, label, source, "E0308")
            .unwrap_or_else(|diagnostics| panic!("raw vector reached {label}:\n{diagnostics}"));
    }
}

fn type_shape(ty: &Type) -> String {
    let Type::Path(path) = ty else {
        return "other".to_owned();
    };
    let Some(segment) = path.path.segments.last() else {
        return "empty".to_owned();
    };
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return segment.ident.to_string();
    };
    let inner = arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(type_shape(ty)),
        _ => None,
    });
    inner.map_or_else(
        || segment.ident.to_string(),
        |inner| format!("{}<{inner}>", segment.ident),
    )
}

#[derive(Default)]
struct NamedInputs(BTreeMap<(String, String), Vec<String>>);

impl<'ast> Visit<'ast> for NamedInputs {
    fn visit_signature(&mut self, signature: &'ast syn::Signature) {
        for argument in &signature.inputs {
            let syn::FnArg::Typed(argument) = argument else {
                continue;
            };
            let Pat::Ident(name) = argument.pat.as_ref() else {
                continue;
            };
            self.0
                .entry((signature.ident.to_string(), name.ident.to_string()))
                .or_default()
                .push(type_shape(&argument.ty));
        }
        visit::visit_signature(self, signature);
    }
}

fn require_input_type(paths: &Paths, source: &str, function: &str, input: &str, expected: &str) {
    let source = paths.root().join(source);
    let text = std::fs::read_to_string(&source)
        .unwrap_or_else(|error| panic!("reading {}: {error}", source.display()));
    let syntax = syn::parse_file(&text)
        .unwrap_or_else(|error| panic!("parsing {}: {error}", source.display()));
    let mut inputs = NamedInputs::default();
    inputs.visit_file(&syntax);
    assert_eq!(
        inputs
            .0
            .get(&(function.to_owned(), input.to_owned()))
            .map(Vec::as_slice),
        Some([expected.to_owned()].as_slice()),
        "{source:?} {function}::{input} lost its vector type"
    );
}

fn reject_raw_vector_inputs(paths: &Paths, sources: &[&str]) {
    for source in sources {
        let source = paths.root().join(source);
        let syntax = module_syntax(&source);
        let mut inputs = NamedInputs::default();
        inputs.visit_file(&syntax);
        for ((function, input), shapes) in inputs.0 {
            if input == "vector" && function != "gate_privilege" {
                assert!(
                    !shapes.iter().any(|shape| shape == "u8"),
                    "{} {function} recovered a raw vector door",
                    source.display()
                );
            }
        }
    }
}

#[derive(Default)]
struct ProductionAssignments(usize);

impl<'ast> Visit<'ast> for ProductionAssignments {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if module.attrs.iter().any(|attribute| {
            attribute.path().is_ident("cfg")
                && attribute
                    .meta
                    .require_list()
                    .is_ok_and(|list| list.tokens.to_string() == "test")
        }) {
            return;
        }
        visit::visit_item_mod(self, module);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(function) = call.func.as_ref() {
            let mut segments = function.path.segments.iter().rev();
            if core::iter::Iterator::next(&mut segments)
                .is_some_and(|segment| segment.ident == "new")
                && core::iter::Iterator::next(&mut segments)
                    .is_some_and(|segment| segment.ident == "Assignment")
            {
                self.0 += 1;
            }
        }
        visit::visit_expr_call(self, call);
    }
}

#[test]
fn one_assignment_and_typed_signatures_own_every_vector_door() {
    let paths = Paths::discover().expect("workspace paths");
    for (source, function, input, expected) in [
        (
            "libs/x86-apic/src/local_vector.rs",
            "configured",
            "vector",
            "InterruptVector",
        ),
        (
            "libs/x86-apic/src/local_vector.rs",
            "configured",
            "spurious",
            "InterruptVector",
        ),
        (
            "libs/x86-apic/src/local_vector.rs",
            "fixed",
            "vector",
            "InterruptVector",
        ),
        (
            "libs/x86-apic/src/local_vector.rs",
            "one_shot",
            "vector",
            "InterruptVector",
        ),
        (
            "libs/x86-apic/src/local_vector.rs",
            "periodic",
            "vector",
            "InterruptVector",
        ),
        (
            "libs/x86-apic/src/spurious.rs",
            "enabled",
            "vector",
            "InterruptVector",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            "init",
            "user_gate",
            "Option<InterruptVector>",
        ),
        (
            "wost/src/arch/x86_64/apic.rs",
            "claim_eoi",
            "vector",
            "InterruptVector",
        ),
        (
            "kernel/src/arch/x86_64/ioapic.rs",
            "route",
            "vector",
            "InterruptVector",
        ),
        (
            "kernel/src/arch/x86_64/ioapic.rs",
            "route_line",
            "vector",
            "InterruptVector",
        ),
        (
            "kernel/src/arch/x86_64/trap.rs",
            "claim_eoi",
            "vector",
            "InterruptVector",
        ),
    ] {
        require_input_type(&paths, source, function, input, expected);
    }
    reject_raw_vector_inputs(
        &paths,
        &[
            "libs/x86-apic/src/local_vector.rs",
            "libs/x86-apic/src/spurious.rs",
            "wost/src/arch/x86_64/apic.rs",
            "wost/src/arch/x86_64/interrupt.rs",
            "kernel/src/arch/x86_64/ioapic.rs",
            "kernel/src/arch/x86_64/ioapic/selftest.rs",
            "kernel/src/arch/x86_64/trap.rs",
            "kernel/src/arch/x86_64/trap/selftest.rs",
        ],
    );

    let mut assignments = Vec::new();
    for root in ["libs/x86-apic/src", "wost/src", "kernel/src"] {
        for source in production_sources_below(&paths.root().join(root)) {
            let text = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("reading {}: {error}", source.display()));
            let syntax = syn::parse_file(&text)
                .unwrap_or_else(|error| panic!("parsing {}: {error}", source.display()));
            let mut count = ProductionAssignments::default();
            count.visit_file(&syntax);
            assignments.extend(core::iter::repeat_n(source, count.0));
        }
    }
    assert_eq!(
        assignments.len(),
        1,
        "need one production vector assignment"
    );
    assert!(
        assignments[0].ends_with("kernel/src/arch/x86_64/vector.rs"),
        "vector assignment moved outside kernel authority: {}",
        assignments[0].display()
    );
}

#[test]
fn apic_policy_observations_are_typed() {
    let paths = Paths::discover().expect("workspace paths");
    let lifecycle = std::fs::read_to_string(paths.root().join("libs/x86-apic/src/lifecycle.rs"))
        .expect("APIC lifecycle source");
    let lifecycle = syn::parse_file(&lifecycle).expect("parse APIC lifecycle source");

    let in_service = std::fs::read_to_string(paths.root().join("libs/x86-apic/src/in_service.rs"))
        .expect("APIC in-service source");
    let in_service = syn::parse_file(&in_service).expect("parse APIC in-service source");
    let mut boolean_inputs = BooleanInputs::default();
    boolean_inputs.visit_file(&lifecycle);
    boolean_inputs.visit_file(&in_service);
    assert!(
        boolean_inputs.0.is_empty(),
        "APIC policy recovered boolean inputs in {:?}",
        boolean_inputs.0
    );

    let completion = in_service
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "may_complete" => Some(function),
            _ => None,
        })
        .expect("APIC in-service source has no completion planner");
    let observations: Vec<String> = completion
        .sig
        .inputs
        .iter()
        .map(|argument| {
            let syn::FnArg::Typed(argument) = argument else {
                panic!("completion planner unexpectedly takes self");
            };
            let syn::Type::Path(path) = argument.ty.as_ref() else {
                panic!("completion observation is not a named path");
            };
            path.path
                .segments
                .last()
                .expect("completion observation path")
                .ident
                .to_string()
        })
        .collect();
    assert_eq!(
        observations,
        [
            "Interrupts",
            "FrameLiveness",
            "u64",
            "InterruptVector",
            "DismissalOwnership",
        ],
        "APIC completion observations lost their typed roles"
    );
}

#[test]
fn apic_ready_publication_precedes_dependent_vector_reads() {
    let paths = Paths::discover().expect("workspace paths");
    let source = module_source(paths.root(), "wost/src/arch/x86_64/apic.rs");
    let between = |start, end| {
        source
            .split_once(start)
            .unwrap_or_else(|| panic!("missing APIC source marker {start:?}"))
            .1
            .split_once(end)
            .unwrap_or_else(|| panic!("missing APIC source marker {end:?}"))
            .0
    };

    let transaction = between(
        "fn initialize_controller<R: InitializationRegisters>",
        "fn initialize<R: InitializationRegisters>",
    );
    let configure = transaction
        .find("let configured = initialize(")
        .expect("APIC vector configuration");
    let publish = transaction
        .find("initialization.commit(configured)")
        .expect("APIC ready publication");
    assert!(
        configure < publish,
        "APIC readiness can publish before vector configuration returns"
    );
    let initialization = function_after(&source, "fn initialize<R: InitializationRegisters>")
        .expect("APIC initialization");
    assert!(
        initialization.contains(".self_interrupt_vector\n        .store(")
            && initialization.contains(".timer_vector\n        .store(")
            && initialization.contains(".spurious_vector\n        .store("),
        "APIC initialization no longer configures every published vector"
    );
    let commit = between(
        "fn commit(mut self, configured: Configured)",
        "impl Drop for Initialization",
    );
    assert!(
        commit.contains("self.controller.state.store(ready.bits(), Ordering::Release);"),
        "APIC readiness lost its Release publication"
    );
    let ready = commit
        .find("self.controller.state.store(ready.bits(), Ordering::Release);")
        .expect("APIC Ready publication");
    let restore = commit
        .find("interrupts.restore();")
        .expect("APIC initialization interrupt restoration");
    assert!(
        ready < restore,
        "APIC initialization restores interrupts before publishing Ready"
    );
    let lifecycle = between("fn lifecycle_at(state: &AtomicU8)", "fn admission_error");
    assert!(
        lifecycle.contains("state.load(Ordering::Acquire)"),
        "APIC readiness lost its Acquire observation"
    );
    let controller_lifecycle = between("fn lifecycle(self)", "fn ready_registers(self)");
    let ready_registers = between(
        "fn ready_registers(self)",
        "fn read(self, register: Register)",
    );
    let enabled = between(
        "fn is_enabled(self)",
        "fn published_vector(self, source: &AtomicU8)",
    );
    assert!(
        controller_lifecycle.contains("lifecycle_at(self.state)")
            && ready_registers.contains("self.lifecycle().is_ready()")
            && enabled.contains("self.ready_registers()"),
        "APIC vector readiness bypasses the Acquire lifecycle chain"
    );

    let vector = between(
        "fn published_vector(self, source: &AtomicU8)",
        "fn send_self_interrupt(self)",
    );
    let readiness = vector
        .find("if !self.is_enabled()")
        .expect("APIC vector readiness gate");
    let dependent = vector
        .find("source.load(Ordering::Relaxed)")
        .expect("APIC vector read");
    assert!(
        readiness < dependent,
        "an APIC vector can be read before acquiring Ready"
    );

    let ignored_write_fixture = source_through(
        &source,
        "fn ignored_spurious_write_is_terminal",
        "fn source_fixture_refuses",
    )
    .expect("APIC ignored-write fixture");
    for bypass in [
        "self_interrupt_vector.load(",
        "timer_vector.load(",
        "spurious_vector.load(",
        "SELF_INTERRUPT_VECTOR.load(",
        "TIMER_VECTOR.load(",
        "SPURIOUS_VECTOR.load(",
    ] {
        assert_eq!(
            source.matches(bypass).count(),
            ignored_write_fixture.matches(bypass).count(),
            "an APIC consumer outside the ignored-write fixture bypasses ordered vector publication: {bypass}"
        );
    }
    for consumer in [
        "self.published_vector(self.self_interrupt_vector)",
        "self.published_vector(self.spurious_vector)",
        "controller.published_vector(controller.timer_vector)",
        "controller.published_vector(controller.spurious_vector)",
    ] {
        assert!(
            source.contains(consumer),
            "an APIC consumer lost the ordered vector boundary: {consumer}"
        );
    }
}

#[test]
fn shared_memory_access_requires_one_partition() {
    let paths = Paths::discover().expect("workspace paths");
    for authority in ["Shared", "SharedBytes", "SharedRing<1>"] {
        let source = format!(
            "#![no_std]\nfn requires_clone<T: Clone>() {{}}\n\
             fn probe() {{ requires_clone::<wost::mm::{authority}>(); }}\n"
        );
        require_error(&paths, "shared-partition-clone", &source, "E0277").unwrap_or_else(
            |diagnostics| panic!("wost::mm::{authority} was duplicable:\n{diagnostics}"),
        );
    }

    for (label, source) in [
        (
            "raw-shared-bytes",
            "#![no_std]\nfn probe(value: &mut wost::mm::Shared) { let _ = value.write_volatile_at(0, &[]); }\n",
        ),
        (
            "raw-shared-atomic",
            "#![no_std]\nfn probe(value: &wost::mm::Shared) { let _ = value.load_acquire_u32(0); }\n",
        ),
        (
            "byte-shared-atomic",
            "#![no_std]\nfn probe(value: &wost::mm::SharedBytes) { let _ = value.load_acquire_u32(0); }\n",
        ),
        (
            "ring-whole-allocation-bytes",
            "#![no_std]\nfn probe(value: &mut wost::mm::SharedRing<1>) { let _ = value.write_volatile_at(0, &[]); }\n",
        ),
    ] {
        require_error(&paths, label, source, "E0599").unwrap_or_else(|diagnostics| {
            panic!("{label} recovered a forbidden door:\n{diagnostics}")
        });
    }

    for (label, source) in [
        (
            "bytes-back-to-raw",
            "#![no_std]\nfn probe(value: wost::mm::SharedBytes) -> wost::mm::Shared { value.into() }\n",
        ),
        (
            "ring-back-to-raw",
            "#![no_std]\nfn probe(value: wost::mm::SharedRing<1>) -> wost::mm::Shared { value.into() }\n",
        ),
    ] {
        require_error(&paths, label, source, "E0277").unwrap_or_else(|diagnostics| {
            panic!("{label} recovered the unpartitioned owner:\n{diagnostics}")
        });
    }

    let admitted = "#![no_std]\n\
        fn bytes(value: &mut wost::mm::SharedBytes) { let _ = value.write_volatile_at(0, &[]); }\n\
        fn ring(value: &mut wost::mm::SharedRing<1>) {\n\
            let _ = value.write_payload_at(0, &[]);\n\
            let _ = value.load_acquire_u32(0);\n\
        }\n";
    require_success(&paths, "partitioned-shared-access", admitted)
        .unwrap_or_else(|diagnostics| panic!("typed shared access was refused:\n{diagnostics}"));
}

#[test]
fn dma_ownership_and_publication_authorities_are_linear() {
    let paths = Paths::discover().expect("workspace paths");
    for authority in ["DmaState", "DmaPublication<'static>"] {
        for (bound, label) in [("Copy", "copy"), ("Clone", "clone")] {
            let source = format!(
                "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
                 fn probe() {{ requires::<nvme::{authority}>(); }}\n"
            );
            require_error(&paths, &format!("dma-{label}"), &source, "E0277").unwrap_or_else(
                |diagnostics| panic!("{authority} satisfied {bound}:\n{diagnostics}"),
            );
        }
    }
}

#[derive(Default)]
struct CompletionDispatch {
    calls: BTreeMap<String, (usize, usize)>,
}

impl<'ast> Visit<'ast> for CompletionDispatch {
    fn visit_expr_match(&mut self, expression: &'ast ExprMatch) {
        for arm in &expression.arms {
            let guard = arm
                .guard
                .as_ref()
                .map(|(_, expression)| expression.as_ref());
            let Some(source) = completion_source(&arm.pat, guard) else {
                continue;
            };
            let mut calls = CompletionCalls::default();
            calls.visit_expr(&arm.body);
            assert!(
                self.calls
                    .insert(source.clone(), (calls.claims, calls.dismissals))
                    .is_none(),
                "interrupt source {source} has more than one dispatch arm"
            );
        }
        visit::visit_expr_match(self, expression);
    }
}

#[derive(Default)]
struct GuardedVector(Vec<String>);

impl<'ast> Visit<'ast> for GuardedVector {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "get"
            && call.args.is_empty()
            && let Expr::Path(source) = call.receiver.as_ref()
        {
            let mut segments = source.path.segments.iter();
            if let (Some(module), Some(vector), None) = (
                core::iter::Iterator::next(&mut segments),
                core::iter::Iterator::next(&mut segments),
                core::iter::Iterator::next(&mut segments),
            ) && module.ident == "vector"
            {
                self.0.push(vector.ident.to_string());
            }
        }
        visit::visit_expr_method_call(self, call);
    }
}

fn completion_source(pattern: &Pat, guard: Option<&Expr>) -> Option<String> {
    let Pat::TupleStruct(interrupt) = pattern else {
        return None;
    };
    if interrupt.path.segments.last()?.ident != "Interrupt" || interrupt.elems.len() != 1 {
        return None;
    }
    let Pat::Ident(raw) = interrupt.elems.first()? else {
        return None;
    };
    assert_eq!(
        raw.ident, "raw",
        "interrupt dispatch lost its raw-boundary name"
    );
    let mut source = GuardedVector::default();
    source.visit_expr(guard.expect("a named interrupt arm has no typed vector guard"));
    assert_eq!(
        source.0.len(),
        1,
        "a named interrupt arm needs one typed vector guard"
    );
    source.0.pop()
}

#[derive(Default)]
struct CompletionCalls {
    claims: usize,
    dismissals: usize,
}

impl<'ast> Visit<'ast> for CompletionCalls {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if selftest_cfg(observer_expr_attributes(expression)) {
            return;
        }
        visit::visit_expr(self, expression);
    }

    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Expr::Path(function) = expression.func.as_ref()
            && function.path.segments.len() == 1
        {
            match function.path.segments[0].ident.to_string().as_str() {
                "claim_eoi" => self.claims += 1,
                "dismiss" => self.dismissals += 1,
                _ => {}
            }
        }
        visit::visit_expr_call(self, expression);
    }
}

#[derive(Default)]
struct ShippedPingDismiss(usize);

impl<'ast> Visit<'ast> for ShippedPingDismiss {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if cfg_not_selftest(&expression.attrs)
            && matches!(expression.func.as_ref(), Expr::Path(path)
                if path.path.is_ident("dismiss"))
            && matches!(expression.args.first(), Some(Expr::Path(path))
                if path.path.is_ident("eoi"))
        {
            self.0 += 1;
        }
        visit::visit_expr_call(self, expression);
    }
}

#[test]
fn every_interrupt_arm_has_explicit_completion_ownership() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("kernel/src/arch/x86_64/trap.rs"))
        .expect("trap source");
    let syntax = syn::parse_file(&source).expect("parse trap source");
    let on_trap = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "on_trap" => Some(function),
            _ => None,
        })
        .expect("trap source has no on_trap dispatcher");
    let mut dispatch = CompletionDispatch::default();
    Visit::visit_item_fn(&mut dispatch, on_trap);

    let expected = [
        ("ERROR", (0, 0)),
        ("KEYBOARD", (1, 1)),
        ("PING", (1, 1)),
        ("SPURIOUS", (0, 0)),
        ("SYSCALL", (0, 0)),
        ("TIMER", (1, 1)),
    ]
    .map(|(source, ownership)| (source.to_owned(), ownership))
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    assert_eq!(
        dispatch.calls, expected,
        "every interrupt arm needs one explicit APIC-completion classification"
    );
    let mut direct_ping = ShippedPingDismiss::default();
    <ShippedPingDismiss as Visit<'_>>::visit_item_fn(&mut direct_ping, on_trap);
    assert_eq!(
        direct_ping.0, 1,
        "the shipped ping arm must dismiss its authority directly"
    );
}
