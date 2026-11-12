use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{
    Expr, ExprCall, ExprForLoop, ExprMatch, ExprMethodCall, GenericArgument, ImplItem, ImplItemFn,
    Item, ItemEnum, ItemFn, Local, Pat, Stmt, Type,
};

use crate::paths::Paths;

use super::{
    expression_is_ident, first_function, path_ends_with, require_error, require_success,
    rust_sources_below,
};

fn top_level_function<'ast>(syntax: &'ast syn::File, name: &str, relative: &str) -> &'ast ItemFn {
    let mut matches = syntax.items.iter().filter_map(|item| match item {
        Item::Fn(function) if function.sig.ident == name => Some(function),
        _ => None,
    });
    let function = matches
        .next()
        .unwrap_or_else(|| panic!("{relative} has no {name} function"));
    assert!(
        matches.next().is_none(),
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
    let method = matches
        .next()
        .unwrap_or_else(|| panic!("{relative} has no {type_name}::{method_name} method"));
    assert!(
        matches.next().is_none(),
        "{relative} has more than one {type_name}::{method_name} method"
    );
    method
}

#[derive(Default)]
struct OpenAtBoundary {
    storage_resolves: usize,
    file_work_submissions: usize,
}

impl<'ast> Visit<'ast> for OpenAtBoundary {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if matches!(call.func.as_ref(), Expr::Path(function)
            if function.qself.is_none()
                && path_ends_with(&function.path, &["crate", "vfs", "resolve"]))
        {
            self.storage_resolves += 1;
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if call.method == "submit"
            && matches!(call.receiver.as_ref(), Expr::Path(receiver)
                if receiver.qself.is_none() && receiver.path.is_ident("FILE_WORK"))
        {
            self.file_work_submissions += 1;
        }
        visit::visit_expr_method_call(self, call);
    }
}

#[test]
fn open_at_storage_work_is_deferred_out_of_trap_context() {
    const RELATIVE: &str = "kernel/src/queue.rs";
    let paths = Paths::discover().expect("workspace paths");
    let syntax = super::module_syntax(&paths.root().join(RELATIVE));

    let mut admission = OpenAtBoundary::default();
    Visit::visit_impl_item_fn(
        &mut admission,
        inherent_method(&syntax, "Queue", "open_at", RELATIVE),
    );
    assert_eq!(
        admission.storage_resolves, 0,
        "OpenAt admission reached storage while RingEnter was in trap context"
    );
    assert_eq!(
        admission.file_work_submissions, 1,
        "OpenAt admission did not transfer exactly one request to the bounded file worker"
    );

    let mut service = OpenAtBoundary::default();
    Visit::visit_item_fn(
        &mut service,
        top_level_function(&syntax, "service", RELATIVE),
    );
    assert_eq!(
        service.storage_resolves, 1,
        "the thread-context file worker does not own the one OpenAt storage resolution"
    );
}

fn is_pub_super(visibility: &syn::Visibility) -> bool {
    matches!(
        visibility,
        syn::Visibility::Restricted(restricted) if restricted.path.is_ident("super")
    )
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
        && call.method == "range_of_words"
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

    let boot_relative = "wost/src/boot.rs";
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
            segments.next(),
            segments.next(),
            segments.next(),
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
    let (Some(first), Some(second), None) = (segments.next(), segments.next(), segments.next())
    else {
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
    let syntax = super::module_syntax(&paths.root().join("kernel/src/user.rs"));

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
    let function = first_function(&file, "force_unlock")?;
    let mut statics = BTreeSet::new();
    for statement in &function.block.stmts {
        let syn::Stmt::Expr(expression, _) = statement else {
            continue;
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
    let reclaim = first_function(&file, "reclaim").expect("wost panic::reclaim");
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

#[test]
fn vtd_admission_proofs_borrow_the_exact_live_unit() {
    let paths = Paths::discover().expect("workspace paths");
    for (label, proof, mint) in [
        ("dma", "DmaDomain", "domain"),
        ("interrupt", "InterruptIsolation", "interrupt_isolation"),
    ] {
        let header = format!("#![no_std]\nuse wost::arch::current::vtd::{{Vtd, {proof}}};\n");
        if proof == "InterruptIsolation" {
            let forged = format!(
                "{header}fn forge(function: pci_config::Address) -> {proof}<'static> {{ {proof} {{ function, _unit: core::marker::PhantomData }} }}"
            );
            require_error(
                &paths,
                &format!("vtd-{label}-forged-owner"),
                &forged,
                "E0451",
            )
            .expect("a unit-free interrupt admission proof could be constructed");
        }
        let dropped = format!(
            "{header}fn probe(unit: Vtd) {{ let proof = unit.{mint}().unwrap(); drop(unit); let _ = proof.function(); }}"
        );
        require_error(
            &paths,
            &format!("vtd-{label}-dropped-owner"),
            &dropped,
            "E0505",
        )
        .expect("admission proof did not borrow its exact owner through its last use");
        let mutated = format!(
            "{header}fn probe(unit: &mut Vtd) {{ let proof = unit.{mint}().unwrap(); let _ = unit.park(); let _ = proof.function(); }}"
        );
        require_error(
            &paths,
            &format!("vtd-{label}-mutated-owner"),
            &mutated,
            "E0502",
        )
        .expect("admission proof allowed mutable access to its live owner");
        let escaped =
            format!("{header}fn probe<'a>(unit: Vtd) -> {proof}<'a> {{ unit.{mint}().unwrap() }}");
        require_error(
            &paths,
            &format!("vtd-{label}-escaped-owner"),
            &escaped,
            "E0515",
        )
        .expect("admission proof escaped the lifetime of its owner");
        let live = format!(
            "{header}fn probe(unit: &mut Vtd) {{ let proof = unit.{mint}().unwrap(); let _ = proof.function(); drop(proof); let _ = unit.park(); }}"
        );
        require_success(&paths, &format!("vtd-{label}-live-owner"), &live)
            .expect("spending a proof did not release its unit borrow");
    }
    const LIVE_ADMISSION: &str = r"#![no_std]
use wost::arch::current::{pci, vtd::Vtd};
fn probe(unit: &Vtd, authority: pci::NvmeBusMasterAuthority<'_>, state: &mut pci_config::CommandState) {
    let domain = unit.domain().unwrap();
    let interrupts = unit.interrupt_isolation().unwrap();
    let _ = pci::admit_bus_master(authority, domain, interrupts, state);
}
";
    require_success(&paths, "vtd-live-paired-admission", LIVE_ADMISSION)
        .expect("both borrowed proofs must authorize one live PCI admission");
}
