use super::*;

#[test]
fn explicit_runtime_and_trait_callers_reach_only_their_called_surface() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_runtime() {}
pub fn from_called_trait() {}
pub fn from_dormant_trait() {}
#[unsafe(no_mangle)]
extern "C" fn runtime_entry() { from_runtime(); }
trait Called { fn call(&self); }
struct Active;
impl Called for Active { fn call(&self) { from_called_trait(); } }
trait Dormant { fn call(&self); }
struct Installed;
impl Dormant for Installed { fn call(&self) { from_dormant_trait(); } }
fn main() { Active.call(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read fixture diagnostics");
    }
    let dead = uses.dead(&inventory);
    assert!(
        !dead
            .iter()
            .any(|node| node.label.contains("fn from_runtime"))
    );
    assert!(
        !dead
            .iter()
            .any(|node| node.label.contains("fn from_called_trait"))
    );
    assert!(
        dead.iter()
            .any(|node| node.label.contains("fn from_dormant_trait"))
    );

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn one_called_trait_implementation_does_not_activate_its_dormant_sibling() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_active() {}
pub fn from_dormant() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { from_active(); } }
struct Dormant;
impl Action for Dormant { fn act(&self) { from_dormant(); } }
fn main() { Active.act(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read fixture diagnostics");
    }
    let dead = uses.dead(&inventory);
    assert!(!dead.iter().any(|node| node.symbol == "from_active"));
    assert!(dead.iter().any(|node| node.symbol == "from_dormant"));
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn ambiguous_receiver_types_cannot_activate_multiple_trait_implementations() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_active() {}
pub fn from_dormant() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { from_active(); } }
struct Dormant;
impl Action for Dormant { fn act(&self) { from_dormant(); } }
fn pick(_: Active, _: Dormant) -> Active { Active }
fn main() { pick(Active, Dormant).act(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("ambiguous-dispatch inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument ambiguous dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read ambiguous-dispatch diagnostics");
    }
    let reason = uses
        .validate_dispatches(&inventory)
        .expect_err("ambiguous receiver evidence must be refused")
        .to_string();
    assert!(reason.contains("resolves to 2 audited trait implementations"));
    std::fs::remove_dir_all(root).expect("remove ambiguous-dispatch fixture");
}

#[test]
fn every_invalid_dispatch_in_one_graph_is_reported() {
    let root = fixture(
        r#"#![warn(deprecated)]
trait First { fn first(&self); }
pub fn from_first_a() {}
struct FirstA;
impl First for FirstA { fn first(&self) { from_first_a(); } }
pub fn from_first_b() {}
struct FirstB;
impl First for FirstB { fn first(&self) { from_first_b(); } }
fn choose_first(_: FirstA, _: FirstB) -> FirstA { FirstA }
trait Second { fn second(&self); }
pub fn from_second_a() {}
struct SecondA;
impl Second for SecondA { fn second(&self) { from_second_a(); } }
pub fn from_second_b() {}
struct SecondB;
impl Second for SecondB { fn second(&self) { from_second_b(); } }
fn choose_second(_: SecondA, _: SecondB) -> SecondA { SecondA }
fn main() {
    choose_first(FirstA, FirstB).first();
    choose_second(SecondA, SecondB).second();
}
"#,
    );
    let inventory = Inventory::discover(&root).expect("multiple-dispatch inventory");
    let mut graphs = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument multiple dispatches");
        let output = compile_fixture(&root, &[]);
        graphs
            .observe(
                0,
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read multiple-dispatch diagnostics");
    }

    let reason = graphs
        .validate_dispatches(&inventory)
        .expect_err("every ambiguous dispatch must be refused")
        .to_string();
    let first = reason
        .lines()
        .find(|line| line.contains("dispatch `first`"))
        .expect("first dispatch refusal");
    let second = reason
        .lines()
        .find(|line| line.contains("dispatch `second`"))
        .expect("second dispatch refusal");
    for refusal in [first, second] {
        assert!(refusal.contains("resolves to 2 audited trait implementations"));
        assert!(refusal.contains("compiler graphs 0"));
    }
    std::fs::remove_dir_all(root).expect("remove multiple-dispatch fixture");
}

#[test]
fn a_later_compiler_graph_cannot_hide_its_invalid_dispatch() {
    let root = fixture(
        r#"#![warn(deprecated)]
trait Early { fn early(&self); }
pub fn from_early_a() {}
struct EarlyA;
impl Early for EarlyA { fn early(&self) { from_early_a(); } }
pub fn from_early_b() {}
struct EarlyB;
impl Early for EarlyB { fn early(&self) { from_early_b(); } }
fn choose_early(_: EarlyA, _: EarlyB) -> EarlyA { EarlyA }
trait Late { fn late(&self); }
pub fn from_late_a() {}
struct LateA;
impl Late for LateA { fn late(&self) { from_late_a(); } }
pub fn from_late_b() {}
struct LateB;
impl Late for LateB { fn late(&self) { from_late_b(); } }
fn choose_late(_: LateA, _: LateB) -> LateA { LateA }
#[cfg(audit_early)]
fn main() { choose_early(EarlyA, EarlyB).early(); }
#[cfg(not(audit_early))]
fn main() { choose_late(LateA, LateB).late(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("cross-graph dispatch inventory");
    let mut graphs = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument cross-graph dispatches");
        for (graph, options) in [&["--cfg", "audit_early"][..], &[][..]]
            .into_iter()
            .enumerate()
        {
            let output = compile_fixture(&root, options);
            graphs
                .observe(
                    graph,
                    RootKind::Production,
                    &inventory,
                    &root,
                    &output,
                    &instrumentation,
                )
                .expect("read cross-graph dispatch diagnostics");
        }
    }

    let reason = graphs
        .validate_dispatches(&inventory)
        .expect_err("invalid dispatches in every graph must be refused")
        .to_string();
    let early = reason
        .lines()
        .find(|line| line.contains("dispatch `early`"))
        .expect("early-graph dispatch refusal");
    let late = reason
        .lines()
        .find(|line| line.contains("dispatch `late`"))
        .expect("later-graph dispatch refusal");
    assert!(early.contains("compiler graphs 0"));
    assert!(late.contains("compiler graphs 1"));
    std::fs::remove_dir_all(root).expect("remove cross-graph dispatch fixture");
}

#[test]
fn a_unique_local_trait_declaration_resolves_variable_receiver_dispatch() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn reached() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { reached(); } }
fn main() { let active = Active; active.act(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("variable-dispatch inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument variable dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read variable-dispatch diagnostics");
    }
    assert!(
        !uses
            .dead(&inventory)
            .iter()
            .any(|node| node.symbol == "reached"),
        "unique declaration-identified variable dispatch was not activated"
    );
    std::fs::remove_dir_all(root).expect("remove variable-dispatch fixture");
}

#[test]
fn ambiguous_variable_receiver_dispatch_fails_closed() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_active() {}
pub fn from_other() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { from_active(); } }
struct Other;
impl Action for Other { fn act(&self) { from_other(); } }
fn main() { let active = Active; active.act(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("ambiguous-variable inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument ambiguous variable dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read ambiguous-variable diagnostics");
    }
    let reason = uses
        .validate_dispatches(&inventory)
        .expect_err("ambiguous variable dispatch must be refused")
        .to_string();
    assert!(reason.contains("resolves to 2 audited trait implementations"));
    std::fs::remove_dir_all(root).expect("remove ambiguous-variable fixture");
}

#[test]
fn an_explicit_external_variable_dispatch_activates_its_implementation() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn cloned() {}
struct Subject;
impl Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
fn main() { let subject = Subject; let _ = <Subject as Clone>::clone(&subject); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("external-variable inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument external variable dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read external-variable diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("explicit external dispatch");
    assert!(
        !uses
            .dead(&inventory)
            .iter()
            .any(|node| node.symbol == "cloned"),
        "explicit external dispatch did not activate its implementation"
    );
    std::fs::remove_dir_all(root).expect("remove external-variable fixture");
}

#[test]
fn generic_trait_arguments_preserve_the_full_explicit_dispatch_qualifier() {
    let expression = syn::parse_str::<Expr>("<Subject as module::External<'_>>::same(&subject)")
        .expect("qualified call expression");
    let mut sites = DispatchSites::default();
    sites.visit_expr(&expression);
    let [site] = sites.sites.as_slice() else {
        panic!("qualified call did not produce exactly one dispatch site");
    };
    let DispatchKind::Call {
        member,
        trait_qualifier,
    } = &site.kind;
    assert_eq!(member, "same");
    assert_eq!(trait_qualifier.as_deref(), Some("module::External"));
}

#[test]
fn qualified_dispatch_distinguishes_same_named_traits() {
    let site = DispatchSite {
        range: SourceRange {
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn { line: 1, column: 1 },
        },
        callee_evidence: Vec::new(),
        self_evidence: Vec::new(),
        exact_self_syntax: false,
        exact_callee_self_syntax: false,
        explicit_qself: true,
        generic_qself_trait: None,
        kind: DispatchKind::Call {
            member: "same".to_string(),
            trait_qualifier: Some("selected::Action".to_string()),
        },
    };
    let activation = |trait_path: &str| TraitActivation {
        self_type: 0,
        exact_self_type: true,
        declaration: None,
        key: TraitMemberKey {
            trait_path: trait_path.to_string(),
            member: "same".to_string(),
        },
        implementation: 1,
    };
    let evidence = BTreeSet::new();

    assert!(activation("selected::Action").call_compatible(&site, &evidence));
    assert!(!activation("other::Action").call_compatible(&site, &evidence));
}

#[test]
fn untyped_external_method_dispatch_does_not_guess_an_implementation() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn cloned() {}
struct Subject;
impl Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
fn main() { let subject = Subject; let _ = subject.clone(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("external-variable inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument external variable dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read external-variable diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("untyped external dispatch stays conservative");
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "cloned"),
        "untyped method syntax guessed its external implementation"
    );
    std::fs::remove_dir_all(root).expect("remove external-variable fixture");
}

#[test]
fn an_unqualified_external_dispatch_with_exact_self_fails_closed() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn cloned() {}
struct Subject;
impl Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
fn main() { let _ = Subject.clone(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("exact external Self inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument exact external Self dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read exact external Self diagnostics");
    }
    let reason = uses
        .validate_dispatches(&inventory)
        .expect_err("an unqualified external dispatch must not use Self alone")
        .to_string();
    assert!(reason.contains("resolves to 0 audited trait implementations"));
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "cloned")
    );
    std::fs::remove_dir_all(root).expect("remove exact external Self fixture");
}

#[test]
fn an_exact_inherent_method_excludes_a_same_named_external_trait_body() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_inherent() {}
pub fn from_trait() {}
struct Subject;
impl Subject {
    fn clone(&self) -> Self { from_inherent(); Subject }
}
impl std::clone::Clone for Subject {
    fn clone(&self) -> Self { from_trait(); Subject }
}
fn main() { let _ = Subject.clone(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("inherent-collision inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument inherent collision");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read inherent-collision diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("exact inherent method excludes trait resolution");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("from_inherent"));
    assert!(dead.contains("from_trait"));
    std::fs::remove_dir_all(root).expect("remove inherent-collision fixture");
}

#[test]
fn an_exact_free_call_excludes_a_same_named_external_trait_body() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_trait() {}
struct Subject;
impl std::ops::Add for Subject {
    type Output = Self;
    fn add(self, _: Self) -> Self { from_trait(); Subject }
}
fn add(_: Subject) -> Subject { Subject }
fn main() { let _ = add(Subject); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("free-call collision inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument free-call collision");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read free-call collision diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("exact free call excludes trait resolution");
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "from_trait")
    );
    std::fs::remove_dir_all(root).expect("remove free-call collision fixture");
}

#[test]
fn generic_qself_static_dispatch_credits_only_its_exact_trait_declaration() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
impl Factory for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn direct<T: Factory>() -> T { <T as Factory>::make(Argument) }
fn where_bound<T>() -> T where T: Factory { <T as Factory>::make(Argument) }
fn main() { let _ = direct::<Product>(); let _ = where_bound::<Product>(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("generic-qself inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument generic-qself dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read generic-qself diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("generic qself has an exact declaration-only route");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("Factory::make"));
    assert!(dead.contains("from_argument"));
    assert!(dead.contains("from_product"));
    std::fs::remove_dir_all(root).expect("remove generic-qself fixture");
}

#[test]
fn parameterized_generic_qself_credits_only_its_exact_trait_declaration() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory<A> { fn make(_: A) -> Self; }
pub struct Argument;
impl Factory<Argument> for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory<Argument> for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn direct<T: Factory<Argument>>() -> T {
    <T as Factory<Argument>>::make(Argument)
}
fn where_bound<T>() -> T where T: Factory<Argument> {
    <T as Factory<Argument>>::make(Argument)
}
fn main() { let _ = direct::<Product>(); let _ = where_bound::<Product>(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("parameterized generic inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument parameterized generic dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read parameterized generic diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("parameterized generic qself has a declaration-only route");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("Factory::make"));
    assert!(dead.contains("from_argument"));
    assert!(dead.contains("from_product"));
    std::fs::remove_dir_all(root).expect("remove parameterized generic fixture");
}

#[test]
fn generic_qself_rejects_indirect_and_nonparameter_shapes() {
    let syntax = syn::parse_file(
        r#"
trait Factory<'a> { fn make(); }
trait Bridge<'a>: Factory<'a> {}
trait Holder { type Item; }
struct Wrapper<T>(T);
struct Concrete;
type Alias = Concrete;
fn indirect<'a, T: Bridge<'a>>() { <T as Factory<'a>>::make(); }
fn higher_ranked<T: for<'a> Factory<'a>>() { <T as Factory<'static>>::make(); }
fn alias() { <Alias as Factory<'static>>::make(); }
fn associated<T: Holder>() { <T::Item as Factory<'static>>::make(); }
fn generic_self<T>() { <Wrapper<T> as Factory<'static>>::make(); }
"#,
    )
    .expect("parse rejected generic-qself shapes");
    let mut sites = DispatchSites::default();
    sites.visit_file(&syntax);
    assert_eq!(sites.sites.len(), 5);
    assert!(
        sites
            .sites
            .iter()
            .all(|site| site.generic_qself_trait.is_none())
    );
}

#[test]
fn unsupported_generic_qself_bounds_fail_closed() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub trait Factory<'a> { fn make() -> Self; }
pub trait Bridge: for<'a> Factory<'a> {}
pub struct Product;
impl<'a> Factory<'a> for Product { fn make() -> Self { Product } }
impl Bridge for Product {}
pub trait Holder { type Item: for<'a> Factory<'a>; }
impl Holder for Product { type Item = Product; }
fn indirect<T: Bridge>() -> T { <T as Factory<'static>>::make() }
fn higher_ranked<T: for<'a> Factory<'a>>() -> T {
    <T as Factory<'static>>::make()
}
fn associated<T: Holder>() -> T::Item {
    <T::Item as Factory<'static>>::make()
}
fn main() {
    let _ = indirect::<Product>();
    let _ = higher_ranked::<Product>();
    let _ = associated::<Product>();
}
"#,
    );
    let inventory = Inventory::discover(&root).expect("unsupported generic inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument unsupported generic dispatches");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read unsupported generic diagnostics");
    }
    let failures = uses.graphs[0].dispatch_failures(&inventory);
    assert_eq!(failures.len(), 3);
    assert!(failures.iter().all(|failure| failure.implementations == 0));
    std::fs::remove_dir_all(root).expect("remove unsupported generic fixture");
}

#[test]
fn an_exact_local_default_is_not_confused_with_a_sibling_override() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_default() {}
pub fn from_override() {}
trait Action { fn act(&self) { from_default(); } }
struct Defaulted;
impl Action for Defaulted {}
struct Overridden;
impl Action for Overridden { fn act(&self) { from_override(); } }
fn main() { <Defaulted as Action>::act(&Defaulted); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("local-default inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument local-default dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read local-default diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("exact local default dispatch");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("from_default"));
    assert!(dead.contains("from_override"));
    std::fs::remove_dir_all(root).expect("remove local-default fixture");
}

#[test]
fn generic_qself_with_concrete_self_evidence_is_not_declaration_only() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
pub struct Product;
fn generic<T: Factory>() -> T { <T as Factory>::make(Argument) }
fn main() {}
"#,
    );
    let inventory = Inventory::discover(&root).expect("generic evidence inventory");
    let (file, site) = inventory
        .files
        .iter()
        .enumerate()
        .find_map(|(file, source)| {
            source
                .dispatch_sites
                .iter()
                .position(|site| site.generic_qself_trait.as_deref() == Some("Factory"))
                .map(|site| (file, site))
        })
        .expect("generic qself dispatch site");
    let declaration = inventory
        .nodes
        .iter()
        .find_map(|node| match &node.trait_route {
            Some(TraitRoute::Declaration(key))
                if key.member == "make" && last_path_segment(&key.trait_path) == "Factory" =>
            {
                Some(node.id)
            }
            _ => None,
        })
        .expect("Factory declaration marker");
    let concrete = inventory
        .nodes
        .iter()
        .find(|node| node.type_declaration && node.symbol == "Argument")
        .map(|node| node.id)
        .expect("concrete type marker");
    let evidence = DispatchEvidence {
        caller: 0,
        nodes: [declaration, concrete].into_iter().collect(),
        callee_nodes: [declaration, concrete].into_iter().collect(),
        self_nodes: [concrete].into_iter().collect(),
    };

    assert!(!Uses::default().declaration_only_generic_dispatch(
        &inventory,
        &inventory.files[file].dispatch_sites[site],
        &evidence,
    ));
    std::fs::remove_dir_all(root).expect("remove generic evidence fixture");
}

#[test]
fn concrete_qself_static_dispatch_uses_self_not_argument_zero() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
impl Factory for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn main() { let _ = <Product as Factory>::make(Argument); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("concrete-qself inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument concrete-qself dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read concrete-qself diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("concrete qself resolves by Self");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("from_product"));
    assert!(dead.contains("from_argument"));
    std::fs::remove_dir_all(root).expect("remove concrete-qself fixture");
}

#[test]
fn type_qualified_static_dispatch_uses_callee_not_argument_zero() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
impl Factory for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn main() { let _ = Product::make(Argument); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("type-qualified inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument type-qualified dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read type-qualified diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("type-qualified dispatch resolves by its callee");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("from_product"));
    assert!(dead.contains("from_argument"));
    std::fs::remove_dir_all(root).expect("remove type-qualified fixture");
}

#[test]
fn a_type_qualified_external_trait_call_credits_its_exact_callee_type() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub struct Origin;
pub struct Argument;
impl From<Origin> for Argument {
    fn from(_: Origin) -> Self { from_argument(); Argument }
}
pub struct Product;
impl From<Argument> for Product {
    fn from(_: Argument) -> Self { from_product(); Product }
}
fn main() { let _ = Product::from(Argument); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("type-qualified external inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument type-qualified external dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read type-qualified external diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("a type-qualified external call resolves by its callee type");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("from_product"));
    assert!(dead.contains("from_argument"));
    std::fs::remove_dir_all(root).expect("remove type-qualified external fixture");
}

#[test]
fn a_type_qualified_external_trait_call_with_two_implementations_fails_closed() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_origin() {}
pub struct Origin;
pub struct Argument;
pub struct Product;
impl From<Argument> for Product {
    fn from(_: Argument) -> Self { from_argument(); Product }
}
impl From<Origin> for Product {
    fn from(_: Origin) -> Self { from_origin(); Product }
}
fn main() { let _ = Product::from(Argument); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("two-implementation inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument two-implementation dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read two-implementation diagnostics");
    }
    let reason = uses
        .validate_dispatches(&inventory)
        .expect_err("two implementations of one external trait must be refused")
        .to_string();
    assert!(reason.contains("resolves to 2 audited trait implementations"));
    std::fs::remove_dir_all(root).expect("remove two-implementation fixture");
}

#[test]
fn an_aliased_callee_does_not_borrow_argument_zero_for_an_external_trait() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub struct Origin;
pub struct Argument;
impl From<Origin> for Argument {
    fn from(_: Origin) -> Self { from_argument(); Argument }
}
pub struct Product;
impl From<Argument> for Product {
    fn from(_: Argument) -> Self { from_product(); Product }
}
pub type Alias = Product;
fn main() { let _ = Alias::from(Argument); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("aliased-callee inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument aliased-callee dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read aliased-callee diagnostics");
    }
    let reason = uses
        .validate_dispatches(&inventory)
        .expect_err("an aliased callee must not resolve through its argument")
        .to_string();
    assert!(reason.contains("resolves to 0 audited trait implementations"));
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "from_argument"),
        "argument zero supplied the Self of an aliased callee"
    );
    std::fs::remove_dir_all(root).expect("remove aliased-callee fixture");
}

#[test]
fn a_generic_variable_receiver_dispatch_fails_closed() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn reached() {}
trait Action { fn act(&self); }
struct Wrapper<T>(T);
impl<T> Action for Wrapper<T> { fn act(&self) { reached(); } }
fn main() { let value = Wrapper(0_u8); value.act(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("generic-variable inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument generic variable dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read generic-variable diagnostics");
    }
    let reason = uses
        .validate_dispatches(&inventory)
        .expect_err("erased generic dispatch must be refused")
        .to_string();
    assert!(reason.contains("resolves to 0 audited trait implementations"));
    std::fs::remove_dir_all(root).expect("remove generic-variable fixture");
}

#[test]
fn erased_generic_self_types_cannot_activate_the_wrong_trait_specialization() {
    let root = fixture(
        r#"#![warn(deprecated)]
use std::marker::PhantomData;
pub fn dormant_specialization() {}
struct Selected;
struct Other;
struct Wrapper<T> { marker: PhantomData<T> }
trait Action { fn act(&self); }
impl Action for Wrapper<Other> {
    fn act(&self) { dormant_specialization(); }
}
impl Wrapper<Selected> { fn act(&self) {} }
fn main() { Wrapper::<Selected> { marker: PhantomData }.act(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("generic-dispatch inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument generic dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read generic-dispatch diagnostics");
    }
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "dormant_specialization"),
        "an erased generic type activated the wrong trait specialization"
    );
    std::fs::remove_dir_all(root).expect("remove generic-dispatch fixture");
}

#[test]
fn unique_external_method_dispatch_activates_only_its_exact_trait() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn cloned() {}
pub fn other_cloned() {}
pub fn dormant_clone() {}
struct Subject;
impl std::clone::Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
trait Other { fn clone(&self) -> Self; }
trait Another { fn clone(&self) -> Self; }
struct Both;
impl Other for Both {
    fn clone(&self) -> Self { other_cloned(); Both }
}
impl Another for Both {
    fn clone(&self) -> Self { dormant_clone(); Both }
}
fn main() {
    let _ = <Subject as std::clone::Clone>::clone(&Subject);
    let _ = Other::clone(&Both);
}
"#,
    );
    let inventory = Inventory::discover(&root).expect("external-dispatch inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument external dispatch");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read external-dispatch diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("explicit external dispatches");
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("cloned"));
    assert!(!dead.contains("other_cloned"));
    assert!(dead.contains("dormant_clone"));
    std::fs::remove_dir_all(root).expect("remove external-dispatch fixture");
}

#[test]
fn independent_type_and_trait_uses_do_not_activate_an_implementation() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn orphan() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) {} }
struct Dormant;
impl Action for Dormant { fn act(&self) { orphan(); } }
fn main() { Active.act(); let _ = Dormant; }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read fixture diagnostics");
    }
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "orphan")
    );
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn a_reachable_type_does_not_activate_an_uncalled_trait_member() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_dormant_trait() {}
trait Action { fn act(&self); }
struct Installed;
impl Action for Installed { fn act(&self) { from_dormant_trait(); } }
fn main() { let _installed = Installed; }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read fixture diagnostics");
    }
    let dead = uses.dead(&inventory);
    assert!(dead.iter().any(|node| node.symbol == "from_dormant_trait"));
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn constructing_a_type_does_not_activate_its_external_trait_body() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn orphan() {}
struct Dormant;
impl std::fmt::Display for Dormant {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        orphan();
        output.write_str("dormant")
    }
}
fn main() { let _ = Dormant; }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read fixture diagnostics");
    }
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "orphan")
    );
    std::fs::remove_dir_all(&root).expect("remove fixture");
}
