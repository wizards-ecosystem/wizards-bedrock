use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, Macro};

use crate::paths::Paths;

use super::{first_function, is_named_path, module_source, require_error};

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

    let source = module_source(paths.root(), "wost/src/arch/x86_64/cpu.rs");
    let syntax = syn::parse_file(&source).expect("parse CPU source");
    let function = |name: &str| first_function(&syntax, name);
    let public = function("invalidate_page").expect("CPU source has no invalidate_page");
    assert!(
        function("invalidate_page_unchecked").is_none(),
        "CPU source retained a raw invalidation door"
    );

    assert!(matches!(public.vis, syn::Visibility::Public(_)));
    assert!(public.sig.unsafety.is_none());
    let mut arguments = public.sig.inputs.iter();
    let Some(syn::FnArg::Typed(address)) = arguments.next() else {
        panic!("invalidate_page has no typed address authority");
    };
    assert!(arguments.next().is_none());
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

#[derive(PartialEq, Eq, Debug)]
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
        let loads_root = is_path_to(&expression.func, "cpu", "set_page_table_base")
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

fn is_path_to(expression: &Expr, module: &str, name: &str) -> bool {
    matches!(expression, Expr::Path(function)
        if function.qself.is_none()
            && function.path.segments.len() == 2
            && function.path.segments[0].ident == module
            && function.path.segments[1].ident == name)
}

fn is_root_path(expression: &Expr) -> bool {
    is_named_path(expression, "root")
}

#[test]
fn paging_activation_loads_cr3_before_publishing_ready() {
    let paths = Paths::discover().expect("workspace paths");
    let source = module_source(paths.root(), "wost/src/arch/x86_64/paging.rs");
    let syntax = syn::parse_file(&source).expect("parse paging source");
    let activate = first_function(&syntax, "activate")
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

#[derive(PartialEq, Eq, Debug)]
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
    let init =
        first_function(&syntax, "init").expect("task source has no production init function");

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

#[derive(PartialEq, Eq, Debug)]
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
        let loads_wanted = is_path_to(&expression.func, "cpu", "set_page_table_base")
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

        let starts_extended_state =
            is_path_to(&expression.func, "xstate", "is_enabled") && expression.args.is_empty();
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
        if is_path_to(&call.func, "cpu", "page_table_base") && call.args.is_empty());
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
    let prepare =
        first_function(&syntax, "prepare").expect("task source has no production prepare function");

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
