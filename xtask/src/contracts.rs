//! Compile-time and source-shape contracts that stable Rust cannot assert directly.

use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use syn::visit::{self, Visit};
use syn::{
    Expr, ExprCall, ExprMatch, Fields, GenericArgument, GenericParam, Item, ItemEnum, ItemFn,
    Macro, Pat, PathArguments, Type,
};

use crate::paths::{KERNEL_TARGET, Paths};

static NEXT_PROBE: AtomicUsize = AtomicUsize::new(0);

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

struct Probe {
    root: std::path::PathBuf,
}

impl Probe {
    fn create(paths: &Paths, label: &str, source: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "wizard-{label}-contract-{}-{}",
            std::process::id(),
            NEXT_PROBE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("src")).expect("contract source directory");

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
                paths.target_dir().join("guard-contracts"),
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

fn require_missing_items(
    paths: &Paths,
    label: &str,
    source: &str,
    expected: &[(&str, &str)],
) -> Result<(), String> {
    let output = Probe::create(paths, label, source).compile(paths);
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    let source_errors = diagnostics
        .lines()
        .filter(|line| {
            (line.contains("src/lib.rs:") || line.contains("src\\lib.rs:"))
                && line.contains("error[")
        })
        .collect::<Vec<_>>();
    let exact = expected.len() == diagnostic_codes(&output).len()
        && expected.len() == source_errors.len()
        && expected.iter().all(|(code, item)| {
            source_errors
                .iter()
                .any(|line| line.contains(&format!("error[{code}]")) && line.contains(item))
        })
        && source_errors.iter().all(|line| {
            expected
                .iter()
                .any(|(code, item)| line.contains(&format!("error[{code}]")) && line.contains(item))
        });
    (!output.status.success() && exact)
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
             fn probe() { wost::arch::current::interrupt::init(handler, Some(0x80)); }\n",
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
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("reading {}: {error}", source.display()));
        let syntax = syn::parse_file(&text)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", source.display()));
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
            "kernel/src/arch/x86_64/trap.rs",
        ],
    );

    let mut assignments = Vec::new();
    for root in ["libs/x86-apic/src", "wost/src", "kernel/src"] {
        for source in rust_sources_below(&paths.root().join(root)) {
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
    let source = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/apic.rs"))
        .expect("wost APIC source");
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
    let initialization = between(
        "fn initialize<R: InitializationRegisters>",
        "/// Claim the exact APIC dismissal",
    );
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

    let ignored_write_fixture = between(
        "fn ignored_spurious_write_is_terminal",
        "fn initialization_conflict",
    );
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

#[test]
fn pci_configuration_writes_require_function_authority() {
    let paths = Paths::discover().expect("workspace paths");
    for (label, authority) in [
        ("pure", "pci_config::FunctionAuthority"),
        ("hardware", "wost::arch::current::pci::FunctionClaim"),
    ] {
        for bound in ["Copy", "Clone"] {
            let source = format!(
                "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
                 fn probe() {{ requires::<{authority}>(); }}\n"
            );
            require_error(
                &paths,
                &format!("pci-function-{label}-linear"),
                &source,
                "E0277",
            )
            .unwrap_or_else(|diagnostics| {
                panic!("PCI {label} function authority satisfied {bound}:\n{diagnostics}")
            });
        }
    }
    for (bound, label) in [("Copy", "copy"), ("Clone", "clone")] {
        let source = format!(
            "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
             fn probe() {{ requires::<wost::arch::current::pci::NvmeBusMasterAuthority<'static>>(); }}\n"
        );
        require_error(
            &paths,
            &format!("nvme-bus-master-authority-{label}"),
            &source,
            "E0277",
        )
        .unwrap_or_else(|diagnostics| {
            panic!("NVMe bus-master authority satisfied {bound}:\n{diagnostics}")
        });
    }

    let forged = "#![no_std]\n\
        use wost::arch::current::pci::{MappedBar, NvmeBusMasterAuthority};\n\
        fn forge<'a>(bar: &'a MappedBar, observed: nvme::DisabledRegisters) -> NvmeBusMasterAuthority<'a> {\n\
            NvmeBusMasterAuthority { bar, _observed: observed }\n\
        }\n";
    require_error(&paths, "nvme-bus-master-forgery", forged, "E0451").unwrap_or_else(
        |diagnostics| panic!("NVMe bus-master authority became forgeable:\n{diagnostics}"),
    );

    let parallel_registry = "#![no_std]\nfn probe(_: pci_config::CommandLeases<1>) {}\n";
    require_error(
        &paths,
        "pci-parallel-command-registry",
        parallel_registry,
        "E0603",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("a caller could substitute a parallel command registry:\n{diagnostics}")
    });
    let unchecked_release = "#![no_std]\n\
        fn probe(registry: &mut pci_config::FunctionClaims<1>, authority: pci_config::FunctionAuthority) {\n\
            let _ = registry.release(authority);\n\
        }\n";
    require_error(
        &paths,
        "pci-unchecked-function-release",
        unchecked_release,
        "E0624",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("function authority bypassed its live sublease check:\n{diagnostics}")
    });

    for (label, body, code) in [
        ("memory", "let _ = pci::enable_memory_space(at);", "E0425"),
        ("bus-master", "let _ = pci::enable_bus_master(at);", "E0425"),
        (
            "bus-master-upgrade",
            "let _ = pci::admit_bus_master(at, state);",
            "E0308",
        ),
        (
            "restore",
            "let _ = pci::restore_command(at, state);",
            "E0308",
        ),
        ("commit", "let _ = pci::commit_command(at, state);", "E0308"),
        ("probe", "let _ = pci::probe_bars(at);", "E0308"),
        (
            "probe-retry",
            "let _ = pci::retry_probe(at, retry);",
            "E0308",
        ),
        ("release", "let _ = pci::release_function(at);", "E0308"),
    ] {
        let arguments = match label {
            "bus-master-upgrade" => ", state: &mut pci_config::CommandState",
            "restore" | "commit" => ", state: pci_config::CommandState",
            "probe-retry" => ", retry: pci_config::ProbeRetry",
            _ => "",
        };
        let source = format!(
            "#![no_std]\nuse wost::arch::current::pci;\n\
             fn probe(at: pci_config::Address{arguments}) {{ {body} }}\n"
        );
        require_error(&paths, &format!("pci-raw-{label}"), &source, code).unwrap_or_else(
            |diagnostics| panic!("raw PCI address reached {label} mutation:\n{diagnostics}"),
        );
    }

    for (label, input) in [
        ("function", "&pci::FunctionClaim"),
        ("mapped-bar", "&pci::MappedBar"),
    ] {
        let source = format!(
            "#![no_std]\nuse wost::arch::current::pci;\n\
             fn probe(input: {input}, state: &mut pci_config::CommandState) {{\n\
                 let _ = pci::admit_bus_master(input, state);\n\
             }}\n"
        );
        require_error(
            &paths,
            &format!("pci-unqualified-bus-master-{label}"),
            &source,
            "E0308",
        )
        .unwrap_or_else(|diagnostics| {
            panic!("{label} authority bypassed fresh NVMe observation:\n{diagnostics}")
        });
    }

    let read_only = "#![no_std]\nuse wost::arch::current::pci;\n\
        fn probe(at: pci_config::Address) { let _ = pci::read(at); let _ = pci::claim_function(at); }\n";
    require_success(&paths, "pci-read-and-claim", read_only).unwrap_or_else(|diagnostics| {
        panic!("read-only enumeration or the sole claim door was lost:\n{diagnostics}")
    });
}

#[derive(Debug, PartialEq, Eq)]
enum PciBarAdmissionEvent {
    Admit,
    Map,
    ClaimDisabled,
    EnableDecode,
}

#[derive(Default)]
struct PciBarAdmissionShape {
    events: Vec<PciBarAdmissionEvent>,
    raw_device_maps: usize,
}

impl<'ast> Visit<'ast> for PciBarAdmissionShape {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Expr::Path(function) = expression.func.as_ref()
            && let Some(name) = function.path.segments.last().map(|segment| &segment.ident)
        {
            if name == "map_pci_bar" {
                self.events.push(PciBarAdmissionEvent::Map);
            } else if name == "map_device" {
                self.raw_device_maps += 1;
            }
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        let event = match expression.method.to_string().as_str() {
            "admit_memory_bar" => Some(PciBarAdmissionEvent::Admit),
            "map_pci_bar" => Some(PciBarAdmissionEvent::Map),
            "claim_disabled" => Some(PciBarAdmissionEvent::ClaimDisabled),
            "enable_memory_bar" => Some(PciBarAdmissionEvent::EnableDecode),
            "map_device" => {
                self.raw_device_maps += 1;
                None
            }
            _ => None,
        };
        if let Some(event) = event {
            self.events.push(event);
        }
        visit::visit_expr_method_call(self, expression);
    }
}

#[test]
fn pci_bar_mapping_requires_exact_linear_authority() {
    let paths = Paths::discover().expect("workspace paths");
    for authority in [
        "pci_config::BarAuthority",
        "wost::arch::current::pci::AdmittedBar",
        "wost::arch::current::pci::BarMapping",
        "wost::arch::current::pci::DisabledBar",
        "wost::arch::current::pci::MappedBar",
        "wost::arch::current::pci::ReleasableBar",
    ] {
        for bound in ["Copy", "Clone"] {
            let source = format!(
                "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
                 fn probe() {{ requires::<{authority}>(); }}\n"
            );
            require_error(&paths, "pci-bar-linear", &source, "E0277").unwrap_or_else(
                |diagnostics| {
                    panic!("PCI BAR authority {authority} satisfied {bound}:\n{diagnostics}")
                },
            );
        }
    }

    let exact_mapping = "#![no_std]\n\
        use wost::arch::current::{paging, pci};\n\
        fn probe(space: &mut paging::AddressSpace, bar: pci::AdmittedBar)\n\
            -> Result<pci::BarMapping, paging::BarMapFailure>\n\
        { space.map_pci_bar(bar) }\n";
    require_success(&paths, "pci-bar-exact-mapping", exact_mapping).unwrap_or_else(|diagnostics| {
        panic!("the exact PCI BAR mapping door was lost:\n{diagnostics}")
    });

    let raw_mapping = "#![no_std]\n\
        use wost::arch::current::paging::{AddressSpace, AdmittedDeviceRange};\n\
        fn probe(space: &mut AddressSpace, range: AdmittedDeviceRange) {\n\
            let _ = space.map_pci_bar(range);\n\
        }\n";
    require_error(&paths, "pci-bar-raw-mapping", raw_mapping, "E0308").unwrap_or_else(
        |diagnostics| {
            panic!("a function-free physical range reached PCI BAR mapping:\n{diagnostics}")
        },
    );

    let replayed_mapping = "#![no_std]\n\
        use wost::arch::current::{paging::AddressSpace, pci::AdmittedBar};\n\
        fn probe(space: &mut AddressSpace, bar: AdmittedBar) {\n\
            let _ = space.map_pci_bar(bar);\n\
            let _ = space.map_pci_bar(bar);\n\
        }\n";
    require_error(
        &paths,
        "pci-bar-replayed-mapping",
        replayed_mapping,
        "E0382",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("one PCI BAR admission mapped more than once:\n{diagnostics}")
    });

    let main_source = std::fs::read_to_string(paths.root().join("kernel/src/main.rs"))
        .expect("read kernel main source");
    let main_syntax = syn::parse_file(&main_source).expect("parse kernel main source");
    let admission = main_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "admit_controller_window" => Some(function),
            _ => None,
        })
        .expect("kernel main has no PCI BAR admission path");
    let mut shape = PciBarAdmissionShape::default();
    Visit::visit_item_fn(&mut shape, admission);
    assert_eq!(
        shape.events,
        [
            PciBarAdmissionEvent::Admit,
            PciBarAdmissionEvent::Map,
            PciBarAdmissionEvent::ClaimDisabled,
            PciBarAdmissionEvent::EnableDecode,
        ],
        "PCI memory decode no longer follows exact range admission and mapping"
    );
    assert_eq!(
        shape.raw_device_maps, 0,
        "PCI BAR admission used a raw mapping door"
    );

    let nvme_source =
        std::fs::read_to_string(paths.root().join("kernel/src/nvme.rs")).expect("read NVMe source");
    let nvme_syntax = syn::parse_file(&nvme_source).expect("parse NVMe source");
    let prepare = nvme_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "prepare" => Some(function),
            _ => None,
        })
        .expect("NVMe source has no prepare function");
    let inputs = &prepare.sig.inputs;
    assert!(!inputs.is_empty(), "NVMe prepare has no context");
    assert!(
        inputs.len() >= 2,
        "NVMe prepare has no mapped register authority"
    );
    let syn::FnArg::Typed(registers) = &inputs[1] else {
        panic!("NVMe prepare has no mapped register authority");
    };
    assert!(
        matches!(registers.ty.as_ref(), Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == "MappedBar")),
        "NVMe prepare accepts registers without exact mapped BAR authority"
    );
    assert!(
        inputs.len() == 2,
        "NVMe prepare regained a raw BAR argument"
    );

    let mut nvme_shape = PciBarAdmissionShape::default();
    Visit::visit_file(&mut nvme_shape, &nvme_syntax);
    assert_eq!(
        nvme_shape.raw_device_maps, 0,
        "NVMe regained the generic device mapping door"
    );
}

#[derive(Default)]
struct InvalidationShape {
    invlpg_macros: usize,
}

impl<'ast> Visit<'ast> for InvalidationShape {
    fn visit_macro(&mut self, item: &'ast Macro) {
        if item.tokens.to_string().contains("invlpg") {
            self.invlpg_macros += 1;
        }
        visit::visit_macro(self, item);
    }
}

#[test]
fn invlpg_requires_canonical_address_authority() {
    let paths = Paths::discover().expect("workspace paths");
    let raw_call = "#![no_std]\nfn probe() { wost::arch::current::cpu::invalidate_page(0u64); }\n";
    require_error(&paths, "invlpg-raw-address", raw_call, "E0308")
        .unwrap_or_else(|diagnostics| panic!("raw u64 reached invlpg:\n{diagnostics}"));

    let source = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/cpu.rs"))
        .expect("read CPU source");
    let syntax = syn::parse_file(&source).expect("parse CPU source");
    let function = |name: &str| {
        syntax.items.iter().find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == name => Some(function),
            _ => None,
        })
    };
    let public = function("invalidate_page").expect("CPU source has no invalidate_page");
    assert!(
        function("invalidate_page_unchecked").is_none(),
        "CPU source retained a raw invalidation door"
    );

    assert!(matches!(public.vis, syn::Visibility::Public(_)));
    assert!(public.sig.unsafety.is_none());
    let mut arguments = public.sig.inputs.iter();
    let Some(syn::FnArg::Typed(address)) = core::iter::Iterator::next(&mut arguments) else {
        panic!("invalidate_page has no typed address authority");
    };
    assert!(core::iter::Iterator::next(&mut arguments).is_none());
    assert!(matches!(
        address.ty.as_ref(),
        syn::Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == "CanonicalAddress")
    ));

    let mut public_shape = InvalidationShape::default();
    Visit::visit_item_fn(&mut public_shape, public);
    assert_eq!(public_shape.invlpg_macros, 1);

    let mut complete_shape = InvalidationShape::default();
    complete_shape.visit_file(&syntax);
    assert_eq!(complete_shape.invlpg_macros, 1);
}

fn top_level_function<'ast>(syntax: &'ast syn::File, name: &str, relative: &str) -> &'ast ItemFn {
    let mut matches = syntax.items.iter().filter_map(|item| match item {
        Item::Fn(function) if function.sig.ident == name => Some(function),
        _ => None,
    });
    let function = core::iter::Iterator::next(&mut matches)
        .unwrap_or_else(|| panic!("{relative} has no {name} function"));
    assert!(
        core::iter::Iterator::next(&mut matches).is_none(),
        "{relative} has more than one {name} function"
    );
    function
}

fn is_pub_super(visibility: &syn::Visibility) -> bool {
    matches!(
        visibility,
        syn::Visibility::Restricted(restricted) if restricted.path.is_ident("super")
    )
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

#[derive(Default)]
struct UnrestrictedUnsafeFunctions(Vec<String>);

impl UnrestrictedUnsafeFunctions {
    fn record(&mut self, visibility: &syn::Visibility, signature: &syn::Signature) {
        if matches!(visibility, syn::Visibility::Public(_)) && signature.unsafety.is_some() {
            self.0.push(signature.ident.to_string());
        }
    }
}

impl<'ast> Visit<'ast> for UnrestrictedUnsafeFunctions {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.record(&function.vis, &function.sig);
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        self.record(&function.vis, &function.sig);
        visit::visit_impl_item_fn(self, function);
    }

    fn visit_foreign_item_fn(&mut self, function: &'ast syn::ForeignItemFn) {
        self.record(&function.vis, &function.sig);
        visit::visit_foreign_item_fn(self, function);
    }
}

#[test]
fn unsafe_instruction_doors_match_their_compiled_caller_scope() {
    let paths = Paths::discover().expect("workspace paths");
    for source in rust_sources_below(&paths.root().join("wost/src")) {
        let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
        let syntax = syn::parse_file(&text)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
        let mut unrestricted = UnrestrictedUnsafeFunctions::default();
        unrestricted.visit_file(&syntax);
        assert!(
            unrestricted.0.is_empty(),
            "{} contains unrestricted public unsafe functions: {:?}",
            relative.display(),
            unrestricted.0
        );
    }

    for (relative, names) in [
        ("wost/src/arch/x86_64/cpu.rs", &["set_page_table_base"][..]),
        ("wost/src/arch/x86_64/xstate.rs", &["save", "restore"][..]),
    ] {
        let source = std::fs::read_to_string(paths.root().join(relative))
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
        for name in names {
            let function = top_level_function(&syntax, name, relative);
            assert!(
                function.sig.unsafety.is_some(),
                "{relative}::{name} is no longer an unsafe instruction door"
            );
            assert!(
                is_pub_super(&function.vis),
                "{relative}::{name} is visible beyond the architecture module"
            );
        }
    }

    let relative = "wost/src/arch/x86_64/task.rs";
    let source = std::fs::read_to_string(paths.root().join(relative))
        .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
    let syntax =
        syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
    let writer = top_level_function(&syntax, "write_thread_base", relative);
    assert!(writer.sig.unsafety.is_none());
    assert!(
        matches!(writer.vis, syn::Visibility::Inherited),
        "{relative}::write_thread_base is visible outside its caller module"
    );
}

#[derive(Debug, PartialEq, Eq)]
enum ActivationEvent {
    LoadPageTable,
    PublishReady,
}

#[derive(Default)]
struct ProductionActivationOrder {
    events: Vec<ActivationEvent>,
}

impl<'ast> Visit<'ast> for ProductionActivationOrder {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        let loads_root = matches!(expression.func.as_ref(), Expr::Path(function)
            if function.qself.is_none()
                && function.path.segments.len() == 2
                && function.path.segments[0].ident == "cpu"
                && function.path.segments[1].ident == "set_page_table_base")
            && expression.args.len() == 1
            && expression.args.first().is_some_and(is_root_path);
        if loads_root {
            self.events.push(ActivationEvent::LoadPageTable);
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        let publishes_ready = expression.method == "commit"
            && is_named_path(&expression.receiver, "activation")
            && expression.args.len() == 1
            && expression.args.first().is_some_and(is_root_path);
        if publishes_ready {
            self.events.push(ActivationEvent::PublishReady);
        }
        visit::visit_expr_method_call(self, expression);
    }
}

fn is_root_path(expression: &Expr) -> bool {
    is_named_path(expression, "root")
}

fn is_named_path(expression: &Expr, expected: &str) -> bool {
    matches!(expression, Expr::Path(path)
        if path.qself.is_none()
            && path.path.segments.len() == 1
            && path.path.segments[0].ident == expected)
}

#[test]
fn paging_activation_loads_cr3_before_publishing_ready() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/paging.rs"))
        .expect("read paging source");
    let syntax = syn::parse_file(&source).expect("parse paging source");
    let activate = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "activate" => Some(function),
            _ => None,
        })
        .expect("paging source has no production activate function");

    let mut order = ProductionActivationOrder::default();
    Visit::visit_item_fn(&mut order, activate);
    assert_eq!(
        order.events,
        [
            ActivationEvent::LoadPageTable,
            ActivationEvent::PublishReady,
        ],
        "paging readiness was not published after the production CR3 load"
    );
}

#[derive(Debug, PartialEq, Eq)]
enum TaskInitEvent {
    ValidateArena,
    PublishInit,
    PinArena,
}

#[derive(Default)]
struct ProductionTaskInitOrder {
    events: Vec<TaskInitEvent>,
}

impl<'ast> Visit<'ast> for ProductionTaskInitOrder {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        let validates_arena = matches!(expression.func.as_ref(), Expr::Path(function)
            if function.qself.is_none()
                && function.path.is_ident("validate_arena"))
            && expression.args.len() == 1
            && matches!(expression.args.first(), Some(Expr::Reference(reference))
                if is_named_path(&reference.expr, "arena"));
        if validates_arena {
            self.events.push(TaskInitEvent::ValidateArena);
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        if expression.args.is_empty()
            && expression.method == "claim"
            && is_named_path(&expression.receiver, "INIT")
        {
            self.events.push(TaskInitEvent::PublishInit);
        }
        if expression.args.is_empty()
            && expression.method == "pin"
            && is_named_path(&expression.receiver, "arena")
        {
            self.events.push(TaskInitEvent::PinArena);
        }
        visit::visit_expr_method_call(self, expression);
    }
}

#[test]
fn task_init_validates_the_arena_before_publication_and_pinning() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/task.rs"))
        .expect("read task source");
    let syntax = syn::parse_file(&source).expect("parse task source");
    let init = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "init" => Some(function),
            _ => None,
        })
        .expect("task source has no production init function");

    let mut order = ProductionTaskInitOrder::default();
    Visit::visit_item_fn(&mut order, init);
    assert_eq!(
        order.events,
        [
            TaskInitEvent::ValidateArena,
            TaskInitEvent::PublishInit,
            TaskInitEvent::PinArena,
        ],
        "task init did not validate its arena before publication and pinning"
    );
}

#[derive(Debug, PartialEq, Eq)]
enum TaskSwitchEvent {
    PageTableGuard,
    LoadPageTable,
    ExtendedState,
    Context,
}

#[derive(Default)]
struct ProductionTaskSwitchOrder {
    events: Vec<TaskSwitchEvent>,
    guarded_loads: usize,
    load_calls: usize,
    inside_page_table_guard: bool,
}

impl<'ast> Visit<'ast> for ProductionTaskSwitchOrder {
    fn visit_expr_if(&mut self, expression: &'ast syn::ExprIf) {
        if !is_task_page_table_guard(&expression.cond) {
            visit::visit_expr_if(self, expression);
            return;
        }

        self.events.push(TaskSwitchEvent::PageTableGuard);
        visit::visit_expr(self, &expression.cond);
        assert!(
            !self.inside_page_table_guard,
            "nested task page-table guard"
        );
        self.inside_page_table_guard = true;
        visit::visit_block(self, &expression.then_branch);
        self.inside_page_table_guard = false;
        if let Some((_, alternative)) = &expression.else_branch {
            visit::visit_expr(self, alternative);
        }
    }

    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        let loads_wanted = matches!(expression.func.as_ref(), Expr::Path(function)
            if function.qself.is_none()
                && function.path.segments.len() == 2
                && function.path.segments[0].ident == "cpu"
                && function.path.segments[1].ident == "set_page_table_base")
            && expression.args.len() == 1
            && expression
                .args
                .first()
                .is_some_and(|argument| is_named_path(argument, "wanted"));
        if loads_wanted {
            self.load_calls += 1;
            if self.inside_page_table_guard {
                self.guarded_loads += 1;
                self.events.push(TaskSwitchEvent::LoadPageTable);
            }
        }

        let starts_extended_state = matches!(expression.func.as_ref(), Expr::Path(function)
            if function.qself.is_none()
                && function.path.segments.len() == 2
                && function.path.segments[0].ident == "xstate"
                && function.path.segments[1].ident == "is_enabled")
            && expression.args.is_empty();
        if starts_extended_state {
            self.events.push(TaskSwitchEvent::ExtendedState);
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        if expression.method == "as_mut_ptr"
            && expression.args.is_empty()
            && is_named_path(&expression.receiver, "CONTEXTS")
        {
            self.events.push(TaskSwitchEvent::Context);
        }
        visit::visit_expr_method_call(self, expression);
    }
}

fn is_task_page_table_guard(expression: &Expr) -> bool {
    let Expr::Binary(binary) = expression else {
        return false;
    };
    let right_is_live_root = matches!(binary.right.as_ref(), Expr::Call(call)
        if matches!(call.func.as_ref(), Expr::Path(function)
            if function.qself.is_none()
                && function.path.segments.len() == 2
                && function.path.segments[0].ident == "cpu"
                && function.path.segments[1].ident == "page_table_base")
            && call.args.is_empty());
    matches!(binary.op, syn::BinOp::Ne(_))
        && is_named_path(&binary.left, "wanted")
        && right_is_live_root
}

#[test]
fn task_switch_loads_cr3_under_its_guard_before_context_state() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/task.rs"))
        .expect("read task source");
    let syntax = syn::parse_file(&source).expect("parse task source");
    let prepare = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "prepare" => Some(function),
            _ => None,
        })
        .expect("task source has no production prepare function");

    let mut order = ProductionTaskSwitchOrder::default();
    Visit::visit_item_fn(&mut order, prepare);
    assert_eq!(
        order.load_calls, 1,
        "task switch has the wrong CR3 load count"
    );
    assert_eq!(
        order.guarded_loads, 1,
        "task switch CR3 load escaped its live-root guard"
    );
    assert_eq!(
        order.events,
        [
            TaskSwitchEvent::PageTableGuard,
            TaskSwitchEvent::LoadPageTable,
            TaskSwitchEvent::ExtendedState,
            TaskSwitchEvent::Context,
        ],
        "task switch CR3 admission no longer precedes context state"
    );
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

#[test]
fn machine_probes_are_absent_from_the_shipped_wost_graph() {
    let paths = Paths::discover().expect("workspace paths");
    let probes = [
        (
            "write-protection-driver",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::cpu::drive_write_protection_for_test; }\n",
            "E0425",
        ),
        (
            "write-protection-probe",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::cpu::write_protection_probe; }\n",
            "E0425",
        ),
        (
            "write-protection-recovery",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::cpu::recover_write_protection_probe; }\n",
            "E0425",
        ),
        (
            "fast-syscall-driver",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::cpu::drive_fast_syscall_shutdown_for_test; }\n",
            "E0425",
        ),
        (
            "extended-state-probe-type",
            "#![no_std]\nfn probe(_: wost::arch::current::xstate::Probe) {}\n",
            "E0425",
        ),
        (
            "extended-state-writer",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::xstate::write_probe; }\n",
            "E0425",
        ),
        (
            "extended-state-reader",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::xstate::read_probe; }\n",
            "E0425",
        ),
        (
            "user-copy-fault-window",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::usercopy::fault_window; }\n",
            "E0425",
        ),
        (
            "user-copy-recovery-driver",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::usercopy::drive_fault_recovery_for_test; }\n",
            "E0425",
        ),
        (
            "user-copy-unwalked-door",
            "#![no_std]\nfn probe() { let _ = wost::arch::current::usercopy::copy_from_user_unwalked; }\n",
            "E0425",
        ),
    ];

    for (label, source, code) in probes {
        require_error(&paths, label, source, code)
            .unwrap_or_else(|diagnostics| panic!("{label} reached shipped code:\n{diagnostics}"));
    }

    const OBSERVERS: &str = r"#![no_std]
use wost::arch::current::apic::is_enabled;
use wost::arch::current::apic::timer_entry;
use wost::arch::current::cpu::access_check_is_off;
use wost::arch::current::interrupt::SAVED_FRAME_BYTES;
use wost::arch::current::interrupt::access_clear_is_armed;
use wost::arch::current::interrupt::breakpoint;
use wost::arch::current::interrupt::buffer_clear_is_armed;
use wost::arch::current::interrupt::buffer_clears;
use wost::arch::current::interrupt::gate_privilege;
use wost::arch::current::interrupt::live_frame_address;
use wost::arch::current::interrupt::table_is_loaded;
use wost::arch::current::paging::cleared_on_drop;
use wost::arch::current::paging::reclaimed_refusals;
use wost::arch::current::pic::masks;
use wost::arch::current::segment::code_selector;
use wost::arch::current::segment::double_fault_stack;
use wost::arch::current::segment::stack_selector;
use wost::arch::current::segment::task_register;
use wost::arch::current::xstate::save_stays_inside_its_area;
use wost::mm::heap::Stats;
use wost::mm::heap::stats;
use wost::mm::first_memory_run;
use wost::mm::free_frames;
use wost::sync::taken_under_a_leaf;
use wost::sync::the_leaf_it_was_under;
fn observe(
    frame: &wost::mm::Frame,
    segment: &wost::mm::Segment,
    space: &wost::arch::current::paging::AddressSpace,
    unmapped: &wost::arch::current::paging::Unmapped,
    framebuffer: &wost::boot::Framebuffer,
) {
    let _ = frame.is_zero();
    let _ = segment.read_at(0, &mut []);
    let _ = space.device_mappings();
    let _ = unmapped.was_user();
    let _ = framebuffer.address();
    let _ = framebuffer.mapped_bytes();
    let _ = framebuffer.has_back_buffer();
    let _ = framebuffer.pixel_at(0, 0);
}
";
    const MISSING: &[(&str, &str)] = &[
        ("E0432", "apic::is_enabled"),
        ("E0432", "apic::timer_entry"),
        ("E0432", "cpu::access_check_is_off"),
        ("E0432", "interrupt::SAVED_FRAME_BYTES"),
        ("E0432", "interrupt::access_clear_is_armed"),
        ("E0432", "interrupt::breakpoint"),
        ("E0432", "interrupt::buffer_clear_is_armed"),
        ("E0432", "interrupt::buffer_clears"),
        ("E0432", "interrupt::gate_privilege"),
        ("E0432", "interrupt::live_frame_address"),
        ("E0432", "interrupt::table_is_loaded"),
        ("E0432", "paging::cleared_on_drop"),
        ("E0432", "paging::reclaimed_refusals"),
        ("E0432", "pic::masks"),
        ("E0432", "segment::code_selector"),
        ("E0432", "segment::double_fault_stack"),
        ("E0432", "segment::stack_selector"),
        ("E0432", "segment::task_register"),
        ("E0432", "xstate::save_stays_inside_its_area"),
        ("E0432", "heap::Stats"),
        ("E0432", "heap::stats"),
        ("E0432", "mm::first_memory_run"),
        ("E0432", "mm::free_frames"),
        ("E0432", "sync::taken_under_a_leaf"),
        ("E0432", "sync::the_leaf_it_was_under"),
        ("E0599", "is_zero"),
        ("E0599", "read_at"),
        ("E0599", "device_mappings"),
        ("E0599", "was_user"),
        ("E0599", "address"),
        ("E0599", "mapped_bytes"),
        ("E0599", "has_back_buffer"),
        ("E0599", "pixel_at"),
    ];
    assert_eq!(MISSING.len(), 33, "D-290's shipped wost inventory");
    require_missing_items(&paths, "machine-only-observers", OBSERVERS, MISSING).unwrap_or_else(
        |diagnostics| panic!("a machine-only observer reached shipped code:\n{diagnostics}"),
    );
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

struct ObserverUses {
    watched: BTreeSet<String>,
    selftest_depth: usize,
    selftest: BTreeMap<String, usize>,
    shipped: BTreeMap<String, usize>,
}

impl ObserverUses {
    fn new(expected: &[(&str, usize)]) -> Self {
        Self {
            watched: expected
                .iter()
                .map(|(name, _)| (*name).to_owned())
                .collect(),
            selftest_depth: 0,
            selftest: BTreeMap::new(),
            shipped: BTreeMap::new(),
        }
    }

    fn guarded(&mut self, attributes: &[syn::Attribute], visit: impl FnOnce(&mut Self)) {
        let guarded = selftest_cfg(attributes);
        self.selftest_depth += usize::from(guarded);
        visit(self);
        self.selftest_depth -= usize::from(guarded);
    }

    fn visit_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => self.visit_tokens(group.stream()),
                proc_macro2::TokenTree::Ident(identifier) => self.visit_ident(&identifier),
                proc_macro2::TokenTree::Literal(_) | proc_macro2::TokenTree::Punct(_) => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for ObserverUses {
    fn visit_item(&mut self, item: &'ast Item) {
        self.guarded(observer_item_attributes(item), |this| {
            visit::visit_item(this, item);
        });
    }

    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        self.guarded(observer_impl_item_attributes(item), |this| {
            visit::visit_impl_item(this, item);
        });
    }

    fn visit_field(&mut self, field: &'ast syn::Field) {
        self.guarded(&field.attrs, |this| visit::visit_field(this, field));
    }

    fn visit_field_value(&mut self, field: &'ast syn::FieldValue) {
        self.guarded(&field.attrs, |this| visit::visit_field_value(this, field));
    }

    fn visit_field_pat(&mut self, field: &'ast syn::FieldPat) {
        self.guarded(&field.attrs, |this| visit::visit_field_pat(this, field));
    }

    fn visit_fn_arg(&mut self, argument: &'ast syn::FnArg) {
        let attributes = match argument {
            syn::FnArg::Receiver(receiver) => &receiver.attrs,
            syn::FnArg::Typed(argument) => &argument.attrs,
        };
        self.guarded(attributes, |this| visit::visit_fn_arg(this, argument));
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        self.guarded(&local.attrs, |this| visit::visit_local(this, local));
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.guarded(&arm.attrs, |this| visit::visit_arm(this, arm));
    }

    fn visit_stmt_macro(&mut self, statement: &'ast syn::StmtMacro) {
        self.guarded(&statement.attrs, |this| {
            visit::visit_stmt_macro(this, statement);
        });
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        self.guarded(observer_expr_attributes(expression), |this| {
            visit::visit_expr(this, expression);
        });
    }

    fn visit_ident(&mut self, identifier: &'ast syn::Ident) {
        let name = identifier.to_string();
        if self.watched.contains(&name) {
            let uses = if self.selftest_depth == 0 {
                &mut self.shipped
            } else {
                &mut self.selftest
            };
            *uses.entry(name).or_default() += 1;
        }
    }

    fn visit_token_stream(&mut self, tokens: &'ast proc_macro2::TokenStream) {
        self.visit_tokens(tokens.clone());
    }
}

fn require_selftest_observers(root: &std::path::Path, relative: &str, expected: &[(&str, usize)]) {
    let source = std::fs::read_to_string(root.join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    let syntax =
        syn::parse_file(&source).unwrap_or_else(|error| panic!("parse {relative}: {error}"));
    let mut uses = ObserverUses::new(expected);
    uses.visit_file(&syntax);

    assert!(
        uses.shipped.is_empty(),
        "{relative} has shipped machine-observer uses: {:?}",
        uses.shipped
    );
    for &(name, count) in expected {
        assert_eq!(
            uses.selftest.get(name).copied().unwrap_or(0),
            count,
            "{relative} changed the self-test-only `{name}` inventory"
        );
    }
}

fn require_selftest_parameter(
    root: &std::path::Path,
    relative: &str,
    function: &str,
    parameter: usize,
) {
    let source = std::fs::read_to_string(root.join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    let syntax =
        syn::parse_file(&source).unwrap_or_else(|error| panic!("parse {relative}: {error}"));
    let functions = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(item) if item.sig.ident == function => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(functions.len(), 1, "{relative} needs one {function}");
    let argument = functions[0]
        .sig
        .inputs
        .iter()
        .nth(parameter)
        .unwrap_or_else(|| panic!("{relative} {function} lost parameter {parameter}"));
    let attributes = match argument {
        syn::FnArg::Receiver(receiver) => &receiver.attrs,
        syn::FnArg::Typed(argument) => &argument.attrs,
    };
    assert!(
        selftest_cfg(attributes),
        "{relative} {function} parameter {parameter} reaches shipped code"
    );
}

struct NamedCalls<'ast> {
    function: &'ast str,
    calls: Vec<&'ast ExprCall>,
}

impl<'ast> Visit<'ast> for NamedCalls<'ast> {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if matches!(expression.func.as_ref(), Expr::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == self.function))
        {
            self.calls.push(expression);
        }
        visit::visit_expr_call(self, expression);
    }
}

fn require_selftest_call_argument(
    root: &std::path::Path,
    relative: &str,
    function: &str,
    argument: usize,
) {
    let source = std::fs::read_to_string(root.join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"));
    let syntax =
        syn::parse_file(&source).unwrap_or_else(|error| panic!("parse {relative}: {error}"));
    let mut calls = NamedCalls {
        function,
        calls: Vec::new(),
    };
    calls.visit_file(&syntax);
    assert_eq!(calls.calls.len(), 1, "{relative} needs one {function} call");
    let value = calls.calls[0]
        .args
        .iter()
        .nth(argument)
        .unwrap_or_else(|| panic!("{relative} {function} lost argument {argument}"));
    assert!(
        selftest_cfg(observer_expr_attributes(value)),
        "{relative} {function} argument {argument} reaches shipped code"
    );
}

#[derive(Default)]
struct MappingResultLocals<'ast> {
    injected: usize,
    results: Vec<&'ast syn::Local>,
}

impl<'ast> Visit<'ast> for MappingResultLocals<'ast> {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Pat::Ident(name) = &local.pat {
            if name.ident == "injected" {
                self.injected += 1;
            } else if name.ident == "result" {
                self.results.push(local);
            }
        }
        visit::visit_local(self, local);
    }
}

fn require_split_mapping_write(root: &std::path::Path) {
    let relative = "wost/src/arch/x86_64/paging.rs";
    let source = std::fs::read_to_string(root.join(relative)).expect("read paging source");
    let syntax = syn::parse_file(&source).expect("parse paging source");
    let methods = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(item) => Some(item.items.iter()),
            _ => None,
        })
        .flatten()
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) if method.sig.ident == "map_range" => Some(method),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 1, "paging needs one map_range method");

    let mut locals = MappingResultLocals::default();
    locals.visit_block(&methods[0].block);
    assert_eq!(
        locals.injected, 0,
        "shipped paging recovered a no-op injection flag"
    );
    assert_eq!(
        locals.results.len(),
        2,
        "map_range needs two compile-time result paths"
    );
    let selftest = locals
        .results
        .iter()
        .filter(|local| selftest_cfg(&local.attrs))
        .collect::<Vec<_>>();
    let shipped = locals
        .results
        .iter()
        .filter(|local| cfg_not_selftest(&local.attrs))
        .collect::<Vec<_>>();
    assert_eq!(
        selftest.len(),
        1,
        "map_range needs one self-test result path"
    );
    assert_eq!(shipped.len(), 1, "map_range needs one shipped result path");
    assert!(
        matches!(shipped[0].init.as_ref().map(|init| init.expr.as_ref()),
            Some(Expr::MethodCall(call)) if call.method == "set"),
        "shipped map_range must call set directly"
    );
    let mut refusal = NamedCalls {
        function: "mapping_refusal",
        calls: Vec::new(),
    };
    <NamedCalls as Visit<'_>>::visit_local(&mut refusal, selftest[0]);
    assert_eq!(
        refusal.calls.len(),
        1,
        "the self-test mapping path needs one refusal injection"
    );
}

fn asm_strings(tokens: proc_macro2::TokenStream, strings: &mut Vec<String>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Group(group) => asm_strings(group.stream(), strings),
            proc_macro2::TokenTree::Literal(literal) => {
                if let Ok(syn::Lit::Str(value)) = syn::parse_str::<syn::Lit>(&literal.to_string()) {
                    strings.push(value.value());
                }
            }
            proc_macro2::TokenTree::Ident(_) | proc_macro2::TokenTree::Punct(_) => {}
        }
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

struct BufferClearSite<'ast> {
    name: String,
    owner: Option<String>,
    invocation: &'ast Macro,
}

#[derive(Default)]
struct BufferClearMacros<'ast> {
    owner: Option<String>,
    found: Vec<BufferClearSite<'ast>>,
}

impl<'ast> Visit<'ast> for BufferClearMacros<'ast> {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let outer = self.owner.replace(function.sig.ident.to_string());
        visit::visit_item_fn(self, function);
        self.owner = outer;
    }

    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let mut strings = Vec::new();
        asm_strings(invocation.tokens.clone(), &mut strings);
        let mut path = invocation.path.segments.iter();
        let exact_path = invocation.path.leading_colon.is_some()
            && matches!(path.next(), Some(segment) if segment.ident == "core" && segment.arguments.is_empty())
            && matches!(path.next(), Some(segment) if segment.ident == "arch" && segment.arguments.is_empty());
        let name = path.next().map(|segment| segment.ident.to_string());
        if exact_path
            && path.next().is_none()
            && name
                .as_deref()
                .is_some_and(|name| matches!(name, "asm" | "global_asm" | "naked_asm"))
            && strings.iter().any(|value| value.contains("BUFFER_CLEARS"))
        {
            self.found.push(BufferClearSite {
                name: name.expect("checked assembly macro name"),
                owner: self.owner.clone(),
                invocation,
            });
        }
        visit::visit_macro(self, invocation);
    }
}

fn require_guarded_buffer_clear(
    source: &str,
    macro_name: &'static str,
    owner: Option<&str>,
    operand: &[&str],
) {
    const GUARDED_INCREMENT: &[&str] = &[
        ".if {record_clears}",
        "jnz 3f",
        "inc qword ptr [rip + BUFFER_CLEARS]",
        "3:",
        ".endif",
    ];

    let syntax = syn::parse_file(source).expect("parse buffer-clear assembly source");
    let mut macros = BufferClearMacros::default();
    macros.visit_file(&syntax);
    assert_eq!(
        macros.found.len(),
        1,
        "one architecture-assembly buffer-clear path"
    );
    let site = &macros.found[0];
    assert_eq!(site.name, macro_name, "buffer-clear assembly macro kind");
    assert_eq!(
        site.owner.as_deref(),
        owner,
        "buffer-clear assembly enclosure"
    );
    let invocation = site.invocation;
    let mut strings = Vec::new();
    asm_strings(invocation.tokens.clone(), &mut strings);
    assert_eq!(
        strings
            .iter()
            .filter(|value| value.contains("BUFFER_CLEARS"))
            .count(),
        1,
        "one buffer-clear symbol reference in the {macro_name} return path"
    );
    assert_eq!(
        strings
            .iter()
            .find(|value| value.contains("BUFFER_CLEARS"))
            .map(String::as_str),
        Some("inc qword ptr [rip + BUFFER_CLEARS]"),
        "the buffer-clear symbol has an unexpected assembly operation"
    );
    let start = strings
        .iter()
        .position(|value| value == GUARDED_INCREMENT[0])
        .expect("buffer-clear tally has no assembler condition");
    let guarded = strings
        .get(start..start + GUARDED_INCREMENT.len())
        .expect("buffer-clear assembler condition is incomplete");
    assert!(
        guarded
            .iter()
            .map(String::as_str)
            .eq(GUARDED_INCREMENT.iter().copied()),
        "buffer-clear tally is not wholly inside its assembler condition"
    );
    let mut flattened = Vec::new();
    asm_tokens(invocation.tokens.clone(), &mut flattened);
    assert_eq!(
        flattened
            .windows(operand.len())
            .filter(|window| {
                window
                    .iter()
                    .map(String::as_str)
                    .eq(operand.iter().copied())
            })
            .count(),
        1,
        "the {macro_name} tally has no unique self-test selector operand"
    );
}

fn plain_type(ty: &Type, expected: &str) -> bool {
    matches!(
        ty,
        Type::Path(path)
            if path.qself.is_none()
                && path.path.leading_colon.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].ident == expected
                && path.path.segments[0].arguments.is_empty()
    )
}

fn require_buffer_clear_state(source: &str) {
    let syntax = syn::parse_file(source).expect("parse buffer-clear state source");
    let selectors = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Const(item) if item.ident == "RECORD_BUFFER_CLEARS" => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(selectors.len(), 1, "one buffer-clear selector constant");
    let selector = selectors[0];
    assert!(matches!(
        &selector.vis,
        syn::Visibility::Restricted(visibility)
            if visibility.in_token.is_none() && visibility.path.is_ident("super")
    ));
    assert!(plain_type(&selector.ty, "usize"));
    let Expr::Cast(selector) = selector.expr.as_ref() else {
        panic!("buffer-clear selector is not derived from a cfg expression");
    };
    assert!(plain_type(&selector.ty, "usize"));
    let Expr::Macro(feature) = selector.expr.as_ref() else {
        panic!("buffer-clear selector is not a cfg expression");
    };
    assert!(feature.mac.path.is_ident("cfg"));
    assert!(
        feature
            .mac
            .parse_body::<syn::Meta>()
            .is_ok_and(|meta| matches!(
                meta,
                syn::Meta::NameValue(value)
                    if value.path.is_ident("feature")
                        && matches!(&value.value, Expr::Lit(value)
                            if matches!(&value.lit, syn::Lit::Str(name)
                                if name.value() == "selftest"))
            )),
        "buffer-clear selector is not the exact self-test feature"
    );

    let counters = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Static(item) if item.ident == "BUFFER_CLEARS" => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(counters.len(), 1, "one buffer-clear counter static");
    let counter = counters[0];
    assert!(selftest_cfg(&counter.attrs));
    assert!(counter.attrs.iter().any(|attribute| {
        attribute.path().is_ident("unsafe")
            && attribute
                .parse_args::<syn::Meta>()
                .is_ok_and(|meta| meta.path().is_ident("no_mangle"))
    }));
    assert!(matches!(counter.mutability, syn::StaticMutability::Mut(_)));
    assert!(plain_type(&counter.ty, "u64"));
    assert!(matches!(
        counter.expr.as_ref(),
        Expr::Lit(value)
            if matches!(&value.lit, syn::Lit::Int(value)
                if matches!(value.base10_parse::<u64>(), Ok(0)))
    ));
}

#[test]
fn machine_evidence_writes_are_absent_from_shipped_paths() {
    const OBSERVERS: &[(&str, &[(&str, usize)])] = &[
        ("kernel/src/dev.rs", &[("KEYBOARD_GSI", 3)]),
        (
            "kernel/src/input.rs",
            &[
                ("evidence", 7),
                ("dropped", 8),
                ("POLLER_ROUNDS", 3),
                ("FOREVER", 4),
                ("POLLER_PASSES", 3),
                ("INTERRUPTS", 3),
                ("WATCH", 4),
                ("POLLER_STARTED", 4),
                ("POLLER_SAW_PEER", 4),
            ],
        ),
        (
            "kernel/src/arch/x86_64/trap.rs",
            &[
                ("SPURIOUS_OBSERVATIONS", 3),
                ("USER_COPY_RECOVERIES", 3),
                ("LAST_USER_FAULT", 3),
                ("SMAP_ON_ENTRY", 3),
                ("LAST_SYSCALL", 3),
                ("LAST_BREAKPOINT", 3),
                ("LAST_PING", 4),
                ("KERNEL_STACK_SELECTOR_ON_ENTRY", 3),
                ("REJECTED_DUPLICATE_EOI_CLAIMS", 3),
                ("RECLAIMED_ABANDONED_EOI", 3),
            ],
        ),
        ("kernel/src/object.rs", &[("LIVE_CHANNELS", 4)]),
        (
            "kernel/src/time.rs",
            &[
                ("APIC_HZ", 3),
                ("LAST_WATCHDOG_TSC", 3),
                ("LAST_WATCHDOG_PIT", 3),
                ("LAST_WATCHDOG_EXPECTED", 3),
                ("EXPIRIES", 3),
            ],
        ),
        (
            "kernel/src/timepage.rs",
            &[("WINDOW_LEFT_PREEMPTIBLE", 3), ("WINDOW_NEVER_OPENED", 3)],
        ),
        (
            "kernel/src/queue.rs",
            &[
                ("settlement_refusals", 4),
                ("LIVE_QUEUES", 4),
                ("WORKER_REAPS", 3),
                ("WORKER_RETAINED", 3),
                ("WORKER_IS_PAUSED", 5),
                ("WORK_RUNS", 3),
            ],
        ),
        (
            "kernel/src/thread.rs",
            &[
                ("POLICY_FALLBACKS", 3),
                ("WAKES", 3),
                ("BLOCKS", 4),
                ("TRAP_WAITS", 3),
                ("IDLE_HALTS", 3),
                ("RETIREMENTS", 3),
                ("HANDLER_ARMS", 3),
            ],
        ),
        (
            "kernel/src/user.rs",
            &[
                ("RECLAIMED_PAGES", 4),
                ("RECLAIMED_TABLES", 3),
                ("REFUSED_TABLE_RECLAIMS", 3),
                ("REFUSED_RING_RECLAIMS", 3),
                ("ROLLED_BACK_PAGES", 3),
                ("ROLLED_BACK_TABLES", 3),
                ("RITES", 3),
                ("REFUSED_MAPPING_RECLAIMS", 4),
                ("REFUSED_TASK_RETIREMENTS", 3),
                ("BIRTHS", 3),
                ("COLLECTION_IS_PAUSED", 4),
                ("COLLECTION_IS_SETTLING", 4),
                ("RING_ENTER_ARMS", 3),
                ("observed_birth_handle", 7),
                ("observed_table_incarnation", 3),
                ("observed_returned_pages", 18),
            ],
        ),
        (
            "kernel/src/main.rs",
            &[
                ("EXTERNAL_MASK_REFUSAL_PROVED", 3),
                ("NVME_CANDIDATE_ATTEMPTS", 3),
                ("NVME_CLEAN_RETIREMENTS", 3),
                ("NVME_SELECTED_ORDINAL", 3),
                ("NVME_BME_REFUSAL_PROVED", 3),
                ("NVME_BAR_ADMISSION_PROVED", 3),
                ("ROUTING_ROLLBACK_PROVED", 3),
                ("CandidateBaseline", 5),
            ],
        ),
        (
            "kernel/src/pci.rs",
            &[("OBSERVED_BUS_MASTER_REFUSAL", 3), ("observed_address", 5)],
        ),
        (
            "kernel/src/arch/x86_64/i8042.rs",
            &[("CONTROL_COMMAND_REFUSED", 5)],
        ),
        (
            "kernel/src/nvme.rs",
            &[
                ("BME_PRECEDING_CC", 3),
                ("BME_PRECEDING_CSTS", 3),
                ("REFUSED_BME_PRECEDING_CC", 3),
                ("REFUSED_BME_PRECEDING_CSTS", 3),
                ("START_ATTEMPTS", 3),
                ("SUBMISSION_DOORBELLS", 3),
                ("COMMAND_HELD", 4),
                ("published_commands", 5),
                ("intercepted_read_submissions", 5),
                ("intercepted_io_queue_creations", 5),
                ("observed_window", 14),
            ],
        ),
        (
            "wost/src/arch/x86_64/paging.rs",
            &[
                ("RECLAIMED_REFUSALS", 6),
                ("CLEARED_ON_DROP", 3),
                ("PROBE_OBSERVATIONS", 7),
                ("PROBE_PARTIAL_OBSERVATIONS", 4),
                ("mapping_refusal", 3),
            ],
        ),
        ("wost/src/arch/x86_64/cpu.rs", &[("PAGE_INVALIDATIONS", 3)]),
        (
            "wost/src/arch/x86_64/apic.rs",
            &[("INIT_OBSERVATIONS", 4), ("INIT_OBSERVATION_FAILED", 4)],
        ),
        ("wost/src/sync.rs", &[("forgotten_for_test", 5)]),
        (
            "wost/src/arch/x86_64/percpu.rs",
            &[
                ("leaves_held", 5),
                ("under_a_leaf", 4),
                ("the_leaf", 4),
                ("last_leaf", 4),
            ],
        ),
        (
            "wost/src/mm/heap.rs",
            &[("slab_frames", 7), ("record_slab_frame", 2)],
        ),
        ("wost/src/mmio.rs", &[("base_address", 1)]),
    ];

    let paths = Paths::discover().expect("workspace paths");
    assert_eq!(
        OBSERVERS
            .iter()
            .map(|(_, observers)| observers.len())
            .sum::<usize>(),
        95,
        "D-290's evidence-identifier inventory"
    );
    for &(relative, expected) in OBSERVERS {
        require_selftest_observers(paths.root(), relative, expected);
    }
    require_selftest_parameter(paths.root(), "kernel/src/dev.rs", "keep_io_apic", 1);
    require_selftest_call_argument(paths.root(), "kernel/src/main.rs", "keep_io_apic", 1);
    require_split_mapping_write(paths.root());
}

#[test]
fn buffer_clear_state_is_selftest_only() {
    let paths = Paths::discover().expect("workspace paths");
    let interrupt = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/interrupt.rs"))
        .expect("read interrupt source");
    require_buffer_clear_state(&interrupt);
}

#[test]
fn buffer_clear_assembly_writes_are_guarded() {
    let paths = Paths::discover().expect("workspace paths");
    let interrupt = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/interrupt.rs"))
        .expect("read interrupt source");
    let task = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/task.rs"))
        .expect("read task source");
    require_guarded_buffer_clear(
        &interrupt,
        "global_asm",
        None,
        &["record_clears", "=", "const", "RECORD_BUFFER_CLEARS", ","],
    );
    require_guarded_buffer_clear(
        &task,
        "naked_asm",
        Some("leave_for_user"),
        &[
            "record_clears",
            "=",
            "const",
            "super",
            ":",
            ":",
            "interrupt",
            ":",
            ":",
            "RECORD_BUFFER_CLEARS",
            ",",
        ],
    );
}

struct ObservationEpoch<'a> {
    expected: &'a [&'a str],
    observations: Vec<String>,
    outside: Vec<String>,
    disables: usize,
    restores: usize,
    excluded: bool,
}

impl<'ast> Visit<'ast> for ObservationEpoch<'_> {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Expr::Path(function) = expression.func.as_ref() {
            let segments = &function.path.segments;
            if segments.len() >= 2
                && segments[segments.len() - 2].ident == "interrupt"
                && segments
                    .last()
                    .is_some_and(|segment| segment.ident == "disable")
            {
                self.disables += 1;
                self.excluded = true;
            } else if let Some(name) = segments.last().map(|segment| segment.ident.to_string())
                && self.expected.contains(&name.as_str())
            {
                if !self.excluded {
                    self.outside.push(name.clone());
                }
                self.observations.push(name);
            }
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        if expression.method == "restore"
            && matches!(expression.receiver.as_ref(), Expr::Path(path)
                if path.path.is_ident("disabled"))
        {
            self.restores += 1;
            self.excluded = false;
        }
        visit::visit_expr_method_call(self, expression);
    }
}

fn require_one_observation_epoch(block: &syn::Block, expected: &[&str], label: &str) {
    let mut epoch = ObservationEpoch {
        expected,
        observations: Vec::new(),
        outside: Vec::new(),
        disables: 0,
        restores: 0,
        excluded: false,
    };
    epoch.visit_block(block);

    let expected = expected
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    let observed = epoch.observations.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(epoch.disables, 1, "{label} needs one outer exclusion");
    assert_eq!(epoch.restores, 1, "{label} needs one matching restoration");
    assert!(!epoch.excluded, "{label} left its exclusion live");
    assert_eq!(
        epoch.observations.len(),
        expected.len(),
        "{label} duplicated an observation"
    );
    assert_eq!(
        observed, expected,
        "{label} lost an owned-resource observation"
    );
    assert!(
        epoch.outside.is_empty(),
        "{label} sampled outside its interrupt epoch: {:?}",
        epoch.outside
    );
}

#[test]
fn allocator_ledgers_hold_one_interrupt_epoch() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("kernel/src/selftest.rs"))
        .expect("read kernel selftest source");
    let syntax = syn::parse_file(&source).expect("parse kernel selftest source");

    let capture = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Impl(implementation)
                if matches!(implementation.self_ty.as_ref(), Type::Path(path)
                    if path.path.is_ident("AllocatorSnapshot")) =>
            {
                implementation.items.iter().find_map(|item| match item {
                    syn::ImplItem::Fn(function) if function.sig.ident == "capture" => {
                        Some(function)
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("AllocatorSnapshot has no capture method");
    require_one_observation_epoch(
        &capture.block,
        &["free_frames", "stats"],
        "allocator snapshot",
    );

    let resources = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "spawn_resources" => Some(function),
            _ => None,
        })
        .expect("kernel selftest has no spawn_resources ledger");
    require_one_observation_epoch(
        &resources.block,
        &[
            "capture",
            "arena_usage_for_test",
            "live_channels_for_test",
            "live_for_test",
            "free_slots",
            "process_records_for_test",
            "refused_mapping_reclaims",
            "refused_table_reclaims",
            "refused_task_retirements",
        ],
        "spawn resource ledger",
    );
}

#[test]
fn production_invariants_are_not_debug_only() {
    let paths = Paths::discover().expect("workspace paths");
    let mut pending = ["kernel/src", "wost/src"]
        .map(|directory| paths.root().join(directory))
        .into_iter()
        .collect::<Vec<_>>();

    while let Some(path) = pending.pop() {
        for entry in crate::paths::directory_entries(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
        {
            let source = entry.path();
            if entry.is_dir() {
                pending.push(source);
            } else if source
                .extension()
                .is_some_and(|extension| extension == "rs")
            {
                let text = std::fs::read_to_string(&source)
                    .unwrap_or_else(|error| panic!("reading {}: {error}", source.display()));
                let syntax = syn::parse_file(&text)
                    .unwrap_or_else(|error| panic!("parsing {}: {error}", source.display()));
                let mut assertions = DebugAssertions::default();
                assertions.visit_file(&syntax);
                assert!(
                    assertions.found == 0,
                    "{} makes an invariant debug-only",
                    source.display()
                );
            }
        }
    }
}

#[test]
fn known_fallible_results_are_not_discarded() {
    let paths = Paths::discover().expect("workspace paths");
    let forbidden = [
        ("kernel/src/user.rs", "let _ = interrupt::answer"),
        ("kernel/src/nvme.rs", "let _ = self.fail_submitted"),
        ("kernel/src/main.rs", "let _ = vfs::enumerate"),
        (
            "kernel/src/main.rs",
            "let _ = wost::arch::current::port::claim",
        ),
        ("kernel/src/selftest.rs", "let _ = vfs::enumerate"),
        ("kernel/src/selftest.rs", "let _ = pci::enumerate"),
        ("kernel/src/selftest.rs", "let _ = task::set_thread_base"),
        (
            "kernel/src/selftest.rs",
            "let _ = user::release_gated_cross",
        ),
        ("kernel/src/selftest.rs", "let _ = mem::unmap_for_test"),
        ("kernel/src/selftest.rs", "let _ = runs_until"),
        ("kernel/src/selftest.rs", "let _ = yields_until"),
        ("kernel/src/arch/x86_64/i8042.rs", "let _ = self.flush"),
        (
            "kernel/src/thread.rs",
            "let _ = IDLE_STATE.compare_exchange",
        ),
        ("wost/src/boot/mod.rs", "let _ = self.region.write_u32"),
        ("wost/src/boot/mod.rs", "let _ = self.region.fill_u32"),
        ("wost/src/boot/mod.rs", ".unwrap_or(0)"),
        ("kernel/src/user.rs", ".saturating_add(abandoned)"),
    ];

    for (relative, discarded) in forbidden {
        let source = std::fs::read_to_string(paths.root().join(relative))
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        assert!(
            !source.contains(discarded),
            "{relative} silently discards {discarded:?}"
        );
    }

    for source in rust_sources_below(&paths.root().join("kernel/src")) {
        let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
        let discarded = discarded_address_space_reclaims(&text)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
        assert!(
            discarded.is_empty(),
            "{} discards AddressSpace::reclaim on lines {discarded:?}",
            relative.display()
        );
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

#[test]
fn discarded_address_space_reclaims_are_found_without_receiver_names() {
    for source in [
        "fn probe(space: Space) { let _ = space.reclaim(None); }",
        "fn probe(observer: Space) { observer.reclaim(None); }",
        "fn probe(renamed: Space) { _ = (renamed.reclaim(None)); }",
        "fn probe(target: Space) { core::mem::drop(target.reclaim(None)); }",
        "fn probe(candidate: Space) { if let _ = candidate.reclaim(None) {} }",
    ] {
        assert_eq!(
            discarded_address_space_reclaims(source).expect("parse hostile reclaim fixture"),
            vec![1]
        );
    }
}

#[test]
fn observed_address_space_reclaims_are_not_reported_as_discarded() {
    let source = r#"
fn probe(mut observer: Space) -> bool {
    if let Err(reason) = observer.reclaim(None) {
        report(reason);
    }
    let reclaimed = observer.reclaim(None).is_ok();
    matches!(observer.reclaim(None), Ok(_)) && reclaimed
}
"#;
    assert!(
        discarded_address_space_reclaims(source)
            .expect("parse observed reclaim fixture")
            .is_empty()
    );
}

#[derive(Default)]
struct AddressSpaceBindingCalls {
    owner: Option<String>,
    calls: Vec<String>,
    bound_stores: Vec<String>,
    bound_releases: Vec<String>,
    released_reclaims: Vec<String>,
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

fn expression_is_call(expression: &Expr, path: &[&str], argument: &str) -> bool {
    match expression {
        Expr::Group(group) => expression_is_call(&group.expr, path, argument),
        Expr::Paren(parenthesized) => expression_is_call(&parenthesized.expr, path, argument),
        Expr::Call(call) => {
            call.args.len() == 1
                && matches!(call.func.as_ref(), Expr::Path(function)
                    if function.qself.is_none() && path_ends_with(&function.path, path))
                && expression_is_ident(&call.args[0], argument)
        }
        _ => false,
    }
}

fn expression_is_bound_store(expression: &Expr) -> bool {
    let Expr::Call(some) = expression else {
        return false;
    };
    if some.args.len() != 1
        || !matches!(some.func.as_ref(), Expr::Path(function)
            if function.qself.is_none() && path_ends_with(&function.path, &["Some"]))
    {
        return false;
    }
    expression_is_call(&some.args[0], &["BootstrapAuthority", "Bound"], "bound")
}

fn expression_is_bootstrap_space(expression: &Expr) -> bool {
    matches!(expression,
        Expr::Field(field)
            if matches!(&field.member, syn::Member::Named(member) if member == "space")
                && expression_is_ident(&field.base, "bootstrap"))
}

fn expression_is_some_released(expression: &Expr) -> bool {
    let Expr::Call(some) = expression else {
        return false;
    };
    some.args.len() == 1
        && matches!(some.func.as_ref(), Expr::Path(function)
            if function.qself.is_none() && path_ends_with(&function.path, &["Some"]))
        && expression_is_ident(&some.args[0], "released")
}

impl<'ast> Visit<'ast> for AddressSpaceBindingCalls {
    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let previous = self.owner.replace(function.sig.ident.to_string());
        visit::visit_item_fn(self, function);
        self.owner = previous;
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        let previous = self.owner.replace(function.sig.ident.to_string());
        visit::visit_impl_item_fn(self, function);
        self.owner = previous;
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "bind" && call.args.is_empty() {
            self.calls
                .push(self.owner.clone().unwrap_or_else(|| "<module>".into()));
        }
        if call.method == "reclaim"
            && call.args.len() == 1
            && expression_is_bootstrap_space(&call.receiver)
            && expression_is_some_released(&call.args[0])
        {
            self.released_reclaims
                .push(self.owner.clone().unwrap_or_else(|| "<module>".into()));
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if call.args.len() == 1
            && matches!(call.func.as_ref(), Expr::Path(function)
                if path_ends_with(&function.path, &["bind"]))
        {
            self.calls
                .push(self.owner.clone().unwrap_or_else(|| "<module>".into()));
        }
        if matches!(call.func.as_ref(), Expr::Path(function)
            if function.qself.is_none()
                && path_ends_with(&function.path, &["task", "release_address_space"]))
            && call.args.len() == 1
            && expression_is_ident(&call.args[0], "bound")
        {
            self.bound_releases
                .push(self.owner.clone().unwrap_or_else(|| "<module>".into()));
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
        if expression.qself.is_none()
            && path_ends_with(&expression.path, &["Bootstrap"])
            && expression.fields.iter().any(|field| {
                matches!(&field.member, syn::Member::Named(member) if member == "authority")
                    && expression_is_bound_store(&field.expr)
            })
        {
            self.bound_stores
                .push(self.owner.clone().unwrap_or_else(|| "<module>".into()));
        }
        visit::visit_expr_struct(self, expression);
    }
}

fn address_space_binding_calls(source: &str) -> syn::Result<AddressSpaceBindingCalls> {
    let syntax = syn::parse_file(source)?;
    let mut found = AddressSpaceBindingCalls::default();
    found.visit_file(&syntax);
    Ok(found)
}

#[test]
fn one_production_owner_binds_and_retires_each_address_space() {
    let paths = Paths::discover().expect("workspace paths");
    let mut calls = Vec::new();
    for source in rust_sources_below(&paths.root().join("kernel/src")) {
        let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
        if relative == std::path::Path::new("kernel/src/selftest.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
        let found = address_space_binding_calls(&text)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
        calls.extend(
            found
                .calls
                .into_iter()
                .map(|owner| (relative.to_string_lossy().replace('\\', "/"), owner)),
        );
    }
    calls.sort();
    assert_eq!(
        calls,
        [("kernel/src/user.rs".into(), "spawn_transaction".into())],
        "production address-space binding owners changed"
    );

    let user = std::fs::read_to_string(paths.root().join("kernel/src/user.rs"))
        .expect("user process source");
    let lifecycle = address_space_binding_calls(&user).expect("parse user process source");
    assert_eq!(lifecycle.bound_stores, ["commit"]);
    assert_eq!(lifecycle.bound_releases, ["reclaim"]);
    assert_eq!(lifecycle.released_reclaims, ["reclaim"]);
}

#[test]
fn binding_inventory_sees_ufcs_and_ignores_prose_decoys() {
    let found = address_space_binding_calls(
        r#"
fn real(space: &mut AddressSpace) { AddressSpace::bind(space); }
fn decoys() {
    let _ = ".bind() AddressSpace::bind(space)";
    // another.bind()
}
"#,
    );
    assert_eq!(found.expect("parse binding fixture").calls, ["real"]);
}

#[test]
fn speculative_and_callback_authority_does_not_regrow() {
    let paths = Paths::discover().expect("workspace paths");
    let forbidden = [
        ("kernel/src/fs.rs", "fn with_volume"),
        ("kernel/src/nvme.rs", "fn with_controller"),
        ("kernel/src/tmpfs.rs", "fn walk("),
        ("libs/acpi-tables/src/lib.rs", "pub fn checksum_is_valid"),
        ("libs/acpi-tables/src/lib.rs", "pub fn fix_checksum"),
        ("libs/fat32/src/lib.rs", "pub const fn decode_entry"),
        ("libs/fat32/src/lib.rs", "pub const fn decode_slot"),
        ("libs/fat32/src/lib.rs", "pub const fn sector_of"),
        ("libs/fat32/src/lib.rs", "pub const fn table_position"),
        ("libs/memory-map/src/lib.rs", "pub fn allocatable_bytes"),
        ("libs/path-walk/src/lib.rs", "pub fn is_a_name"),
        ("libs/pci-config/src/lib.rs", "pub const fn io_window_size"),
        ("libs/pci-config/src/lib.rs", "fn subordinate_bus"),
        ("libs/stack-layout/src/lib.rs", "fn stack_overlaps"),
        ("wost/src/arch/x86_64/pci.rs", "fn write_command"),
        ("wost/src/mm/mod.rs", "fn first_region"),
        ("wost/src/mm/mod.rs", "fn past_every_region"),
    ];

    for (relative, surface) in forbidden {
        let source = std::fs::read_to_string(paths.root().join(relative))
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        assert!(
            !source.contains(surface),
            "{relative} regrew speculative/callback surface {surface:?}"
        );
    }
}

#[test]
fn pci_command_transactions_hold_the_port_pair_once() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/pci.rs"))
        .expect("PCI source");
    for (start, algorithm) in [
        (
            "pub fn admit_bus_master",
            "pci_config::admit_mapped_bus_master",
        ),
        ("pub fn restore_command", "pci_config::restore_command"),
        ("pub fn commit_command", "pci_config::commit_command"),
        ("pub fn probe_bars", "pci_config::probe_bars"),
        ("pub fn retry_probe", "pci_config::retry_probe"),
    ] {
        let transaction = source
            .split_once(start)
            .unwrap_or_else(|| panic!("missing {start}"))
            .1
            .split_once("\n}\n")
            .expect("complete PCI transaction")
            .0;
        assert_eq!(
            transaction.matches("CONFIGURATION.lock_leaf()").count(),
            1,
            "{start} does not own one continuous port interval"
        );
        assert!(
            transaction.contains(algorithm),
            "{start} bypasses the verified pure transaction"
        );
    }
}

#[test]
fn fallible_operation_bools_stay_must_use() {
    let paths = Paths::discover().expect("workspace paths");
    let required = [
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused firmware read copied no bytes\"]\n    pub fn read",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused frame fill changed no bytes\"]\n    pub fn fill",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused frame clear changed no bytes\"]\n    pub fn zero",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused segment fill changed no bytes\"]\n    pub fn fill",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused segment clear changed no bytes\"]\n    pub fn zero",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused segment write changed no bytes\"]\n    pub fn write_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused segment read copied no bytes\"]\n    pub fn read_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused DMA-segment fill changed no bytes\"]\n    pub fn fill",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused DMA-segment clear changed no bytes\"]\n    pub fn zero",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused DMA-segment write changed no bytes\"]\n    pub fn write_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused DMA-segment read copied no bytes\"]\n    pub fn read_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused shared-memory write changed no bytes\"]\n    pub fn write_volatile_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused shared-memory read copied no bytes\"]\n    pub fn read_volatile_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused ring-payload write changed no bytes\"]\n    pub fn write_payload_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused ring-payload read copied no bytes\"]\n    pub fn read_payload_at",
        ),
        (
            "wost/src/mm/mod.rs",
            "#[must_use = \"a refused shared-memory publication stored no index\"]\n    pub fn store_release_u32",
        ),
        (
            "wost/src/boot/mod.rs",
            "#[must_use = \"a refused framebuffer shadow was not attached\"]\n    pub fn attach_back_buffer",
        ),
        (
            "kernel/src/console.rs",
            "#[must_use = \"a refused console shadow was not attached\"]\n    pub(crate) fn attach_back_buffer",
        ),
        (
            "wost/src/arch/x86_64/cpu.rs",
            "#[must_use = \"a refused PAT commit restores the previous entry on drop\"]\n    pub fn commit",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            "#[must_use = \"a refused resume changed no trap frame\"]\npub(super) fn resume_usercopy",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            "#[must_use = \"a refused resume changed no trap frame\"]\npub(super) fn resume_write_protection",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            "#[must_use = \"a refused answer changed no trap frame\"]\npub fn answer",
        ),
        (
            "wost/src/arch/x86_64/pci.rs",
            "#[must_use = \"a refused PCI initialization left configuration space unavailable\"]\npub fn init",
        ),
        (
            "wost/src/arch/x86_64/segment.rs",
            "#[must_use = \"a refused double-fault stack install changed no TSS entry\"]\npub fn set_double_fault_stack",
        ),
        (
            "wost/src/arch/x86_64/usercopy.rs",
            "#[must_use = \"a refused user-copy recovery changed no trap frame\"]\npub fn recover",
        ),
        (
            "wost/src/arch/x86_64/task.rs",
            "#[must_use = \"a refused thread-base write changed no register\"]\npub fn set_thread_base",
        ),
        (
            "wost/src/mm/heap.rs",
            "#[must_use = \"a refused slot return changed no heap state\"]\n    unsafe fn give_slot",
        ),
        (
            "wost/src/mm/heap.rs",
            "#[must_use = \"a refused slab refill changed no heap state\"]\n    unsafe fn refill_with",
        ),
        (
            "kernel/src/handle.rs",
            "#[must_use = \"a refused quota transition changed no count\"]\n    fn step",
        ),
        (
            "kernel/src/timepage.rs",
            "#[must_use = \"a deferred publication must be observed\"]\npub(crate) enum Publication",
        ),
    ];

    for (relative, marker) in required {
        let source = std::fs::read_to_string(paths.root().join(relative))
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        assert!(
            source.contains(marker),
            "{relative} lets an operation refusal be discarded"
        );
    }
}

#[derive(Default)]
struct DebugAssertions {
    found: usize,
}

impl<'ast> Visit<'ast> for DebugAssertions {
    fn visit_macro(&mut self, item: &'ast Macro) {
        if item.path.segments.last().is_some_and(|segment| {
            segment.ident == "debug_assert"
                || segment.ident == "debug_assert_eq"
                || segment.ident == "debug_assert_ne"
        }) {
            self.found += 1;
        }
        visit::visit_macro(self, item);
    }
}

#[test]
fn a_live_spin_guard_keeps_the_thread_context_borrowed() {
    const SECOND_LOCK: &str = r"#![no_std]
use wost::sync::{SpinLock, ThreadContext};
static FIRST: SpinLock<u8> = SpinLock::new(0);
static SECOND: SpinLock<u8> = SpinLock::new(0);
fn probe(context: &mut ThreadContext) {
    let _first = FIRST.lock(context);
    let _second = SECOND.lock(context);
}
";
    const CONTEXT_REUSE: &str = r"#![no_std]
use wost::sync::{SpinLock, ThreadContext};
static LOCK: SpinLock<u8> = SpinLock::new(0);
fn may_block(_: &mut ThreadContext) {}
fn probe(context: &mut ThreadContext) {
    let _guard = LOCK.lock(context);
    may_block(context);
}
";
    const RELEASED_CONTEXT: &str = r"#![no_std]
use wost::sync::{SpinLock, ThreadContext};
static LOCK: SpinLock<u8> = SpinLock::new(0);
fn may_block(_: &mut ThreadContext) {}
fn probe(context: &mut ThreadContext) {
    let guard = LOCK.lock(context);
    drop(guard);
    may_block(context);
}
";

    let paths = Paths::discover().expect("workspace paths");
    require_error(&paths, "second-lock", SECOND_LOCK, "E0499")
        .expect("a second token-bearing lock did not produce exactly E0499");
    require_error(&paths, "context-reuse", CONTEXT_REUSE, "E0499")
        .expect("a live guard did not make the blocking token exactly E0499");
    require_success(&paths, "released-context", RELEASED_CONTEXT)
        .expect("dropping the guard did not release the blocking token");
}

#[test]
fn a_kernel_stack_top_cannot_outlive_its_pin() {
    const STALE_TOP: &str = r"#![no_std]
struct StackIdentity;
struct TaskStackPin(StackIdentity);
struct KernelStackTop<'pin>(&'pin StackIdentity);
impl TaskStackPin {
    fn stack_top(&self) -> KernelStackTop<'_> { KernelStackTop(&self.0) }
}
fn write_kernel_stack(_: KernelStackTop<'_>) {}
fn probe(pin: TaskStackPin) {
    let top = pin.stack_top();
    drop(pin);
    write_kernel_stack(top);
}
";
    const LIVE_TOP: &str = r"#![no_std]
struct StackIdentity;
struct TaskStackPin(StackIdentity);
struct KernelStackTop<'pin>(&'pin StackIdentity);
impl TaskStackPin {
    fn stack_top(&self) -> KernelStackTop<'_> { KernelStackTop(&self.0) }
}
fn write_kernel_stack(_: KernelStackTop<'_>) {}
fn probe(pin: &TaskStackPin) { write_kernel_stack(pin.stack_top()); }
";

    let paths = Paths::discover().expect("workspace paths");
    let paging = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/paging.rs"))
        .expect("paging source");
    let syntax = syn::parse_file(&paging).expect("parse paging source");
    let stack_top = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "KernelStackTop" => Some(item),
            _ => None,
        })
        .expect("paging has no KernelStackTop authority");
    assert!(matches!(
        stack_top.generics.params.first(),
        Some(GenericParam::Lifetime(lifetime)) if lifetime.lifetime.ident == "pin"
    ));
    assert_eq!(stack_top.generics.params.len(), 1);
    let Fields::Unnamed(fields) = &stack_top.fields else {
        panic!("KernelStackTop does not directly borrow its stack identity");
    };
    let mut fields = fields.unnamed.iter();
    let Some(field) = core::iter::Iterator::next(&mut fields) else {
        panic!("KernelStackTop does not carry exactly one stack-identity borrow");
    };
    assert!(core::iter::Iterator::next(&mut fields).is_none());
    assert!(matches!(
        &field.ty,
        Type::Reference(reference)
            if reference.lifetime.as_ref().is_some_and(|lifetime| lifetime.ident == "pin")
                && matches!(
                    reference.elem.as_ref(),
                    Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment| segment.ident == "StackIdentity")
                )
    ));
    assert!(
        paging
            .contains("pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>>")
    );
    assert!(paging.contains(
        r"        let identity = self.stacks.get(index)?;
        if !is_active() {
            return None;
        }
        let current = cpu::page_table_base();
        identity
            .invalid_page_in(current)
            .is_none()
            .then_some(KernelStackTop(identity))"
    ));
    assert!(!paging.contains("impl Drop for TaskStackPin"));
    assert_eq!(paging.matches("TASK_STACK_PIN").count(), 4);
    assert_eq!(paging.matches("TASK_STACK_PIN.lock_leaf()").count(), 2);
    assert_eq!(
        paging.matches("TASK_STACK_PIN.force_unlock(panic)").count(),
        1
    );
    assert!(paging.contains("*pinned = Some((self.root, self.layout));"));
    assert!(!paging.contains("*pinned = None;"));
    assert!(
        paging.contains(
            "if let Some(pinned) = protected_stack_overlap(self.root(), virtual_base, len)"
        )
    );

    let task = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/task.rs"))
        .expect("task source");
    assert!(task.contains("stack_index: Option<usize>"));
    assert!(!task.contains("stack_top: Option<KernelStackTop"));
    assert!(task.contains("self.stacks.as_ref()?.stack_top(index)"));
    let segment = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/segment.rs"))
        .expect("segment source");
    assert!(segment.contains("pub(super) fn write_kernel_stack(top: KernelStackTop<'_>)"));

    require_error(&paths, "stale-kernel-stack-top", STALE_TOP, "E0505")
        .unwrap_or_else(|diagnostics| panic!("a stack top outlived its pin:\n{diagnostics}"));
    require_success(&paths, "live-kernel-stack-top", LIVE_TOP).unwrap_or_else(|diagnostics| {
        panic!("a live pin could not authorize RSP0:\n{diagnostics}")
    });
}

#[derive(Debug, PartialEq, Eq)]
enum DeviceLedgerEvent {
    Reserve,
    FindGap,
    Map,
    Publish,
}

#[derive(Default)]
struct DeviceLedgerOrder {
    events: Vec<DeviceLedgerEvent>,
    direct_fits: usize,
    fixed_arrays: usize,
    mapping_started: bool,
    post_map_tries: usize,
}

impl<'ast> Visit<'ast> for DeviceLedgerOrder {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if matches!(expression.func.as_ref(), Expr::Path(function)
            if function.path.segments.last().is_some_and(|segment| segment.ident == "first_fit_by"))
        {
            self.events.push(DeviceLedgerEvent::FindGap);
            if matches!(expression.args.iter().nth(3), Some(Expr::Reference(reference))
                if matches!(reference.expr.as_ref(), Expr::Field(field)
                    if is_named_path(&field.base, "self")
                        && matches!(&field.member, syn::Member::Named(member) if member == "devices")))
            {
                self.direct_fits += 1;
            }
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        let event = if expression.method == "try_reserve" {
            Some(DeviceLedgerEvent::Reserve)
        } else if expression.method == "map_range" {
            self.mapping_started = true;
            Some(DeviceLedgerEvent::Map)
        } else if expression.method == "push" {
            Some(DeviceLedgerEvent::Publish)
        } else {
            None
        };
        if let Some(event) = event {
            self.events.push(event);
        }
        visit::visit_expr_method_call(self, expression);
    }

    fn visit_expr_array(&mut self, expression: &'ast syn::ExprArray) {
        self.fixed_arrays += 1;
        visit::visit_expr_array(self, expression);
    }

    fn visit_expr_repeat(&mut self, expression: &'ast syn::ExprRepeat) {
        self.fixed_arrays += 1;
        visit::visit_expr_repeat(self, expression);
    }

    fn visit_expr_try(&mut self, expression: &'ast syn::ExprTry) {
        if self.mapping_started {
            self.post_map_tries += 1;
        }
        visit::visit_expr_try(self, expression);
    }
}

#[test]
fn an_address_space_keeps_its_bounded_ledger_off_the_stack() {
    const COMPACT_ADDRESS_SPACE: &str = r"#![no_std]
use core::mem::size_of;
use wost::arch::current::paging::AddressSpace;
const _: () = assert!(size_of::<AddressSpace>() <= 256);
";

    let paths = Paths::discover().expect("workspace paths");
    let paging = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/paging.rs"))
        .expect("paging source");
    let syntax = syn::parse_file(&paging).expect("parse paging source");
    let address_space = syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "AddressSpace" => Some(item),
            _ => None,
        })
        .expect("paging has no AddressSpace");
    let Fields::Named(fields) = &address_space.fields else {
        panic!("AddressSpace has no named ownership fields");
    };
    let devices = fields
        .named
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|ident| ident == "devices"))
        .expect("AddressSpace has no device ledger");
    let Type::Path(vector) = &devices.ty else {
        panic!("AddressSpace device ledger is not a Vec");
    };
    let Some(vector) = vector.path.segments.last() else {
        panic!("AddressSpace device ledger has no type path");
    };
    assert_eq!(
        vector.ident, "Vec",
        "AddressSpace inlines its device ledger"
    );
    let PathArguments::AngleBracketed(arguments) = &vector.arguments else {
        panic!("AddressSpace device ledger Vec has no element type");
    };
    let mut arguments = arguments.args.iter();
    let Some(GenericArgument::Type(Type::Path(option))) =
        core::iter::Iterator::next(&mut arguments)
    else {
        panic!("AddressSpace device ledger does not preserve stable slots");
    };
    assert!(core::iter::Iterator::next(&mut arguments).is_none());
    let Some(option) = option.path.segments.last() else {
        panic!("AddressSpace device slot has no type path");
    };
    assert_eq!(option.ident, "Option");
    let PathArguments::AngleBracketed(arguments) = &option.arguments else {
        panic!("AddressSpace device slot has no extent type");
    };
    let mut arguments = arguments.args.iter();
    assert!(matches!(
        core::iter::Iterator::next(&mut arguments),
        Some(GenericArgument::Type(Type::Path(extent)))
            if extent.path.segments.last().is_some_and(|segment| segment.ident == "DeviceExtent")
    ));
    assert!(core::iter::Iterator::next(&mut arguments).is_none());

    let map_device = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(item) => Some(item),
            _ => None,
        })
        .flat_map(|item| item.items.iter())
        .find_map(|item| match item {
            syn::ImplItem::Fn(function) if function.sig.ident == "map_device_range" => {
                Some(function)
            }
            _ => None,
        })
        .expect("AddressSpace has no shared device mapping implementation");
    let mut order = DeviceLedgerOrder::default();
    Visit::visit_impl_item_fn(&mut order, map_device);
    assert_eq!(
        order.events,
        [
            DeviceLedgerEvent::Reserve,
            DeviceLedgerEvent::FindGap,
            DeviceLedgerEvent::Map,
            DeviceLedgerEvent::Publish,
        ],
        "device-ledger allocation is not fallible and complete before mapping publication"
    );
    assert_eq!(
        order.direct_fits, 1,
        "device gap selection does not read the ownership ledger directly"
    );
    assert_eq!(
        order.fixed_arrays, 0,
        "device mapping rebuilds a fixed projection on the kernel stack"
    );
    assert_eq!(
        order.post_map_tries, 0,
        "device mapping retains a fallible step after page-table commit"
    );
    assert!(paging.contains("devices: Vec::new()"));

    require_success(&paths, "compact-address-space", COMPACT_ADDRESS_SPACE).unwrap_or_else(
        |diagnostics| panic!("AddressSpace regained an inline ledger:\n{diagnostics}"),
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WaitOwnership {
    CannotWait,
    CompletionPlane,
}

struct DirectDoor {
    call: &'static str,
    ownership: WaitOwnership,
}

const DIRECT_DOORS: &[DirectDoor] = &[
    DirectDoor {
        call: "Exit",
        ownership: WaitOwnership::CannotWait,
    },
    DirectDoor {
        call: "RingEnter",
        ownership: WaitOwnership::CannotWait,
    },
    DirectDoor {
        call: "RingWait",
        ownership: WaitOwnership::CompletionPlane,
    },
    DirectDoor {
        call: "ThreadBase",
        ownership: WaitOwnership::CannotWait,
    },
    DirectDoor {
        call: "Unknown",
        ownership: WaitOwnership::CannotWait,
    },
];

struct CallDispatch<'ast> {
    arms: BTreeMap<String, &'ast syn::Arm>,
}

impl<'ast> Visit<'ast> for CallDispatch<'ast> {
    fn visit_expr_match(&mut self, expression: &'ast ExprMatch) {
        if is_call_decode(&expression.expr) {
            assert!(self.arms.is_empty(), "more than one Call::decode dispatch");
            for arm in &expression.arms {
                let call = call_pattern(&arm.pat)
                    .unwrap_or_else(|| panic!("Call::decode has a non-Call or wildcard arm"));
                assert!(
                    self.arms.insert(call.clone(), arm).is_none(),
                    "Call::{call} has more than one dispatch arm"
                );
            }
        }
        visit::visit_expr_match(self, expression);
    }
}

fn is_call_decode(expression: &Expr) -> bool {
    let Expr::Call(call) = expression else {
        return false;
    };
    let Expr::Path(function) = call.func.as_ref() else {
        return false;
    };
    let mut segments = function.path.segments.iter();
    matches!(
        (
            core::iter::Iterator::next(&mut segments),
            core::iter::Iterator::next(&mut segments),
            core::iter::Iterator::next(&mut segments),
        ),
        (Some(first), Some(second), None) if first.ident == "Call" && second.ident == "decode"
    )
}

fn call_pattern(pattern: &Pat) -> Option<String> {
    let path = match pattern {
        Pat::Path(pattern) => &pattern.path,
        Pat::TupleStruct(pattern) => &pattern.path,
        _ => return None,
    };
    let mut segments = path.segments.iter();
    let (Some(first), Some(second), None) = (
        core::iter::Iterator::next(&mut segments),
        core::iter::Iterator::next(&mut segments),
        core::iter::Iterator::next(&mut segments),
    ) else {
        return None;
    };
    (first.ident == "Call").then(|| second.ident.to_string())
}

fn call_enum(items: &[Item]) -> &ItemEnum {
    items
        .iter()
        .find_map(|item| match item {
            Item::Enum(item) if item.ident == "Call" => Some(item),
            _ => None,
        })
        .expect("kernel::user has no Call enum")
}

fn call_function(items: &[Item]) -> &ItemFn {
    items
        .iter()
        .find_map(|item| match item {
            Item::Fn(item) if item.sig.ident == "call" => Some(item),
            _ => None,
        })
        .expect("kernel::user has no call dispatcher")
}

struct DirectCalls {
    functions: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for DirectCalls {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Expr::Path(function) = expression.func.as_ref()
            && function.path.segments.len() == 1
        {
            self.functions
                .insert(function.path.segments[0].ident.to_string());
        }
        visit::visit_expr_call(self, expression);
    }
}

fn called_functions(arm: &syn::Arm) -> BTreeSet<String> {
    let mut calls = DirectCalls {
        functions: BTreeSet::new(),
    };
    <DirectCalls as Visit<'_>>::visit_expr(&mut calls, &arm.body);
    calls.functions
}

#[test]
fn every_direct_door_has_an_explicit_wait_owner() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("kernel/src/user.rs"))
        .expect("read kernel user calls");
    let syntax = syn::parse_file(&source).expect("parse kernel user calls");

    let variants = call_enum(&syntax.items)
        .variants
        .iter()
        .map(|variant| variant.ident.to_string())
        .collect::<BTreeSet<_>>();
    let mut contracts = BTreeMap::new();
    for door in DIRECT_DOORS {
        assert!(
            contracts
                .insert(door.call.to_owned(), door.ownership)
                .is_none(),
            "Call::{} has duplicate wait-ownership contracts",
            door.call
        );
    }
    assert_eq!(
        variants,
        contracts.keys().cloned().collect::<BTreeSet<_>>(),
        "every direct Call needs an explicit wait-ownership contract"
    );

    let mut dispatch = CallDispatch {
        arms: BTreeMap::new(),
    };
    Visit::visit_item_fn(&mut dispatch, call_function(&syntax.items));
    assert_eq!(
        variants,
        dispatch.arms.keys().cloned().collect::<BTreeSet<_>>(),
        "every direct Call needs one explicit dispatch arm"
    );

    for (call, ownership) in contracts {
        let functions = called_functions(dispatch.arms[&call]);
        let reaches_completion_wait = functions.contains("wait_ring");
        match ownership {
            WaitOwnership::CannotWait => assert!(
                !reaches_completion_wait,
                "Call::{call} is classified CannotWait but invokes wait_ring"
            ),
            WaitOwnership::CompletionPlane => assert!(
                reaches_completion_wait,
                "Call::{call} owns completion waiting but does not invoke wait_ring"
            ),
        }
    }
}

/// Every `static` spin or thread lock in `wost`, keyed by its module file.
fn wost_static_locks(paths: &Paths) -> BTreeMap<String, BTreeSet<String>> {
    let mut locks = BTreeMap::new();
    for source in rust_sources_below(&paths.root().join("wost/src")) {
        let relative = source
            .strip_prefix(paths.root())
            .unwrap_or(&source)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        let file =
            syn::parse_file(&text).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
        for item in &file.items {
            let Item::Static(item) = item else { continue };
            let Type::Path(path) = &*item.ty else {
                continue;
            };
            let Some(last) = path.path.segments.last() else {
                continue;
            };
            if last.ident == "SpinLock" || last.ident == "ThreadLock" {
                locks
                    .entry(relative.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(item.ident.to_string());
            }
        }
    }
    locks
}

/// The statics a module's `force_unlock` hands to the panic path.
fn force_unlocked_statics(text: &str) -> Option<BTreeSet<String>> {
    let file = syn::parse_file(text).ok()?;
    let function = file.items.iter().find_map(|item| match item {
        Item::Fn(function) if function.sig.ident == "force_unlock" => Some(function),
        _ => None,
    })?;
    let mut statics = BTreeSet::new();
    for statement in &function.block.stmts {
        let expression = match statement {
            syn::Stmt::Expr(expression, _) => expression,
            _ => continue,
        };
        let Expr::MethodCall(call) = expression else {
            continue;
        };
        if call.method != "force_unlock" {
            continue;
        }
        let Expr::Path(receiver) = &*call.receiver else {
            continue;
        };
        if let Some(name) = receiver.path.get_ident() {
            statics.insert(name.to_string());
        }
    }
    Some(statics)
}

/// The module paths `panic::reclaim` unlocks, as `crate::…` strings.
fn reclaimed_modules(text: &str) -> BTreeSet<String> {
    let file = syn::parse_file(text).expect("parse wost panic source");
    let reclaim = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "reclaim" => Some(function),
            _ => None,
        })
        .expect("wost panic::reclaim");
    let mut modules = BTreeSet::new();
    for statement in &reclaim.block.stmts {
        let syn::Stmt::Expr(Expr::Call(call), _) = statement else {
            continue;
        };
        let Expr::Path(callee) = &*call.func else {
            continue;
        };
        let segments: Vec<String> = callee
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        if segments.last().map(String::as_str) == Some("force_unlock") {
            modules.insert(segments[..segments.len() - 1].join("::"));
        }
    }
    modules
}

#[test]
fn every_static_lock_in_wost_is_reclaimed_by_the_panic_path() {
    let paths = Paths::discover().expect("workspace paths");
    let locks = wost_static_locks(&paths);
    assert!(!locks.is_empty(), "no static locks were found in wost");
    let panic_source =
        std::fs::read_to_string(paths.root().join("wost/src/panic.rs")).expect("wost panic source");
    let reclaimed = reclaimed_modules(&panic_source);

    for (relative, statics) in &locks {
        let module = relative
            .trim_start_matches("wost/src/")
            .trim_end_matches(".rs")
            .trim_end_matches("/mod")
            .replace("arch/x86_64", "arch::current")
            .replace('/', "::");
        let module = format!("crate::{module}");
        assert!(
            reclaimed.contains(&module),
            "panic::reclaim does not unlock {module} ({relative} holds {statics:?})"
        );
        let text = std::fs::read_to_string(paths.root().join(relative))
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        let unlocked = force_unlocked_statics(&text)
            .unwrap_or_else(|| panic!("{relative} has static locks and no force_unlock"));
        assert_eq!(
            &unlocked, statics,
            "{relative}: force_unlock hands the panic path {unlocked:?}, but the module holds {statics:?}"
        );
    }
    assert_eq!(
        reclaimed.len(),
        locks.len(),
        "panic::reclaim names modules without static locks: {reclaimed:?} vs {:?}",
        locks.keys().collect::<Vec<_>>()
    );
}
