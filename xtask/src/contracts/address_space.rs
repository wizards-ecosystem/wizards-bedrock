use syn::visit::{self, Visit};
use syn::{
    Expr, ExprCall, Fields, GenericArgument, GenericParam, ImplItemFn, Item, ItemEnum, ItemFn,
    ItemStruct, PathArguments, Type,
};

use crate::paths::Paths;

use super::{
    discarded_address_space_reclaims, expression_is_ident, is_named_path, path_ends_with,
    require_error, require_success, rust_sources_below,
};

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
        ("libs/acpi-tables/src", "pub fn checksum_is_valid"),
        ("libs/acpi-tables/src", "pub fn fix_checksum"),
        ("libs/fat32/src", "pub const fn decode_entry"),
        ("libs/fat32/src", "pub const fn decode_slot"),
        ("libs/fat32/src", "pub const fn sector_of"),
        ("libs/fat32/src", "pub const fn table_position"),
        ("libs/memory-map/src", "pub fn allocatable_bytes"),
        ("libs/path-walk/src", "pub fn is_a_name"),
        ("libs/pci-config/src", "pub const fn io_window_size"),
        ("libs/stack-layout/src", "fn stack_overlaps"),
        ("wost/src/arch/x86_64/pci.rs", "fn write_command"),
        ("wost/src/mm.rs", "fn first_region"),
        ("wost/src/mm.rs", "fn past_every_region"),
    ];

    for (relative, surface) in forbidden {
        let root = paths.root().join(relative);
        let sources = if root.is_dir() {
            rust_sources_below(&root)
        } else {
            super::module_sources(&root)
        };
        for path in sources {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
            assert!(
                !source.contains(surface),
                "{} regrew speculative/callback surface {surface:?}",
                path.display()
            );
        }
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
    assert!(paging.contains("*pinned = Some((self.roots, self.layout));"));
    assert!(!paging.contains("*pinned = None;"));
    assert!(paging.contains(
        "if let Some(pinned) = protected_stack_overlap(self.roots(), virtual_base, len)"
    ));

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
