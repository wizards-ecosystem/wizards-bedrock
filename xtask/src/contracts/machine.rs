use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, Item, ItemFn, Macro, Pat, Type};

use crate::paths::Paths;

use super::{
    Probe, asm_tokens, cfg_not_selftest, diagnostic_codes, observer_expr_attributes,
    observer_impl_item_attributes, observer_item_attributes, require_error, selftest_cfg,
};

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

#[test]
fn machine_probes_are_absent_from_the_shipped_wost_graph() {
    let paths = Paths::discover().expect("workspace paths");
    let probes = [
        (
            "dma-owner-drop-driver",
            "#![no_std]\nfn probe() { let _ = wost::mm::dma_mapping_owner_drop_retains_for_test; }\n",
            "E0425",
        ),
        (
            "published-table-drop-driver",
            "#![no_std]\nfn probe() { let _ = wost::mm::published_iommu_table_drop_retains_for_test; }\n",
            "E0425",
        ),
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
    let syntax = super::module_syntax(&root.join(relative));
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
    expected_increments: usize,
    expected_instructions: &[&str],
    operand: &[&str],
) {
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
    let increments = strings
        .iter()
        .enumerate()
        .filter(|(_, value)| value.as_str() == "inc qword ptr [rip + BUFFER_CLEARS]")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    assert_eq!(
        increments.len(),
        expected_increments,
        "unexpected buffer-clear symbol reference count in the {macro_name} return path"
    );
    assert_eq!(
        expected_instructions.len(),
        expected_increments,
        "every buffer-clear tally needs one expected instruction"
    );
    for (increment, expected_instruction) in increments.into_iter().zip(expected_instructions) {
        assert_eq!(
            strings.get(increment.checked_sub(3).expect("tally without instruction")),
            Some(&(*expected_instruction).to_string()),
            "buffer-clear tally is not bound to its expected instruction"
        );
        assert_eq!(
            strings.get(increment.checked_sub(2).expect("unguarded tally")),
            Some(&".if {record_clears}".to_string()),
            "buffer-clear tally is not inside its assembler condition"
        );
        let jump = strings
            .get(increment - 1)
            .and_then(|value| value.strip_prefix("jnz "))
            .and_then(|value| value.strip_suffix('f'))
            .expect("buffer-clear tally has no conditional success jump");
        assert_eq!(
            strings.get(increment + 1).map(String::as_str),
            Some(format!("{jump}:").as_str()),
            "buffer-clear tally's success label does not match its jump"
        );
        assert_eq!(
            strings.get(increment + 2).map(String::as_str),
            Some(".endif"),
            "buffer-clear tally is not wholly inside its assembler condition"
        );
    }
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
                ("dropped", 10),
                ("POLLER_ROUNDS", 4),
                ("FOREVER", 5),
                ("POLLER_PASSES", 3),
                ("POLLER_RUNNING", 6),
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
        ("kernel/src/pci.rs", &[("OBSERVED_BUS_MASTER_REFUSAL", 3)]),
        (
            "kernel/src/arch/x86_64/i8042.rs",
            &[("CONTROL_COMMAND_REFUSED", 5)],
        ),
        (
            "kernel/src/nvme.rs",
            &[
                ("BME_PRECEDING_CC", 4),
                ("BME_PRECEDING_CSTS", 4),
                ("REFUSED_BME_PRECEDING_CC", 4),
                ("REFUSED_BME_PRECEDING_CSTS", 4),
                ("START_ATTEMPTS", 4),
                ("SUBMISSION_DOORBELLS", 4),
                ("COMMAND_HELD", 5),
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
                ("observe_candidate_stack", 4),
                ("CandidateStackMeasurement", 5),
                ("finish_candidate_stack", 2),
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
    require_guarded_buffer_clear(
        &interrupt,
        "global_asm",
        None,
        2,
        &["verw word ptr [rbx - 8]", "verw word ptr [rsp]"],
        &["record_clears", "=", "const", "RECORD_BUFFER_CLEARS", ","],
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
