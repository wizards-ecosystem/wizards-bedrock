//! Compile-time and source-shape contracts that stable Rust cannot assert directly.

use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use syn::visit::{self, Visit};
use syn::{
    Expr, ExprCall, ExprForLoop, ExprMatch, ExprMethodCall, Fields, GenericArgument, GenericParam,
    ImplItem, ImplItemFn, Item, ItemEnum, ItemFn, ItemStruct, Local, Macro, Pat, PathArguments,
    Stmt, Type,
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
    // A probe directory is created exclusively inside this checkout's target
    // tree, so no other account can pre-place or redirect it (D-344).
    fn create(paths: &Paths, label: &str, source: &str) -> Self {
        let target = crate::mutation_workspace::canonical_existing_real_directory(
            paths.target_dir(),
            "contract probe target",
        )
        .expect("contract probe target");
        std::fs::create_dir_all(target.join("contract-probes")).expect("contract probe parent");
        let parent = crate::mutation_workspace::canonical_existing_real_directory(
            &target.join("contract-probes"),
            "contract probe parent",
        )
        .expect("contract probe parent");
        let root = fresh_probe_directory(&parent, label, || {
            NEXT_PROBE.fetch_add(1, Ordering::Relaxed)
        });
        std::fs::create_dir(root.join("src")).expect("contract source directory");

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
                crate::cargo::child_process_path(&paths.target_dir().join("guard-contracts")),
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

/// Create a directory no earlier entry occupies; an existing name is skipped.
fn fresh_probe_directory(
    parent: &std::path::Path,
    label: &str,
    mut next: impl FnMut() -> usize,
) -> std::path::PathBuf {
    core::iter::repeat_with(|| parent.join(format!("{label}-{}-{}", std::process::id(), next())))
        .take(128)
        .find(|root| match std::fs::create_dir(root) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => panic!("creating contract probe {}: {error}", root.display()),
        })
        .expect("a fresh contract probe directory")
}

#[test]
fn contract_probes_are_fresh_directories_inside_the_target_tree() {
    let paths = Paths::discover().expect("workspace paths");
    let target = crate::mutation_workspace::canonical_existing_real_directory(
        paths.target_dir(),
        "contract probe target",
    )
    .expect("contract probe target");
    let probe = Probe::create(&paths, "fresh-probe", "#![no_std]\n");
    assert!(
        probe.root.starts_with(target.join("contract-probes")),
        "contract probe escaped the target tree: {}",
        probe.root.display()
    );
    drop(probe);

    let parent = target.join("contract-probes");
    let occupied = parent.join(format!("occupied-{}-0", std::process::id()));
    let outside = parent.join(format!("outside-{}", std::process::id()));
    let _ = std::fs::remove_file(&occupied).or_else(|_| std::fs::remove_dir(&occupied));
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir(parent.join(format!("occupied-{}-1", std::process::id())));
    std::fs::create_dir(&outside).expect("outside fixture");
    std::fs::write(outside.join("marker"), b"outside").expect("outside marker");
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&outside, &occupied).expect("occupying link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &occupied).expect("occupying link");
    let mut counter = 0;
    let fresh = fresh_probe_directory(&parent, "occupied", || {
        counter += 1;
        counter - 1
    });
    assert_eq!(
        fresh,
        parent.join(format!("occupied-{}-1", std::process::id()))
    );
    assert_eq!(
        std::fs::read_dir(&outside)
            .expect("outside listing")
            .count(),
        1,
        "a pre-placed link redirected the probe"
    );
    std::fs::remove_dir(&fresh).expect("removing the fresh probe");
    std::fs::remove_file(&occupied)
        .or_else(|_| std::fs::remove_dir(&occupied))
        .expect("removing the occupying link");
    std::fs::remove_dir_all(&outside).expect("removing the outside fixture");
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

/// A type that names `mem`'s `AddressSpace`, however deeply it is nested.
#[derive(Default)]
struct NamesAddressSpace(bool);

impl<'ast> Visit<'ast> for NamesAddressSpace {
    fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
        if path
            .path
            .segments
            .iter()
            .any(|segment| segment.ident == "AddressSpace")
        {
            self.0 = true;
        }
        visit::visit_type_path(self, path);
    }
}

/// Whether an item with this visibility is reachable from outside its module.
fn is_visible_outside_module(visibility: &syn::Visibility) -> bool {
    match visibility {
        syn::Visibility::Public(_) => true,
        syn::Visibility::Restricted(restricted) => !restricted.path.is_ident("self"),
        syn::Visibility::Inherited => false,
    }
}

fn signature_names_the_space(signature: &syn::Signature) -> bool {
    let mut names = NamesAddressSpace::default();
    names.visit_signature(signature);
    names.0
}

fn type_names_the_space(ty: &Type) -> bool {
    let mut names = NamesAddressSpace::default();
    names.visit_type(ty);
    names.0
}

/// Every item `mem` exposes to the rest of the kernel whose type surface names
/// the running `AddressSpace`. Function bodies are excluded on purpose: a body
/// may reach the space through the private helper, but only a crossing
/// signature or an accessible field hands it out (D-189).
#[derive(Default)]
struct AddressSpaceSurface(Vec<String>);

impl<'ast> Visit<'ast> for AddressSpaceSurface {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        if is_visible_outside_module(&item.vis) && signature_names_the_space(&item.sig) {
            self.0.push(item.sig.ident.to_string());
        }
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        if is_visible_outside_module(&item.vis) && signature_names_the_space(&item.sig) {
            self.0.push(item.sig.ident.to_string());
        }
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        if !is_visible_outside_module(&item.vis) {
            return;
        }
        for field in &item.fields {
            if is_visible_outside_module(&field.vis) && type_names_the_space(&field.ty) {
                self.0.push(item.ident.to_string());
            }
        }
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        if !is_visible_outside_module(&item.vis) {
            return;
        }
        for variant in &item.variants {
            for field in &variant.fields {
                if type_names_the_space(&field.ty) {
                    self.0.push(item.ident.to_string());
                }
            }
        }
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        if is_visible_outside_module(&item.vis) && type_names_the_space(&item.ty) {
            self.0.push(item.ident.to_string());
        }
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if is_visible_outside_module(&item.vis) && type_names_the_space(&item.ty) {
            self.0.push(item.ident.to_string());
        }
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if is_visible_outside_module(&item.vis) && type_names_the_space(&item.ty) {
            self.0.push(item.ident.to_string());
        }
    }
}

/// D-189: the running `AddressSpace` carries reclaim and arbitrary-unmap
/// authority, so nothing `mem` exposes to the rest of the kernel may name it.
/// A rename of the private borrow helper fails compilation; a second borrow
/// route under any other name is caught here, where the old name search could
/// not see it (PW-552).
#[test]
fn no_item_mem_exposes_names_the_running_address_space() {
    let paths = Paths::discover().expect("workspace paths");
    let source = paths.root().join("kernel/src/mem.rs");
    let text = std::fs::read_to_string(&source)
        .unwrap_or_else(|error| panic!("reading {}: {error}", source.display()));
    let syntax = syn::parse_file(&text)
        .unwrap_or_else(|error| panic!("parsing {}: {error}", source.display()));

    let mut surface = AddressSpaceSurface::default();
    surface.visit_file(&syntax);

    // Items whose crossing signature names the space by audited exception.
    const ALLOWED: &[&str] = &[];
    let leaked: Vec<&String> = surface
        .0
        .iter()
        .filter(|name| !ALLOWED.contains(&name.as_str()))
        .collect();

    assert!(
        leaked.is_empty(),
        "these items visible outside `mem` name the running AddressSpace, \
         which no safe module may borrow (D-189): {leaked:?}"
    );
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

    let pure_forgery = "#![no_std]\n\
        fn probe(authority: pci_config::FunctionAuthority) {\n\
            let pci_config::FunctionAuthority { lease: _, .. } = authority;\n\
        }\n";
    require_error(&paths, "pci-pure-function-forgery", pure_forgery, "E0451").unwrap_or_else(
        |diagnostics| {
            panic!("pure PCI function authority exposed its private lease:\n{diagnostics}")
        },
    );

    let hardware_forgery = "#![no_std]\n\
        fn probe(authority: wost::arch::current::pci::FunctionClaim) {\n\
            let wost::arch::current::pci::FunctionClaim { identity: _, .. } = authority;\n\
        }\n";
    require_error(
        &paths,
        "pci-hardware-function-forgery",
        hardware_forgery,
        "E0451",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("hardware PCI function authority exposed its private identity:\n{diagnostics}")
    });

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

    // The domain proof is minted only by a programmed unit (D-357).
    let forged_domain = "#![no_std]\n\
        fn forge(function: pci_config::Address) -> wost::arch::current::vtd::DmaDomain {\n\
            wost::arch::current::vtd::DmaDomain { function }\n\
        }\n";
    require_error(&paths, "dma-domain-forgery", forged_domain, "E0451").unwrap_or_else(
        |diagnostics| panic!("the DMA domain proof became forgeable:\n{diagnostics}"),
    );
    let reused_domain = "#![no_std]\n\
        fn spend(_: wost::arch::current::vtd::DmaDomain) {}\n\
        fn probe(domain: wost::arch::current::vtd::DmaDomain) {\n\
            spend(domain);\n\
            spend(domain);\n\
        }\n";
    require_error(&paths, "dma-domain-reuse", reused_domain, "E0382").unwrap_or_else(
        |diagnostics| panic!("one DMA domain proof could be spent twice:\n{diagnostics}"),
    );
    let cloned_domain = "#![no_std]\nfn requires<T: Clone>() {}\n\
        fn probe() { requires::<wost::arch::current::vtd::DmaDomain>(); }\n";
    require_error(&paths, "dma-domain-clone", cloned_domain, "E0277")
        .unwrap_or_else(|diagnostics| panic!("a DMA domain proof could be cloned:\n{diagnostics}"));

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
            "let _ = pci::admit_bus_master(at, domain, state);",
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
            "bus-master-upgrade" => {
                ", domain: wost::arch::current::vtd::DmaDomain, state: &mut pci_config::CommandState"
            }
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
             fn probe(input: {input}, domain: wost::arch::current::vtd::DmaDomain, state: &mut pci_config::CommandState) {{\n\
                 let _ = pci::admit_bus_master(input, domain, state);\n\
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

/// Types whose values are PCI function authority (D-335, D-342).
const PCI_AUTHORITY_ROOTS: [&str; 2] = ["FunctionAuthority", "FunctionClaim"];
/// The only doors that may produce authority without consuming any.
const PCI_AUTHORITY_MINTS: [(&str, &str); 2] = [
    ("libs/pci-config/src/lib.rs", "FunctionClaims::claim"),
    ("wost/src/arch/x86_64/pci.rs", "claim_function"),
];
/// Traits none of whose members can manufacture the implementing type.
const PCI_AUTHORITY_TRAITS: [&str; 5] = ["Debug", "Display", "Error", "PartialEq", "Eq"];
const PCI_AUTHORITY_DERIVES: [&str; 3] = ["Debug", "PartialEq", "Eq"];
const UNBOUNDED: u32 = u32::MAX;

/// How a type holds PCI authority: owned copies, a shared view, or a channel
/// the inventory cannot count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Holding {
    owned: u32,
    shared: bool,
    escape: bool,
}

impl Holding {
    const ESCAPE: Self = Self {
        owned: 0,
        shared: false,
        escape: true,
    };

    const fn any(self) -> bool {
        self.owned > 0 || self.shared || self.escape
    }

    const fn both(self, other: Self) -> Self {
        Self {
            owned: self.owned.saturating_add(other.owned),
            shared: self.shared || other.shared,
            escape: self.escape || other.escape,
        }
    }

    fn either(self, other: Self) -> Self {
        Self {
            owned: self.owned.max(other.owned),
            shared: self.shared || other.shared,
            escape: self.escape || other.escape,
        }
    }

    const fn times(self, count: u32) -> Self {
        Self {
            owned: self.owned.saturating_mul(count),
            ..self
        }
    }
}

#[derive(Clone, Copy)]
struct AuthorityTypes<'a> {
    carriers: &'a BTreeMap<String, Holding>,
    self_type: Option<Holding>,
}

impl AuthorityTypes<'_> {
    fn names_ident(self, ident: &proc_macro2::Ident) -> bool {
        self.carriers.contains_key(&ident.to_string())
            || (ident == "Self" && self.self_type.is_some_and(Holding::any))
    }

    fn names_tokens(self, tokens: proc_macro2::TokenStream) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => self.names_ident(&ident),
            proc_macro2::TokenTree::Group(group) => self.names_tokens(group.stream()),
            _ => false,
        })
    }

    fn uncounted(self, visit: impl FnOnce(&mut AuthorityNames<'_>)) -> Holding {
        let mut names = AuthorityNames {
            types: self,
            found: false,
        };
        visit(&mut names);
        if names.found {
            Holding::ESCAPE
        } else {
            Holding::default()
        }
    }

    fn holding(self, ty: &Type) -> Holding {
        match ty {
            Type::Path(path) if path.qself.is_none() => self.path_holding(&path.path),
            Type::Reference(reference) => {
                let inner = self.holding(&reference.elem);
                if reference.mutability.is_some() && (inner.owned > 0 || inner.escape) {
                    Holding::ESCAPE
                } else {
                    Holding {
                        owned: 0,
                        shared: inner.any(),
                        escape: inner.escape,
                    }
                }
            }
            Type::Tuple(tuple) => tuple
                .elems
                .iter()
                .fold(Holding::default(), |held, element| {
                    held.both(self.holding(element))
                }),
            Type::Array(array) => {
                let length = match &array.len {
                    Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Int(length),
                        ..
                    }) => length.base10_parse::<u32>().unwrap_or(UNBOUNDED),
                    _ => UNBOUNDED,
                };
                self.holding(&array.elem).times(length)
            }
            Type::Slice(slice) => self.holding(&slice.elem).times(UNBOUNDED),
            Type::Paren(inner) => self.holding(&inner.elem),
            Type::Group(inner) => self.holding(&inner.elem),
            other => self.uncounted(|names| names.visit_type(other)),
        }
    }

    fn path_holding(self, path: &syn::Path) -> Holding {
        let Some(last) = path.segments.last() else {
            return Holding::default();
        };
        let mut held = if path.segments.len() == 1 && last.ident == "Self" {
            self.self_type.unwrap_or_default()
        } else {
            self.carriers
                .get(&last.ident.to_string())
                .copied()
                .unwrap_or_default()
        };
        for earlier in path.segments.iter().take(path.segments.len() - 1) {
            held =
                held.both(self.uncounted(|names| names.visit_path_arguments(&earlier.arguments)));
        }
        match &last.arguments {
            PathArguments::None => held,
            PathArguments::AngleBracketed(arguments) => {
                let alternatives = last.ident == "Option" || last.ident == "Result";
                let mut inner = Holding::default();
                for argument in &arguments.args {
                    let argument = match argument {
                        GenericArgument::Type(ty) => self.holding(ty),
                        other => self.uncounted(|names| names.visit_generic_argument(other)),
                    };
                    inner = if alternatives {
                        inner.either(argument)
                    } else {
                        inner.both(argument)
                    };
                }
                held.both(if alternatives {
                    inner
                } else {
                    inner.times(UNBOUNDED)
                })
            }
            PathArguments::Parenthesized(_) => {
                held.both(self.uncounted(|names| names.visit_path_arguments(&last.arguments)))
            }
        }
    }
}

struct AuthorityNames<'a> {
    types: AuthorityTypes<'a>,
    found: bool,
}

impl<'ast> Visit<'ast> for AuthorityNames<'_> {
    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        self.found |= self.types.names_ident(ident);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        self.found |= self.types.names_tokens(mac.tokens.clone());
        visit::visit_macro(self, mac);
    }
}

fn cfg_test(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test")
    })
}

fn authority_fields<'f>(
    types: AuthorityTypes<'_>,
    fields: impl IntoIterator<Item = &'f syn::Field>,
) -> Holding {
    fields.into_iter().fold(Holding::default(), |held, field| {
        held.both(types.holding(&field.ty))
    })
}

fn grow_pci_carriers(
    items: &[Item],
    carriers: &BTreeMap<String, Holding>,
    grown: &mut BTreeMap<String, Holding>,
) {
    let types = AuthorityTypes {
        carriers,
        self_type: None,
    };
    for item in items {
        if cfg_test(observer_item_attributes(item)) {
            continue;
        }
        let (name, held) = match item {
            Item::Struct(item) => (&item.ident, authority_fields(types, &item.fields)),
            Item::Union(item) => (&item.ident, authority_fields(types, &item.fields.named)),
            Item::Enum(item) => (
                &item.ident,
                item.variants
                    .iter()
                    .fold(Holding::default(), |held, variant| {
                        held.either(authority_fields(types, &variant.fields))
                    }),
            ),
            Item::Type(item) => (&item.ident, types.holding(&item.ty)),
            Item::Use(item) => {
                grow_pci_renames(&item.tree, carriers, grown);
                continue;
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    grow_pci_carriers(items, carriers, grown);
                }
                continue;
            }
            _ => continue,
        };
        if held.any() {
            let entry = grown.entry(name.to_string()).or_default();
            *entry = entry.either(held);
        }
    }
}

fn grow_pci_renames(
    tree: &syn::UseTree,
    carriers: &BTreeMap<String, Holding>,
    grown: &mut BTreeMap<String, Holding>,
) {
    match tree {
        syn::UseTree::Path(path) => grow_pci_renames(&path.tree, carriers, grown),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                grow_pci_renames(tree, carriers, grown);
            }
        }
        syn::UseTree::Rename(rename) => {
            if let Some(held) = carriers.get(&rename.ident.to_string()) {
                let entry = grown.entry(rename.rename.to_string()).or_default();
                *entry = entry.either(*held);
            }
        }
        syn::UseTree::Name(_) | syn::UseTree::Glob(_) => {}
    }
}

fn copies(count: u32) -> String {
    if count == UNBOUNDED {
        "unboundedly many".to_string()
    } else {
        count.to_string()
    }
}

struct AuthorityInventory<'a> {
    file: &'a str,
    carriers: &'a BTreeMap<String, Holding>,
    names_authority: bool,
    self_type: Option<Holding>,
    owner: Option<String>,
    violations: Vec<String>,
    mints: BTreeMap<String, usize>,
}

impl<'a> AuthorityInventory<'a> {
    const fn types(&self) -> AuthorityTypes<'a> {
        AuthorityTypes {
            carriers: self.carriers,
            self_type: self.self_type,
        }
    }

    fn refuse(&mut self, reason: &str) {
        self.violations.push(format!("{}: {reason}", self.file));
    }

    fn require_fields<'f>(
        &mut self,
        owner: &syn::Ident,
        fields: impl IntoIterator<Item = &'f syn::Field>,
    ) {
        let types = AuthorityTypes {
            carriers: self.carriers,
            self_type: None,
        };
        if fields
            .into_iter()
            .any(|field| types.holding(&field.ty).escape)
        {
            self.refuse(&format!(
                "a field of `{owner}` reaches PCI authority through a type the inventory cannot count"
            ));
        }
    }

    fn require_derives(&mut self, owner: &syn::Ident, attributes: &[syn::Attribute]) {
        if !self
            .carriers
            .get(&owner.to_string())
            .is_some_and(|held| held.owned > 0)
        {
            return;
        }
        for attribute in attributes
            .iter()
            .filter(|attribute| attribute.path().is_ident("derive"))
        {
            let derived = attribute
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .unwrap_or_else(|error| panic!("parsing the derives of `{owner}`: {error}"));
            for path in derived {
                let name = path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string())
                    .unwrap_or_default();
                if !PCI_AUTHORITY_DERIVES.contains(&name.as_str()) {
                    self.refuse(&format!("PCI authority `{owner}` derives `{name}`"));
                }
            }
        }
    }

    fn require_signature(&mut self, signature: &syn::Signature) {
        let name = match &self.owner {
            Some(owner) => format!("{owner}::{}", signature.ident),
            None => signature.ident.to_string(),
        };
        let types = self.types();
        let mut consumed = 0_u32;
        let mut uncounted = false;
        for input in &signature.inputs {
            let held = match input {
                syn::FnArg::Receiver(receiver) => types.holding(&receiver.ty),
                syn::FnArg::Typed(argument) => types.holding(&argument.ty),
            };
            uncounted |= held.escape;
            consumed = consumed.saturating_add(held.owned);
        }
        let produced = match &signature.output {
            syn::ReturnType::Default => Holding::default(),
            syn::ReturnType::Type(_, output) => types.holding(output),
        };
        if uncounted {
            self.refuse(&format!(
                "`{name}` takes PCI authority through a parameter the inventory cannot count"
            ));
        }
        if produced.escape {
            self.refuse(&format!(
                "`{name}` returns PCI authority through a type the inventory cannot count"
            ));
        }
        if produced.owned > consumed {
            let audited = PCI_AUTHORITY_MINTS
                .iter()
                .any(|(file, mint)| *file == self.file && *mint == name);
            if audited && consumed == 0 && produced.owned == 1 {
                *self.mints.entry(name).or_default() += 1;
            } else {
                self.refuse(&format!(
                    "`{name}` returns {} PCI authority copies while consuming {}",
                    copies(produced.owned),
                    copies(consumed)
                ));
            }
        }
    }

    fn require_opaque_macro(&mut self) {
        if self.names_authority {
            self.refuse("an item macro can expand to a door the inventory cannot see");
        }
    }
}

impl<'ast> Visit<'ast> for AuthorityInventory<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !cfg_test(observer_item_attributes(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        if !cfg_test(observer_impl_item_attributes(item)) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        self.require_derives(&item.ident, &item.attrs);
        self.require_fields(&item.ident, &item.fields);
        visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        self.require_derives(&item.ident, &item.attrs);
        for variant in &item.variants {
            self.require_fields(&item.ident, &variant.fields);
        }
        visit::visit_item_enum(self, item);
    }

    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.require_derives(&item.ident, &item.attrs);
        self.require_fields(&item.ident, &item.fields.named);
        visit::visit_item_union(self, item);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!("static `{}` stores PCI authority", item.ident));
        }
        visit::visit_item_static(self, item);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!("constant `{}` stores PCI authority", item.ident));
        }
        visit::visit_item_const(self, item);
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!("constant `{}` stores PCI authority", item.ident));
        }
        visit::visit_impl_item_const(self, item);
    }

    fn visit_impl_item_type(&mut self, item: &'ast syn::ImplItemType) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!(
                "associated type `{}` names PCI authority",
                item.ident
            ));
        }
        visit::visit_impl_item_type(self, item);
    }

    fn visit_generics(&mut self, generics: &'ast syn::Generics) {
        if self
            .types()
            .uncounted(|names| names.visit_generics(generics))
            .escape
        {
            self.refuse("a generic parameter or bound names PCI authority");
        }
        visit::visit_generics(self, generics);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        if self.types().names_tokens(mac.tokens.clone()) {
            self.refuse("a macro receives PCI authority names the inventory cannot follow");
        }
        visit::visit_macro(self, mac);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        self.require_opaque_macro();
        visit::visit_item_macro(self, item);
    }

    fn visit_impl_item_macro(&mut self, item: &'ast syn::ImplItemMacro) {
        self.require_opaque_macro();
        visit::visit_impl_item_macro(self, item);
    }

    fn visit_trait_item_macro(&mut self, item: &'ast syn::TraitItemMacro) {
        self.require_opaque_macro();
        visit::visit_trait_item_macro(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let held = AuthorityTypes {
            carriers: self.carriers,
            self_type: None,
        }
        .holding(&item.self_ty);
        let owner = match item.self_ty.as_ref() {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
            _ => None,
        };
        if let Some((_, implemented, _)) = &item.trait_
            && held.owned > 0
        {
            let implemented = implemented
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default();
            if !PCI_AUTHORITY_TRAITS.contains(&implemented.as_str()) {
                self.refuse(&format!(
                    "PCI authority `{}` implements `{implemented}`",
                    owner.as_deref().unwrap_or("?")
                ));
            }
        }
        let self_type = self.self_type.replace(held);
        let previous = core::mem::replace(&mut self.owner, owner);
        visit::visit_item_impl(self, item);
        self.self_type = self_type;
        self.owner = previous;
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        let self_type = self.self_type.take();
        let previous = self.owner.replace(item.ident.to_string());
        visit::visit_item_trait(self, item);
        self.self_type = self_type;
        self.owner = previous;
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let self_type = self.self_type.take();
        let previous = self.owner.take();
        self.require_signature(&function.sig);
        visit::visit_item_fn(self, function);
        self.self_type = self_type;
        self.owner = previous;
    }

    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        self.require_signature(&method.sig);
        visit::visit_impl_item_fn(self, method);
    }

    fn visit_trait_item_fn(&mut self, method: &'ast syn::TraitItemFn) {
        self.require_signature(&method.sig);
        visit::visit_trait_item_fn(self, method);
    }
}

fn pci_authority_violations(sources: &[(&str, &str)]) -> Vec<String> {
    let files = sources
        .iter()
        .map(|(relative, text)| {
            syn::parse_file(text).unwrap_or_else(|error| panic!("parsing {relative}: {error}"))
        })
        .collect::<Vec<_>>();
    let mut carriers = PCI_AUTHORITY_ROOTS
        .iter()
        .map(|root| {
            (
                (*root).to_string(),
                Holding {
                    owned: 1,
                    ..Holding::default()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    loop {
        let mut grown = carriers.clone();
        for syntax in &files {
            grow_pci_carriers(&syntax.items, &carriers, &mut grown);
        }
        if grown == carriers {
            break;
        }
        carriers = grown;
    }

    let mut violations = Vec::new();
    let mut mints = BTreeMap::<(String, String), usize>::new();
    for (&(relative, text), syntax) in sources.iter().zip(&files) {
        let tokens = text
            .parse::<proc_macro2::TokenStream>()
            .unwrap_or_else(|error| panic!("tokenizing {relative}: {error}"));
        let mut inventory = AuthorityInventory {
            file: relative,
            carriers: &carriers,
            names_authority: AuthorityTypes {
                carriers: &carriers,
                self_type: None,
            }
            .names_tokens(tokens),
            self_type: None,
            owner: None,
            violations: Vec::new(),
            mints: BTreeMap::new(),
        };
        inventory.visit_file(syntax);
        violations.append(&mut inventory.violations);
        for (mint, seen) in inventory.mints {
            *mints.entry((relative.to_string(), mint)).or_default() += seen;
        }
    }
    for (file, mint) in PCI_AUTHORITY_MINTS {
        let seen = mints
            .get(&(file.to_string(), mint.to_string()))
            .copied()
            .unwrap_or(0);
        if seen != 1 {
            violations.push(format!(
                "{file}: audited mint `{mint}` was seen {seen} times"
            ));
        }
    }
    violations
}

#[test]
fn pci_function_authority_has_audited_mints_and_no_duplication_door() {
    let paths = Paths::discover().expect("workspace paths");
    let mut sources = Vec::new();
    for directory in ["libs/pci-config/src", "wost/src"] {
        for path in rust_sources_below(&paths.root().join(directory)) {
            let relative = path
                .strip_prefix(paths.root())
                .expect("source below the workspace")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
            sources.push((relative, text));
        }
    }
    let sources = sources
        .iter()
        .map(|(relative, text)| (relative.as_str(), text.as_str()))
        .collect::<Vec<_>>();
    let violations = pci_authority_violations(&sources);
    assert!(
        violations.is_empty(),
        "PCI function authority has an uncounted door:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_pci_authority_inventory_counts_every_door_and_channel() {
    const PURE: &str = "libs/pci-config/src/lib.rs";
    const HARDWARE: &str = "wost/src/arch/x86_64/pci.rs";
    let pure = "#[derive(Debug, PartialEq, Eq)]\n\
        pub struct FunctionAuthority { lease: u64 }\n\
        pub struct FunctionClaims;\n\
        impl FunctionClaims {\n\
            pub fn claim(&mut self) -> Result<FunctionAuthority, ()> { Err(()) }\n\
        }\n\
        pub struct FunctionReleaseFailure { authority: FunctionAuthority }\n\
        impl FunctionReleaseFailure {\n\
            pub fn into_authority(self) -> FunctionAuthority { self.authority }\n\
        }\n\
        impl core::fmt::Display for FunctionReleaseFailure {\n\
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { Ok(()) }\n\
        }\n\
        #[cfg(test)]\n\
        mod tests {\n\
            fn authority() -> super::FunctionAuthority { loop {} }\n\
        }\n";
    let hardware = "pub struct FunctionClaim { authority: FunctionAuthority }\n\
        impl FunctionClaim { pub const fn address(&self) -> u32 { 0 } }\n\
        pub struct ReleaseFailure { function: FunctionClaim }\n\
        pub fn claim_function() -> Result<FunctionClaim, ()> { Err(()) }\n\
        pub fn release_function(function: FunctionClaim) -> Result<(), ReleaseFailure> { Ok(()) }\n\
        pub fn probe_bars(function: &FunctionClaim) -> Option<&FunctionClaim> { Some(function) }\n";
    let check = |pure: &str, hardware: &str| {
        pci_authority_violations(&[(PURE, pure), (HARDWARE, hardware)])
    };
    assert_eq!(check(pure, hardware), Vec::<String>::new());

    for (label, pure_extra, hardware_extra, expected) in [
        (
            "renamed",
            "",
            "impl FunctionClaim { pub fn fork(&self) -> Self { loop {} } }\n",
            "`FunctionClaim::fork` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "split",
            "impl FunctionAuthority { pub fn split(self) -> (Self, Self) { loop {} } }\n",
            "",
            "`FunctionAuthority::split` returns 2 PCI authority copies while consuming 1",
        ),
        (
            "array",
            "",
            "pub fn pair(a: FunctionClaim) -> [FunctionClaim; 2] { loop {} }\n",
            "`pair` returns 2 PCI authority copies while consuming 1",
        ),
        (
            "container",
            "",
            "pub fn many(a: FunctionClaim) -> Vec<FunctionClaim> { loop {} }\n",
            "`many` returns unboundedly many PCI authority copies while consuming 1",
        ),
        (
            "alias",
            "pub type Token = FunctionAuthority;\npub fn token() -> Token { loop {} }\n",
            "",
            "`token` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "rename",
            "",
            "use crate::FunctionClaim as Lease;\npub fn lease() -> Lease { loop {} }\n",
            "`lease` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "wrapper",
            "pub struct Spare(FunctionReleaseFailure);\npub fn spare() -> Option<Spare> { None }\n",
            "",
            "`spare` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "mutable parameter",
            "",
            "pub fn overwrite(slot: &mut Option<FunctionClaim>) {}\n",
            "`overwrite` takes PCI authority through a parameter the inventory cannot count",
        ),
        (
            "mutable receiver",
            "impl FunctionAuthority { pub fn retarget(&mut self) {} }\n",
            "",
            "`FunctionAuthority::retarget` takes PCI authority through a parameter",
        ),
        (
            "callback",
            "",
            "pub fn lend(sink: impl FnOnce(FunctionClaim)) {}\n",
            "`lend` takes PCI authority through a parameter the inventory cannot count",
        ),
        (
            "pointer",
            "",
            "pub fn raw() -> *const FunctionClaim { loop {} }\n",
            "`raw` returns PCI authority through a type the inventory cannot count",
        ),
        (
            "static",
            "",
            "static STASH: Option<FunctionClaim> = None;\n",
            "static `STASH` stores PCI authority",
        ),
        (
            "field",
            "",
            "pub struct Handle<'a> { claim: &'a mut FunctionClaim }\n",
            "a field of `Handle` reaches PCI authority through a type the inventory cannot count",
        ),
        (
            "trait default",
            "",
            "pub trait Twin: Sized { fn twin(&self) -> Self { loop {} } }\nimpl Twin for FunctionClaim {}\n",
            "PCI authority `FunctionClaim` implements `Twin`",
        ),
        (
            "derive",
            "#[derive(Debug, Clone)]\npub struct Spare { authority: FunctionAuthority }\n",
            "",
            "PCI authority `Spare` derives `Clone`",
        ),
        (
            "associated type",
            "",
            "pub struct Minter;\nimpl Iterator for Minter {\n    type Item = FunctionClaim;\n    fn next(&mut self) -> Option<Self::Item> { None }\n}\n",
            "associated type `Item` names PCI authority",
        ),
        (
            "bound",
            "",
            "pub fn with<F: FnOnce(FunctionClaim)>(sink: F) {}\n",
            "a generic parameter or bound names PCI authority",
        ),
        (
            "macro argument",
            "",
            "pub fn named() { let _ = core::mem::size_of::<u8>(); door!(FunctionClaim); }\n",
            "a macro receives PCI authority names the inventory cannot follow",
        ),
        (
            "item macro",
            "",
            "macro_rules! door { () => {}; }\n",
            "an item macro can expand to a door the inventory cannot see",
        ),
        (
            "configured",
            "",
            "#[cfg(feature = \"selftest\")]\npub fn probe_claim() -> FunctionClaim { loop {} }\n",
            "`probe_claim` returns 1 PCI authority copies while consuming 0",
        ),
    ] {
        let violations = check(
            &format!("{pure}{pure_extra}"),
            &format!("{hardware}{hardware_extra}"),
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "the {label} door was not refused with `{expected}`: {violations:?}"
        );
    }

    let unaudited = hardware.replace("claim_function()", "claim_unaudited()");
    let violations = check(pure, &unaudited);
    assert!(
        violations.contains(&format!(
            "{HARDWARE}: audited mint `claim_function` was seen 0 times"
        )),
        "a missing audited mint was not reported: {violations:?}"
    );
    assert!(
        violations.iter().any(|violation| violation
            .contains("`claim_unaudited` returns 1 PCI authority copies while consuming 0")),
        "an unaudited mint was not refused: {violations:?}"
    );
}

/// Checks that refuse a link or non-file at a path (D-344).
const PATH_CHECKS: [&str; 5] = [
    "canonical_existing_real_directory",
    "require_contained_regular_file",
    "require_direct_file",
    "require_direct_file_if_present",
    "require_regular_file",
];

#[derive(Default)]
struct CheckedReopens {
    checks: bool,
    opens: usize,
}

impl<'ast> Visit<'ast> for CheckedReopens {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(function) = call.func.as_ref() {
            let segments = function
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if segments
                .last()
                .is_some_and(|name| PATH_CHECKS.contains(&name.as_str()))
            {
                self.checks = true;
            }
            if segments.ends_with(&["File".to_string(), "open".to_string()]) {
                self.opens += 1;
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_item(&mut self, _item: &'ast Item) {}
}

/// Every function body outside test code, including functions nested in bodies.
fn production_bodies(syntax: &syn::File) -> Vec<(String, &syn::Block)> {
    let mut found = Vec::new();
    let mut pending = syntax.items.iter().collect::<Vec<_>>();
    while let Some(item) = pending.pop() {
        if cfg_test(observer_item_attributes(item)) {
            continue;
        }
        let mut bodies = Vec::new();
        match item {
            Item::Fn(function) => bodies.push((function.sig.ident.to_string(), &*function.block)),
            Item::Impl(implementation) => {
                for member in &implementation.items {
                    if let ImplItem::Fn(method) = member
                        && !cfg_test(&method.attrs)
                    {
                        bodies.push((method.sig.ident.to_string(), &method.block));
                    }
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    pending.extend(items);
                }
            }
            _ => {}
        }
        for (name, body) in bodies {
            for statement in &body.stmts {
                if let Stmt::Item(nested) = statement {
                    pending.push(nested);
                }
            }
            found.push((name, body));
        }
    }
    found
}

fn checked_reopens(relative: &str, source: &str) -> Vec<String> {
    let syntax =
        syn::parse_file(source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
    let mut violations = Vec::new();
    for (name, body) in production_bodies(&syntax) {
        let mut shape = CheckedReopens::default();
        shape.visit_block(body);
        if shape.checks && shape.opens > 0 {
            violations.push(format!(
                "{relative}: `{name}` checks a path, then follows it"
            ));
        }
    }
    violations
}

/// Record builders whose source is established again when their record is
/// qualified, so the Git that answered is the qualifying run's (D-344).
const SOURCE_WITHOUT_GIT: [&str; 1] = ["generation_record_from_digests"];
/// Publishers that must re-measure Git after its last answer (D-344).
const GIT_PUBLISHERS: [(&str, &str, &str); 2] = [
    ("xtask/src/evidence.rs", "create", "write_manifest"),
    ("xtask/src/qualification.rs", "commit_record", "rename"),
];

#[derive(Default)]
struct GitUse {
    binds_source: bool,
    names_resolved: bool,
    bare_git: bool,
    calls: Vec<String>,
}

impl<'ast> Visit<'ast> for GitUse {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(function) = call.func.as_ref() {
            let segments = function
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if segments.ends_with(&["git".to_string(), "resolved".to_string()]) {
                self.names_resolved = true;
            }
            if segments.ends_with(&["Command".to_string(), "new".to_string()])
                && call.args.first().is_some_and(|argument| {
                    matches!(argument, Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(program), .. })
                        if program.value() == "git")
                })
            {
                self.bare_git = true;
            }
            if let Some(last) = segments.last() {
                self.calls.push(last.clone());
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if call.method == "insert"
            && call.args.first().is_some_and(|argument| {
                matches!(argument, Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(key), .. })
                    if key.value() == "source.commit")
            })
        {
            self.binds_source = true;
        }
        self.calls.push(call.method.to_string());
        visit::visit_expr_method_call(self, call);
    }

    fn visit_item(&mut self, _item: &'ast Item) {}
}

fn git_bindings(relative: &str, source: &str) -> Vec<String> {
    let syntax =
        syn::parse_file(source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
    let mut violations = Vec::new();
    for (name, body) in production_bodies(&syntax) {
        let mut shape = GitUse::default();
        shape.visit_block(body);
        if shape.binds_source
            && !shape.names_resolved
            && !SOURCE_WITHOUT_GIT.contains(&name.as_str())
        {
            violations.push(format!(
                "{relative}: `{name}` binds a source without the Git that established it"
            ));
        }
        if shape.bare_git && relative != "xtask/src/git.rs" {
            violations.push(format!("{relative}: `{name}` spawns `git` from PATH"));
        }
        for (file, publisher, publication) in GIT_PUBLISHERS {
            if relative == file && name == publisher {
                let measured = shape
                    .calls
                    .iter()
                    .position(|call| call == "require_unchanged");
                let published = shape.calls.iter().position(|call| call == publication);
                if !matches!((measured, published), (Some(measured), Some(published)) if measured < published)
                {
                    violations.push(format!(
                        "{relative}: `{name}` publishes before re-measuring Git"
                    ));
                }
            }
        }
    }
    violations
}

#[test]
fn every_source_identity_names_the_git_that_established_it() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    let mut publishers = 0;
    for path in rust_sources_below(&paths.root().join("xtask/src")) {
        let relative = path
            .strip_prefix(paths.root())
            .expect("source below the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
        publishers += production_bodies(&syntax)
            .iter()
            .filter(|(name, _)| {
                GIT_PUBLISHERS
                    .iter()
                    .any(|(file, publisher, _)| *file == relative && name == publisher)
            })
            .count();
        violations.extend(git_bindings(&relative, &source));
    }
    assert_eq!(publishers, GIT_PUBLISHERS.len(), "a Git publisher moved");
    assert!(
        violations.is_empty(),
        "source identity escapes its Git binding:\n{}",
        violations.join("\n")
    );

    let fixture = "fn bound(record: &mut Record) {\n\
            record.insert(\"source.commit\", commit);\n\
            for field in crate::git::resolved()?.fields() {}\n\
        }\n\
        fn unbound(record: &mut Record) { record.insert(\"source.commit\", commit); }\n\
        fn bare() { let _ = Command::new(\"git\"); }\n";
    assert_eq!(
        git_bindings("xtask/src/fixture.rs", fixture),
        [
            "xtask/src/fixture.rs: `bare` spawns `git` from PATH",
            "xtask/src/fixture.rs: `unbound` binds a source without the Git that established it",
        ]
    );
}

#[test]
fn a_checked_path_is_opened_only_without_following_links() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    for path in rust_sources_below(&paths.root().join("xtask/src")) {
        let relative = path
            .strip_prefix(paths.root())
            .expect("source below the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        violations.extend(checked_reopens(&relative, &source));
    }
    assert!(
        violations.is_empty(),
        "checked paths are reopened by a following open:\n{}",
        violations.join("\n")
    );

    let fixture = "fn checked(parent: &Path, path: &Path) {\n\
            require_direct_file(parent, path, \"x\").unwrap();\n\
            let _ = std::fs::File::open(path);\n\
        }\n\
        fn unchecked(path: &Path) { let _ = File::open(path); }\n\
        #[cfg(test)]\n\
        mod tests { fn checked(p: &Path) { require_regular_file(p, \"x\"); File::open(p); } }\n";
    assert_eq!(
        checked_reopens("fixture.rs", fixture),
        ["fixture.rs: `checked` checks a path, then follows it"]
    );
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

fn is_unqualified_type(ty: &Type, expected: &str) -> bool {
    matches!(
        ty,
        Type::Path(path)
            if path.qself.is_none()
                && path.path.leading_colon.is_none()
                && path.path.segments.len() == 1
                && path.path.is_ident(expected)
    )
}

fn inherent_method<'ast>(
    syntax: &'ast syn::File,
    type_name: &str,
    method_name: &str,
    relative: &str,
) -> &'ast ImplItemFn {
    let mut matches = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(implementation)
                if implementation.trait_.is_none()
                    && is_unqualified_type(&implementation.self_ty, type_name) =>
            {
                Some(implementation.items.iter())
            }
            _ => None,
        })
        .flatten()
        .filter_map(|item| match item {
            ImplItem::Fn(method) if method.sig.ident == method_name => Some(method),
            _ => None,
        });
    let method = core::iter::Iterator::next(&mut matches)
        .unwrap_or_else(|| panic!("{relative} has no {type_name}::{method_name} method"));
    assert!(
        core::iter::Iterator::next(&mut matches).is_none(),
        "{relative} has more than one {type_name}::{method_name} method"
    );
    method
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
struct PublicErrorContracts {
    enums: BTreeMap<String, bool>,
    displays: BTreeSet<String>,
    errors: BTreeSet<String>,
}

fn nominal_type_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

impl<'ast> Visit<'ast> for PublicErrorContracts {
    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        let name = item.ident.to_string();
        if matches!(item.vis, syn::Visibility::Public(_)) && name.ends_with("Error") {
            let extensible = item
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("non_exhaustive"));
            assert!(
                self.enums.insert(name.clone(), extensible).is_none(),
                "duplicate public error enum {name} in one source file"
            );
        }
        visit::visit_item_enum(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let Some((_, trait_path, _)) = &item.trait_ else {
            visit::visit_item_impl(self, item);
            return;
        };
        let Some(name) = nominal_type_name(&item.self_ty) else {
            visit::visit_item_impl(self, item);
            return;
        };
        match trait_path.segments.last().map(|segment| &segment.ident) {
            Some(trait_name) if trait_name == "Display" => {
                self.displays.insert(name);
            }
            Some(trait_name) if trait_name == "Error" => {
                self.errors.insert(name);
            }
            _ => {}
        }
        visit::visit_item_impl(self, item);
    }
}

#[test]
fn public_error_enums_have_stable_standard_contracts() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    for root in ["kernel/src", "libs", "wost/src"] {
        for source in rust_sources_below(&paths.root().join(root)) {
            let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
            let text = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
            let syntax = syn::parse_file(&text)
                .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
            let mut contracts = PublicErrorContracts::default();
            contracts.visit_file(&syntax);
            for (name, extensible) in contracts.enums {
                if !extensible {
                    violations.push(format!(
                        "{}::{name} is missing #[non_exhaustive]",
                        relative.display()
                    ));
                }
                if !contracts.displays.contains(&name) {
                    violations.push(format!("{}::{name} is missing Display", relative.display()));
                }
                if !contracts.errors.contains(&name) {
                    violations.push(format!(
                        "{}::{name} is missing core::error::Error",
                        relative.display()
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "public error contracts changed:\n{}",
        violations.join("\n")
    );
}

#[derive(Default)]
struct PublicFailures(Vec<String>);

impl<'ast> Visit<'ast> for PublicFailures {
    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        let name = item.ident.to_string();
        if matches!(item.vis, syn::Visibility::Public(_))
            && name.ends_with("Failure")
            && !item.attrs.iter().any(|attribute| {
                if !attribute.path().is_ident("must_use") {
                    return false;
                }
                let syn::Meta::NameValue(meta) = &attribute.meta else {
                    return false;
                };
                let Expr::Lit(value) = &meta.value else {
                    return false;
                };
                let syn::Lit::Str(reason) = &value.lit else {
                    return false;
                };
                !reason.value().trim().is_empty()
            })
        {
            self.0.push(name);
        }
        visit::visit_item_struct(self, item);
    }
}

#[test]
fn public_failure_wrappers_are_must_use() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    for root in ["kernel/src", "libs", "wost/src"] {
        for source in rust_sources_below(&paths.root().join(root)) {
            let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
            let text = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
            let syntax = syn::parse_file(&text)
                .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
            let mut failures = PublicFailures::default();
            failures.visit_file(&syntax);
            violations.extend(
                failures
                    .0
                    .into_iter()
                    .map(|name| format!("{}::{name}", relative.display())),
            );
        }
    }
    assert!(
        violations.is_empty(),
        "public failure wrappers missing #[must_use = \"reason\"]:\n{}",
        violations.join("\n")
    );
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

/// The raw port-claim doors, which take a numeric port rather than a device.
const RAW_PORT_CLAIMS: [&str; 4] = ["claim", "claim_pair", "claim_block", "claim_block_pair"];

#[derive(Default)]
struct RawPortClaims {
    calls: Vec<String>,
}

impl<'ast> Visit<'ast> for RawPortClaims {
    fn visit_macro(&mut self, invocation: &'ast Macro) {
        let mut tokens = Vec::new();
        asm_tokens(invocation.tokens.clone(), &mut tokens);
        let through_port = tokens
            .iter()
            .any(|token| token.trim_start_matches("r#") == "port");
        if through_port {
            for token in &tokens {
                let door = token.trim_start_matches("r#");
                if RAW_PORT_CLAIMS.contains(&door) || door == "*" {
                    self.calls.push(format!("macro port::{door}"));
                }
            }
            for rename in tokens.windows(2) {
                if matches!(rename[0].trim_start_matches("r#"), "port" | "self")
                    && rename[1] == "as"
                {
                    self.calls.push("macro port as".to_string());
                }
            }
        }
        visit::visit_macro(self, invocation);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        let segments = path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if segments.iter().any(|segment| segment == "port")
            && segments
                .last()
                .is_some_and(|name| RAW_PORT_CLAIMS.contains(&name.as_str()))
        {
            self.calls.push(segments.join("::"));
        }
        visit::visit_expr_path(self, path);
    }

    // An import can name a door, or the module, by a path with no `port` segment.
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.imports(&mut Vec::new(), &item.tree);
        visit::visit_item_use(self, item);
    }
}

impl RawPortClaims {
    /// Record each import of a raw door, rename of `port` or glob from it.
    fn imports(&mut self, prefix: &mut Vec<String>, tree: &syn::UseTree) {
        let through_port = prefix.iter().any(|segment| segment == "port");
        let path = |last: &str| {
            prefix
                .iter()
                .map(String::as_str)
                .chain([last])
                .collect::<Vec<_>>()
                .join("::")
        };
        match tree {
            syn::UseTree::Path(next) => {
                prefix.push(next.ident.to_string());
                self.imports(prefix, &next.tree);
                prefix.pop();
            }
            syn::UseTree::Group(group) => {
                for item in &group.items {
                    self.imports(prefix, item);
                }
            }
            syn::UseTree::Name(name) => {
                if through_port && RAW_PORT_CLAIMS.contains(&name.ident.to_string().as_str()) {
                    self.calls
                        .push(format!("use {}", path(&name.ident.to_string())));
                }
            }
            syn::UseTree::Rename(rename) => {
                let imported = if rename.ident == "self" {
                    prefix.last().cloned().unwrap_or_default()
                } else {
                    rename.ident.to_string()
                };
                if imported == "port"
                    || (through_port && RAW_PORT_CLAIMS.contains(&imported.as_str()))
                {
                    self.calls.push(format!(
                        "use {} as {}",
                        path(&rename.ident.to_string()),
                        rename.rename
                    ));
                }
            }
            syn::UseTree::Glob(_) => {
                if through_port {
                    self.calls.push(format!("use {}", path("*")));
                }
            }
        }
    }
}

/// Production kernel code names a device, never a raw port number (D-351).
///
/// A driver asks `port::i8042()` or `port::com1()`, not `port::claim(0x60)`.
/// Only the self-test harness may probe the registry with a raw claim, so a
/// component cannot reserve hardware it does not own.
#[test]
fn production_kernel_code_claims_no_port_by_number() {
    let paths = Paths::discover().expect("workspace paths");
    let mut offenders = Vec::new();
    for path in rust_sources_below(&paths.root().join("kernel/src")) {
        let relative = path
            .strip_prefix(paths.root())
            .expect("source below the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        // The self-test harness alone probes exclusivity with raw claims.
        if relative == "kernel/src/selftest.rs" {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
        let mut raw = RawPortClaims::default();
        raw.visit_file(&syntax);
        for call in raw.calls {
            offenders.push(format!("{relative}: `{call}`"));
        }
    }
    assert!(
        offenders.is_empty(),
        "production kernel code claims a port by number, or imports a door out of this scan's sight, instead of naming a device:\n{}",
        offenders.join("\n")
    );

    // The scanner sees a raw claim wherever `port` and the door both appear.
    let fixture = "fn drive() {\n\
            let _ = wost::arch::current::port::claim::<u32>(0xF4);\n\
            let _ = port::claim_block(0x40);\n\
            let _ = port::com1();\n\
            let _ = port::claim_spans::<2>(&spans);\n\
        }\n";
    let syntax = syn::parse_file(fixture).expect("fixture parses");
    let mut raw = RawPortClaims::default();
    raw.visit_file(&syntax);
    assert_eq!(
        raw.calls,
        [
            "wost::arch::current::port::claim".to_string(),
            "port::claim_block".to_string(),
        ]
    );

    // An import that drops the `port` segment is refused where it is written.
    let imports = "use wost::arch::current::port::{self, Port, claim_spans};\n\
        use wost::arch::current::port::claim;\n\
        use wost::arch::current::port::{claim_block as block, com1};\n\
        use wost::arch::current::port as io;\n\
        use wost::arch::current::port::{self as raw};\n\
        fn drive() { use wost::arch::current::port::*; }\n";
    let syntax = syn::parse_file(imports).expect("import fixture parses");
    let mut raw = RawPortClaims::default();
    raw.visit_file(&syntax);
    assert_eq!(
        raw.calls,
        [
            "use wost::arch::current::port::claim",
            "use wost::arch::current::port::claim_block as block",
            "use wost::arch::current::port as io",
            "use wost::arch::current::port::self as raw",
            "use wost::arch::current::port::*",
        ]
    );
}

#[test]
fn raw_port_claims_inside_macros_are_refused() {
    for (source, door) in [
        (
            "fn drive() { info!(\"{:?}\", port::claim::<u8>(0x80)); }",
            "claim",
        ),
        (
            "fn drive() { assert!(port::claim_block(0x80).is_ok()); }",
            "claim_block",
        ),
        (
            "fn drive() { info!(\"{:?}\", nested!({ port::claim_pair::<u8>(0x80, 0x81) })); }",
            "claim_pair",
        ),
        (
            "macro_rules! drive { () => { wost::arch::current::port::claim_block_pair(0x80, 0x90) }; }",
            "claim_block_pair",
        ),
        (
            "fn drive() { check!(r#port::r#claim::<u8>(0x80)); }",
            "claim",
        ),
        (
            "macro_rules! drive { () => { use wost::arch::current::port::*; }; }",
            "*",
        ),
        (
            "macro_rules! drive { () => { use wost::arch::current::port::{com1, claim_block as block}; }; }",
            "claim_block",
        ),
    ] {
        let syntax = syn::parse_file(source).expect("macro fixture parses");
        let mut raw = RawPortClaims::default();
        raw.visit_file(&syntax);
        assert_eq!(raw.calls, [format!("macro port::{door}")], "{source}");
    }
    for source in [
        "macro_rules! door { () => { use wost::arch::current::port as io; }; }",
        "macro_rules! door { () => { use wost::arch::current::port::{self as io}; }; }",
    ] {
        let syntax = syn::parse_file(source).expect("macro alias fixture parses");
        let mut raw = RawPortClaims::default();
        raw.visit_file(&syntax);
        assert!(!raw.calls.is_empty(), "a macro hid a port alias: {source}");
    }
    let source = "fn drive() { info!(\"port::claim(0x80)\");\n\
        assert!(port::com1().is_ok()); info!(\"{:?}\", port::claim_spans::<2>(&spans)); }";
    let syntax = syn::parse_file(source).expect("literal and typed-door fixture parses");
    let mut raw = RawPortClaims::default();
    raw.visit_file(&syntax);
    assert!(raw.calls.is_empty(), "literal or allowed door was refused");
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
use wost::arch::current::paging::PaintedStack;
use wost::arch::current::paging::cleared_on_drop;
use wost::arch::current::paging::paint_current_permanent_stack_for_test;
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
        ("E0432", "paging::PaintedStack"),
        ("E0432", "paging::cleared_on_drop"),
        ("E0432", "paging::paint_current_permanent_stack_for_test"),
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
    assert_eq!(MISSING.len(), 35, "D-290's shipped wost inventory");
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
                ("CandidateBaseline", 4),
                ("CandidateStackMeasurement", 1),
                ("finish_candidate_stack", 2),
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
                ("STACK_METER_STATE", 8),
                ("STACK_LOWEST", 4),
                ("STACK_MAX_USED", 3),
                ("STACK_MAX_LEAF", 3),
                ("STACK_CANDIDATES", 3),
                ("STACK_MEASUREMENTS", 3),
                ("STACK_MEASUREMENT_VALID", 3),
                ("observe_candidate_stack", 3),
                ("CandidateStackMeasurement", 4),
                ("finish_candidate_stack", 1),
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
        107,
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

fn pattern_is_ident(pattern: &Pat, expected: &str) -> bool {
    matches!(
        pattern,
        Pat::Ident(name)
            if name.by_ref.is_none()
                && name.mutability.is_none()
                && name.subpat.is_none()
                && name.ident == expected
    )
}

fn pattern_is_some_ident(pattern: &Pat, expected: &str) -> bool {
    matches!(
        pattern,
        Pat::TupleStruct(some)
            if some.qself.is_none()
                && some.path.is_ident("Some")
                && some.elems.len() == 1
                && pattern_is_ident(&some.elems[0], expected)
    )
}

fn expression_is_self_field(expression: &Expr, expected: &str) -> bool {
    match expression {
        Expr::Group(group) => expression_is_self_field(&group.expr, expected),
        Expr::Paren(parenthesized) => expression_is_self_field(&parenthesized.expr, expected),
        Expr::Field(field) => {
            expression_is_ident(&field.base, "self")
                && matches!(&field.member, syn::Member::Named(name) if name == expected)
        }
        _ => false,
    }
}

fn expression_is_self_field_borrow(expression: &Expr, expected: &str, mutable: bool) -> bool {
    matches!(
        expression,
        Expr::Reference(reference)
            if reference.mutability.is_some() == mutable
                && expression_is_self_field(&reference.expr, expected)
    )
}

fn method_arguments_are_idents(call: &ExprMethodCall, expected: &[&str]) -> bool {
    call.args.len() == expected.len()
        && call
            .args
            .iter()
            .zip(expected)
            .all(|(argument, expected)| expression_is_ident(argument, expected))
}

fn framebuffer_plan_binding(local: &Local) -> bool {
    let Some(initializer) = &local.init else {
        return false;
    };
    let Expr::MethodCall(call) = initializer.expr.as_ref() else {
        return false;
    };
    pattern_is_some_ident(&local.pat, "plan")
        && initializer.diverge.is_some()
        && call.method == "copy_rows_up"
        && call.turbofish.is_none()
        && expression_is_self_field(&call.receiver, "layout")
        && method_arguments_are_idents(call, &["from_y", "to_y", "rows"])
}

fn framebuffer_row_count(local: &Local) -> bool {
    let Some(initializer) = &local.init else {
        return false;
    };
    matches!(
        initializer.expr.as_ref(),
        Expr::MethodCall(call)
            if pattern_is_ident(&local.pat, "rows")
                && initializer.diverge.is_none()
                && call.method == "len"
                && call.turbofish.is_none()
                && call.args.is_empty()
                && expression_is_ident(&call.receiver, "plan")
    )
}

fn framebuffer_shadow_condition(expression: &Expr) -> bool {
    let Expr::Binary(nonempty_and_nonzero) = expression else {
        return false;
    };
    let Expr::Unary(nonempty) = nonempty_and_nonzero.left.as_ref() else {
        return false;
    };
    let Expr::MethodCall(is_empty) = nonempty.expr.as_ref() else {
        return false;
    };
    let Expr::Binary(nonzero) = nonempty_and_nonzero.right.as_ref() else {
        return false;
    };
    matches!(nonempty_and_nonzero.op, syn::BinOp::And(_))
        && matches!(nonempty.op, syn::UnOp::Not(_))
        && is_empty.method == "is_empty"
        && is_empty.turbofish.is_none()
        && is_empty.args.is_empty()
        && expression_is_self_field(&is_empty.receiver, "back")
        && matches!(nonzero.op, syn::BinOp::Ne(_))
        && expression_is_ident(&nonzero.left, "rows")
        && matches!(
            nonzero.right.as_ref(),
            Expr::Lit(literal)
                if matches!(&literal.lit, syn::Lit::Int(value)
                    if value.base10_parse::<u128>().is_ok_and(|value| value == 0))
        )
}

fn framebuffer_shadow_row_loop(statement: &Stmt) -> Option<&ExprForLoop> {
    let Stmt::Expr(Expr::If(branch), None) = statement else {
        return None;
    };
    let [
        Stmt::Expr(Expr::ForLoop(row_loop), None),
        Stmt::Expr(Expr::Return(exit), Some(_)),
    ] = branch.then_branch.stmts.as_slice()
    else {
        return None;
    };
    (branch.else_branch.is_none()
        && framebuffer_shadow_condition(&branch.cond)
        && exit.expr.is_none()
        && row_loop.label.is_none()
        && pattern_is_ident(&row_loop.pat, "span")
        && expression_is_ident(&row_loop.expr, "plan"))
    .then_some(row_loop)
}

fn framebuffer_destination(local: &Local) -> bool {
    let Some(initializer) = &local.init else {
        return false;
    };
    matches!(
        initializer.expr.as_ref(),
        Expr::MethodCall(call)
            if pattern_is_ident(&local.pat, "destination")
                && initializer.diverge.is_none()
                && call.method == "destination_offset"
                && call.turbofish.is_none()
                && call.args.is_empty()
                && expression_is_ident(&call.receiver, "span")
    )
}

fn framebuffer_source(local: &Local) -> bool {
    let Some(initializer) = &local.init else {
        return false;
    };
    matches!(
        initializer.expr.as_ref(),
        Expr::MethodCall(call)
            if pattern_is_some_ident(&local.pat, "values")
                && initializer.diverge.is_some()
                && call.method == "source"
                && call.turbofish.is_none()
                && call.args.len() == 1
                && expression_is_ident(&call.receiver, "span")
                && expression_is_self_field_borrow(&call.args[0], "back", false)
    )
}

fn framebuffer_device_publication(expression: &Expr) -> bool {
    let Expr::Call(access) = expression else {
        return false;
    };
    let Expr::Path(function) = access.func.as_ref() else {
        return false;
    };
    let Some(Expr::MethodCall(write)) = access.args.first() else {
        return false;
    };
    function.qself.is_none()
        && function.path.is_ident("framebuffer_access")
        && access.args.len() == 1
        && write.method == "write_u32_slice"
        && write.turbofish.is_none()
        && expression_is_self_field(&write.receiver, "region")
        && method_arguments_are_idents(write, &["destination", "values"])
}

fn framebuffer_shadow_commit(expression: &Expr) -> bool {
    let Expr::If(branch) = expression else {
        return false;
    };
    let Expr::MethodCall(is_none) = branch.cond.as_ref() else {
        return false;
    };
    let Expr::MethodCall(copy) = is_none.receiver.as_ref() else {
        return false;
    };
    branch.else_branch.is_none()
        && is_none.method == "is_none"
        && is_none.turbofish.is_none()
        && is_none.args.is_empty()
        && copy.method == "copy_within"
        && copy.turbofish.is_none()
        && copy.args.len() == 1
        && expression_is_ident(&copy.receiver, "span")
        && expression_is_self_field_borrow(&copy.args[0], "back", true)
}

fn values_iter_copied(expression: &Expr) -> bool {
    let Expr::MethodCall(copied) = expression else {
        return false;
    };
    let Expr::MethodCall(iter) = copied.receiver.as_ref() else {
        return false;
    };
    copied.method == "copied"
        && copied.turbofish.is_none()
        && copied.args.is_empty()
        && iter.method == "iter"
        && iter.turbofish.is_none()
        && iter.args.is_empty()
        && expression_is_ident(&iter.receiver, "values")
}

fn mmio_slice_delegation(expression: &Expr) -> bool {
    let Expr::MethodCall(call) = expression else {
        return false;
    };
    call.method == "write_values"
        && call.turbofish.is_none()
        && expression_is_ident(&call.receiver, "self")
        && call.args.len() == 2
        && expression_is_ident(&call.args[0], "offset")
        && values_iter_copied(&call.args[1])
}

fn values_len(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::MethodCall(call)
            if call.method == "len"
                && call.turbofish.is_none()
                && call.args.is_empty()
                && expression_is_ident(&call.receiver, "values")
    )
}

fn mmio_whole_span_check(local: &Local) -> bool {
    let Some(initializer) = &local.init else {
        return false;
    };
    let Expr::Try(checked) = initializer.expr.as_ref() else {
        return false;
    };
    let Expr::MethodCall(call) = checked.expr.as_ref() else {
        return false;
    };
    pattern_is_ident(&local.pat, "range")
        && initializer.diverge.is_none()
        && call.method == "check_words"
        && call.turbofish.is_none()
        && expression_is_ident(&call.receiver, "self")
        && call.args.len() == 2
        && expression_is_ident(&call.args[0], "offset")
        && values_len(&call.args[1])
}

fn mmio_checked_values_loop(expression: &ExprForLoop) -> bool {
    let Pat::Tuple(pattern) = expression.pat.as_ref() else {
        return false;
    };
    let Expr::MethodCall(zip) = expression.expr.as_ref() else {
        return false;
    };
    let Expr::MethodCall(offsets) = zip.receiver.as_ref() else {
        return false;
    };
    pattern.elems.len() == 2
        && pattern_is_ident(&pattern.elems[0], "cursor")
        && pattern_is_ident(&pattern.elems[1], "value")
        && zip.method == "zip"
        && zip.turbofish.is_none()
        && zip.args.len() == 1
        && expression_is_ident(&zip.args[0], "values")
        && offsets.method == "offsets"
        && offsets.turbofish.is_none()
        && offsets.args.is_empty()
        && expression_is_ident(&offsets.receiver, "range")
}

fn u32_turbofish(call: &ExprMethodCall) -> bool {
    let Some(arguments) = &call.turbofish else {
        return false;
    };
    arguments.args.len() == 1
        && matches!(
            arguments.args.first(),
            Some(GenericArgument::Type(Type::Path(path)))
                if path.qself.is_none() && path.path.is_ident("u32")
        )
}

fn mmio_word_pointer(local: &Local) -> bool {
    let Some(initializer) = &local.init else {
        return false;
    };
    let Expr::MethodCall(cast) = initializer.expr.as_ref() else {
        return false;
    };
    let Expr::MethodCall(add) = cast.receiver.as_ref() else {
        return false;
    };
    pattern_is_ident(&local.pat, "word")
        && initializer.diverge.is_none()
        && cast.method == "cast"
        && cast.args.is_empty()
        && u32_turbofish(cast)
        && add.method == "wrapping_add"
        && add.turbofish.is_none()
        && expression_is_self_field(&add.receiver, "base")
        && method_arguments_are_idents(add, &["cursor"])
}

fn mmio_volatile_write(expression: &Expr) -> bool {
    let Expr::Unsafe(block) = expression else {
        return false;
    };
    let [Stmt::Expr(Expr::MethodCall(write), None)] = block.block.stmts.as_slice() else {
        return false;
    };
    write.method == "write_volatile"
        && write.turbofish.is_none()
        && expression_is_ident(&write.receiver, "word")
        && method_arguments_are_idents(write, &["value"])
}

fn expression_is_ok_unit(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::Call(call)
            if call.args.len() == 1
                && matches!(
                    call.func.as_ref(),
                    Expr::Path(function)
                        if function.qself.is_none() && function.path.is_ident("Ok")
                )
                && matches!(&call.args[0], Expr::Tuple(unit) if unit.elems.is_empty())
    )
}

#[test]
fn framebuffer_rows_publish_one_checked_volatile_span_before_shadow_commit() {
    let paths = Paths::discover().expect("workspace paths");

    let boot_relative = "wost/src/boot/mod.rs";
    let boot_source = std::fs::read_to_string(paths.root().join(boot_relative))
        .unwrap_or_else(|error| panic!("reading {boot_relative}: {error}"));
    let boot = syn::parse_file(&boot_source)
        .unwrap_or_else(|error| panic!("parsing {boot_relative}: {error}"));
    let copy_rows = inherent_method(&boot, "Framebuffer", "copy_rows", boot_relative);
    let [Stmt::Local(plan), Stmt::Local(rows), branch, ..] = copy_rows.block.stmts.as_slice()
    else {
        panic!("Framebuffer::copy_rows does not begin with its checked shadow-row path");
    };
    assert!(
        framebuffer_plan_binding(plan) && framebuffer_row_count(rows),
        "Framebuffer::copy_rows does not carry one checked plan into its shadow-row path"
    );
    let row_loop = framebuffer_shadow_row_loop(branch)
        .expect("Framebuffer::copy_rows does not consume that plan in its exact shadow branch");
    let [
        Stmt::Local(destination),
        Stmt::Local(source),
        Stmt::Expr(device, Some(_)),
        Stmt::Expr(shadow, None),
    ] = row_loop.body.stmts.as_slice()
    else {
        panic!("Framebuffer::copy_rows does not have one exact row-publication body");
    };
    assert!(
        framebuffer_destination(destination)
            && framebuffer_source(source)
            && framebuffer_device_publication(device)
            && framebuffer_shadow_commit(shadow),
        "framebuffer rows are not published from one checked span before shadow commit"
    );

    let mmio_relative = "wost/src/mmio.rs";
    let mmio_source = std::fs::read_to_string(paths.root().join(mmio_relative))
        .unwrap_or_else(|error| panic!("reading {mmio_relative}: {error}"));
    let mmio = syn::parse_file(&mmio_source)
        .unwrap_or_else(|error| panic!("parsing {mmio_relative}: {error}"));
    let write_slice = inherent_method(&mmio, "MmioRegion", "write_u32_slice", mmio_relative);
    assert!(
        matches!(
            write_slice.block.stmts.as_slice(),
            [Stmt::Expr(expression, None)] if mmio_slice_delegation(expression)
        ),
        "MmioRegion::write_u32_slice does not delegate the complete slice"
    );

    let write_values = inherent_method(&mmio, "MmioRegion", "write_values", mmio_relative);
    let [
        Stmt::Local(check),
        Stmt::Expr(Expr::ForLoop(values), None),
        Stmt::Expr(done, None),
    ] = write_values.block.stmts.as_slice()
    else {
        panic!("MmioRegion::write_values does not have one exact checked publication body");
    };
    assert!(
        mmio_whole_span_check(check)
            && mmio_checked_values_loop(values)
            && expression_is_ok_unit(done),
        "MmioRegion::write_values does not check its complete span before publication"
    );
    let [Stmt::Local(pointer), Stmt::Expr(volatile, Some(_))] = values.body.stmts.as_slice() else {
        panic!("MmioRegion::write_values does not have one exact per-word publication body");
    };
    assert!(
        mmio_word_pointer(pointer) && mmio_volatile_write(volatile),
        "MmioRegion::write_values does not publish each checked word with a volatile write"
    );
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
