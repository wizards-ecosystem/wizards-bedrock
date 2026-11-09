use syn::{Expr, Item, Pat, ReturnType, Stmt, Type};

use super::{
    field_chain_is, function, integer_literal_is, method, parse, path_ends, path_expression_is,
};

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

    let policy = parse("libs/pci-config/src/fence.rs");
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
