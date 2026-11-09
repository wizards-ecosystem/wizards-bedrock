use super::*;

#[test]
fn reviewed_nominal_drop_types_activate_only_from_exact_constructor_spans() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::write(
        root.join("wost/src/lib.rs"),
        r#"#![warn(deprecated)]
pub mod mm;
pub mod sync;
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { sync::construct(); mm::mention(); }
"#,
    )
    .expect("drop fixture crate root");
    std::fs::write(
        root.join("wost/src/sync.rs"),
        r#"pub fn unit_dropped() {}
pub fn tuple_dropped() {}
pub fn record_dropped() {}
pub struct SpinLockGuard;
impl std::ops::Drop for SpinLockGuard { fn drop(&mut self) { unit_dropped(); } }
pub struct ForgottenSpinLockGuard(pub u8);
impl std::ops::Drop for ForgottenSpinLockGuard { fn drop(&mut self) { tuple_dropped(); } }
pub struct ThreadLockGuard { pub value: u8 }
impl std::ops::Drop for ThreadLockGuard { fn drop(&mut self) { record_dropped(); } }
use self::{
    ForgottenSpinLockGuard as TupleAlias,
    SpinLockGuard as UnitAlias,
    ThreadLockGuard as RecordAlias,
};
pub fn construct() {
    let _ = UnitAlias;
    let _ = TupleAlias(1);
    let _ = RecordAlias { value: 2 };
}
"#,
    )
    .expect("drop fixture constructors");
    std::fs::create_dir_all(root.join("wost/src/mm")).expect("drop fixture module");
    std::fs::write(
        root.join("wost/src/mm/mod.rs"),
        r#"pub fn dormant_drop() {}
pub fn type_only_drop() {}
pub fn static_drop() {}
pub fn custom_drop() {}
pub struct Frame;
impl std::ops::Drop for Frame { fn drop(&mut self) { dormant_drop(); } }
impl Frame { pub fn from_raw() -> Self { Self } }
pub fn construct_dormant() { let _ = Frame; }
pub struct Segment;
impl std::ops::Drop for Segment { fn drop(&mut self) { type_only_drop(); } }
impl Segment {
    pub fn from_raw() -> Self { Self }
    pub const SIZE: usize = 1;
    #[allow(non_upper_case_globals)]
    pub const Segment: usize = 2;
    pub fn size() -> usize { Self::SIZE }
}
pub struct USegment;
impl std::ops::Drop for USegment { fn drop(&mut self) { static_drop(); } }
impl From<()> for USegment { fn from(_: ()) -> Self { Self } }
pub static STATIC_VALUE: USegment = USegment;
mod hostile {
    pub trait Drop { fn drop(&mut self); }
    pub struct Live;
    impl Drop for Live { fn drop(&mut self) { super::custom_drop(); } }
    pub fn construct() { let _ = Live; }
}
pub fn mention() {
    let _ = std::mem::size_of::<Segment>();
    let _ = Segment::SIZE;
    let _ = Segment::Segment;
    let _ = Segment::size();
    let _ = &STATIC_VALUE;
    hostile::construct();
}
"#,
    )
    .expect("drop fixture negative cases");
    let inventory = Inventory::discover(&root).expect("drop fixture inventory");
    let mut uses = Uses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument drop fixture");
        let output = compile_wost_fixture(&root);
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read drop fixture diagnostics");
    }
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("sync::unit_dropped"));
    assert!(!dead.contains("sync::tuple_dropped"));
    assert!(!dead.contains("sync::record_dropped"));
    assert!(dead.contains("mm::dormant_drop"));
    assert!(dead.contains("mm::type_only_drop"));
    assert!(dead.contains("mm::static_drop"));
    assert!(dead.contains("mm::custom_drop"));
    std::fs::remove_dir_all(root).expect("remove drop fixture");
}

#[test]
fn a_reviewed_owned_self_producer_activates_drop_without_type_use_shortcuts() {
    for (body, drop_live) in [
        ("let _window = Window::open();", true),
        (
            "let _ = core::mem::size_of::<Window>(); let _ = Window::status();",
            false,
        ),
    ] {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
            .expect("Drop producer fixture module directory");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            r#"#![warn(deprecated)]
pub mod arch { pub mod x86_64 { pub mod usercopy; } }
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { arch::x86_64::usercopy::exercise(); }
"#,
        )
        .expect("Drop producer fixture crate root");
        std::fs::write(
            root.join("wost/src/arch/x86_64/usercopy.rs"),
            format!(
                r#"pub fn drop_body_api() {{}}
pub struct Window;
impl Window {{
    pub fn open() -> Self {{ Self }}
    pub fn status() -> usize {{ 1 }}
}}
impl core::ops::Drop for Window {{
    fn drop(&mut self) {{ drop_body_api(); }}
}}
pub fn exercise() {{ {body} }}
"#
            ),
        )
        .expect("Drop producer fixture source");
        let inventory = Inventory::discover(&root).expect("Drop producer inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument Drop producer fixture");
            let output = compile_wost_fixture(&root);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read Drop producer diagnostics");
        }
        assert_eq!(
            !uses
                .dead(&inventory)
                .iter()
                .any(|node| node.symbol == "arch::x86_64::usercopy::drop_body_api"),
            drop_live,
            "unexpected Drop verdict for `{body}`"
        );
        std::fs::remove_dir_all(root).expect("remove Drop producer fixture");
    }
}

#[test]
fn plain_type_from_dispatch_reaches_the_reviewed_usegment_drop_producer() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::create_dir_all(root.join("wost/src/mm"))
        .expect("USegment producer fixture module directory");
    std::fs::write(
        root.join("wost/src/lib.rs"),
        r#"#![warn(deprecated)]
pub mod mm;
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { mm::construct(); }
"#,
    )
    .expect("USegment producer fixture root");
    std::fs::write(
        root.join("wost/src/mm/mod.rs"),
        r#"pub fn drop_body_api() {}
pub struct Segment;
pub struct USegment;
impl From<Segment> for USegment { fn from(_: Segment) -> Self { Self } }
impl core::ops::Drop for USegment { fn drop(&mut self) { drop_body_api(); } }
pub fn construct() { let _ = <USegment as From<Segment>>::from(Segment); }
"#,
    )
    .expect("USegment producer fixture source");
    let inventory = Inventory::discover(&root).expect("USegment producer inventory");
    let mut uses = Uses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument USegment producer fixture");
        let output = compile_wost_fixture(&root);
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read USegment producer diagnostics");
    }
    uses.validate_dispatches(&inventory)
        .expect("explicit USegment From dispatch resolves exactly");
    assert!(
        !uses
            .dead(&inventory)
            .iter()
            .any(|node| node.symbol == "mm::drop_body_api")
    );
    std::fs::remove_dir_all(root).expect("remove USegment producer fixture");
}

#[test]
fn reviewed_dma_owners_activate_drop_only_from_exact_owned_producers() {
    for (owner, definition, construction) in [
        (
            "DmaSegment",
            "pub struct Segment; pub struct DmaSegment; impl From<Segment> for DmaSegment { fn from(_: Segment) -> Self { Self } }",
            "let _owner = <DmaSegment as From<Segment>>::from(Segment);",
        ),
        (
            "PublishedIommuTable",
            "pub struct IommuTable; pub(crate) struct PublishedIommuTable; impl PublishedIommuTable { pub(crate) fn new(_: IommuTable) -> Self { Self } }",
            "let _owner = PublishedIommuTable::new(IommuTable);",
        ),
    ] {
        for constructed in [true, false] {
            let root = fixture("pub fn foundation() {}\nfn main() {}\n");
            std::fs::create_dir_all(root.join("wost/src/mm")).expect("DMA Drop fixture directory");
            std::fs::write(
                root.join("wost/src/lib.rs"),
                r#"#![warn(deprecated)]
pub mod mm;
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { mm::exercise(); }
"#,
            )
            .expect("DMA Drop fixture root");
            let body = if constructed {
                construction.to_string()
            } else {
                format!("let _ = core::mem::size_of::<{owner}>();")
            };
            std::fs::write(
                root.join("wost/src/mm/mod.rs"),
                format!("pub fn drop_body_api() {{}}\n{definition}\nimpl core::ops::Drop for {owner} {{ fn drop(&mut self) {{ drop_body_api(); }} }}\npub fn exercise() {{ {body} }}\n"),
            ).expect("DMA Drop fixture source");
            let inventory = Inventory::discover(&root).expect("reviewed DMA Drop inventory");
            let mut uses = Uses::default();
            for round in 0..inventory.rounds {
                let instrumentation = inventory
                    .instrument(&root, round)
                    .expect("instrument DMA Drop fixture");
                let output = compile_wost_fixture(&root);
                uses.observe(
                    RootKind::Production,
                    &inventory,
                    &root,
                    &output,
                    &instrumentation,
                )
                .expect("read DMA Drop diagnostics");
            }
            uses.validate_dispatches(&inventory)
                .expect("exact DMA producer dispatch");
            assert_eq!(
                !uses
                    .dead(&inventory)
                    .iter()
                    .any(|node| node.symbol == "mm::drop_body_api"),
                constructed,
                "{owner} Drop activation must follow an owned construction, not a type mention",
            );
            std::fs::remove_dir_all(root).expect("remove DMA Drop fixture");
        }
    }
}

#[test]
fn a_new_owned_self_drop_producer_is_refused_until_reviewed() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
        .expect("unreviewed producer fixture module directory");
    std::fs::write(
        root.join("wost/src/lib.rs"),
        "pub mod arch { pub mod x86_64 { pub mod usercopy; } }\n",
    )
    .expect("unreviewed producer fixture root");
    std::fs::write(
        root.join("wost/src/arch/x86_64/usercopy.rs"),
        r#"pub struct Window;
impl Window {
    pub fn open() -> Self { Self }
    pub fn alternate() -> Self { Self }
}
impl core::ops::Drop for Window { fn drop(&mut self) {} }
"#,
    )
    .expect("unreviewed producer fixture source");
    let error = Inventory::discover(&root)
        .expect_err("a new owned-Self producer must require review")
        .to_string();
    assert!(
        error.contains("unreviewed implicit Drop producer"),
        "{error}"
    );
    std::fs::remove_dir_all(root).expect("remove unreviewed producer fixture");
}

#[test]
fn a_cross_file_owned_self_drop_producer_is_refused_until_reviewed() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
        .expect("cross-file producer fixture module directory");
    std::fs::write(
        root.join("wost/src/lib.rs"),
        r#"pub mod arch { pub mod x86_64 { pub mod usercopy; } }
mod sibling;
"#,
    )
    .expect("cross-file producer fixture root");
    std::fs::write(
        root.join("wost/src/arch/x86_64/usercopy.rs"),
        r#"pub struct Window;
impl Window { pub fn open() -> Self { Self } }
impl core::ops::Drop for Window { fn drop(&mut self) {} }
"#,
    )
    .expect("cross-file producer target source");
    std::fs::write(
        root.join("wost/src/sibling.rs"),
        r#"impl crate::arch::x86_64::usercopy::Window {
    pub fn alternate() -> Self { Self }
}
"#,
    )
    .expect("cross-file producer source");
    compile_wost_fixture(&root);
    let error = Inventory::discover(&root)
        .expect_err("a cross-file owned-Self producer must require review")
        .to_string();
    assert!(
        error.contains("unreviewed implicit Drop producer"),
        "{error}"
    );
    std::fs::remove_dir_all(root).expect("remove cross-file producer fixture");
}

#[test]
fn a_private_generic_qemu_lifetime_has_one_reviewed_drop_producer() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::create_dir_all(root.join("xtask/src")).unwrap();
    std::fs::write(root.join("xtask/src/lib.rs"), "mod qemu;\n").unwrap();
    std::fs::create_dir_all(root.join("xtask/src/qemu")).unwrap();
    std::fs::write(root.join("xtask/src/qemu.rs"), "mod selftest;\n").unwrap();
    let source = r#"struct QemuLifetime<C> { child: C }
impl<C> QemuLifetime<C> { fn new(child: C) -> Self { Self { child } } }
impl<C> Drop for QemuLifetime<C> { fn drop(&mut self) {} }
"#;
    std::fs::write(root.join("xtask/src/qemu/selftest.rs"), source).unwrap();
    let inventory = Inventory::discover(&root).expect("reviewed private generic owner");
    let producers = inventory
        .nodes
        .iter()
        .filter(|node| {
            node.reviewed_drop_producer.is_some_and(|index| {
                let producer = &REVIEWED_IMPLICIT_DROP_PRODUCERS[index];
                producer.relative == "xtask/src/qemu/selftest.rs"
                    && producer.owner == "qemu::selftest::QemuLifetime"
                    && producer.member == "new"
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(producers.len(), 1);
    assert!(producers[0].direct_self_construction);
    let drops = inventory
        .nodes
        .iter()
        .filter(|node| {
            matches!(&node.trait_route, Some(TraitRoute::Implementation {
            key, implicit: Some(ImplicitTrait::Drop), ..
        }) if key.member == "drop")
        })
        .collect::<Vec<_>>();
    assert_eq!(drops.len(), 1);
    assert!(inventory.implicit_trait_activations.iter().any(|activation| {
        matches!(activation.trigger, ImplicitTraitTrigger::LiveNode(node) if node == producers[0].id)
            && activation.implementation == drops[0].id
    }));
    std::fs::write(root.join("xtask/src/qemu/selftest.rs"), format!(
        "{source}impl<C> QemuLifetime<C> {{ fn alternate(child: C) -> Self {{ Self {{ child }} }} }}\n"
    )).unwrap();
    let error = Inventory::discover(&root).expect_err("alternate constructor is not reviewed");
    assert!(
        error
            .to_string()
            .contains("unreviewed implicit Drop producer")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_reviewed_drop_producer_resolves_in_the_current_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a repository parent");
    let mut targets = fixture_targets(root).expect("current source fixture targets");
    for target in &mut targets {
        if target.relative == Path::new("kernel/src/main.rs") {
            target.package = crate::paths::KERNEL_PACKAGE.to_string();
            target.package_id = crate::paths::KERNEL_PACKAGE.to_string();
        }
    }
    let inventory =
        Inventory::discover_with_targets(root, &targets).expect("current source API inventory");
    for (index, producer) in REVIEWED_IMPLICIT_DROP_PRODUCERS.iter().enumerate() {
        let matches = inventory
            .nodes
            .iter()
            .filter(|node| node.reviewed_drop_producer == Some(index))
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "reviewed producer {}::{} did not resolve exactly once",
            producer.owner,
            producer.member
        );
        assert!(matches[0].direct_self_construction);
    }
}

#[test]
fn a_reviewed_drop_producer_must_belong_to_its_exact_result_type() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
        .expect("producer-owner fixture module directory");
    std::fs::write(
        root.join("wost/src/lib.rs"),
        "pub mod arch { pub mod x86_64 { pub mod usercopy; } }\n",
    )
    .expect("producer-owner fixture root");
    std::fs::write(
        root.join("wost/src/arch/x86_64/usercopy.rs"),
        r#"pub struct Other;
pub struct Window;
impl Window { pub fn open() -> Self { Self } }
impl core::ops::Drop for Window { fn drop(&mut self) {} }
"#,
    )
    .expect("producer-owner fixture source");
    let inventory = Inventory::discover(&root).expect("producer-owner inventory");
    let mut nodes = inventory.nodes.clone();
    let producer = nodes
        .iter_mut()
        .find(|node| {
            node.reviewed_drop_producer
                == REVIEWED_IMPLICIT_DROP_PRODUCERS
                    .iter()
                    .position(|producer| producer.produced == "arch::x86_64::usercopy::Window")
        })
        .expect("reviewed Window producer");
    producer.implementation_owner = Some("arch::x86_64::usercopy::Other".to_string());
    let error = trait_activations(&nodes, &inventory.files)
        .expect_err("a producer owned by another type must be refused")
        .to_string();
    assert!(error.contains("does not belong to its exact result type"));
    std::fs::remove_dir_all(root).expect("remove producer-owner fixture");
}

#[test]
fn a_new_implicit_drop_contract_is_refused_until_reviewed() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn drop_body_api() {}
struct Unreviewed;
impl std::ops::Drop for Unreviewed { fn drop(&mut self) { drop_body_api(); } }
fn main() { let _ = Unreviewed; }
"#,
    );
    compile_fixture(&root, &[]);
    let error = Inventory::discover(&root)
        .expect_err("an unreviewed implicit Drop implementation must be refused");
    assert!(
        format!("{error:#}").contains("reviewed implicit-drop boundary"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).expect("remove unreviewed Drop fixture");

    let root = fixture(
        r#"pub mod sync {
    pub fn drop_body_api() {}
    pub struct SpinLockGuard;
    impl std::ops::Drop for SpinLockGuard {
        fn drop(&mut self) { drop_body_api(); }
    }
}
fn main() { let _ = sync::SpinLockGuard; }
"#,
    );
    compile_fixture(&root, &[]);
    let targets = [CargoTarget {
        package: "impostor".to_string(),
        package_id: "impostor-id".to_string(),
        name: "wost".to_string(),
        relative: PathBuf::from("libs/fixture/src/main.rs"),
        kind: CargoTargetKind::Bin,
        test: true,
        executable: true,
    }];
    let error = Inventory::discover_with_targets(&root, &targets)
        .expect_err("a colliding Cargo target name must not inherit a Drop contract");
    assert!(
        format!("{error:#}").contains("reviewed implicit-drop boundary"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).expect("remove colliding Drop fixture");
}

#[test]
fn reviewed_drop_activation_is_an_explicit_construction_boundary_for_leaks() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::write(
        root.join("wost/src/lib.rs"),
        r#"#![warn(deprecated)]
pub mod sync;
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { sync::leak(); }
"#,
    )
    .expect("Drop leak fixture crate root");
    std::fs::write(
        root.join("wost/src/sync.rs"),
        r#"pub fn drop_body_api() {}
pub struct SpinLockGuard;
impl std::ops::Drop for SpinLockGuard { fn drop(&mut self) { drop_body_api(); } }
pub fn leak() {
    std::mem::forget(SpinLockGuard);
    let bound = SpinLockGuard;
    std::mem::forget(bound);
    let _manual = std::mem::ManuallyDrop::new(SpinLockGuard);
}
"#,
    )
    .expect("Drop leak fixture source");
    let inventory = Inventory::discover(&root).expect("Drop leak fixture inventory");
    let mut uses = Uses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument Drop leak fixture");
        let output = compile_wost_fixture(&root);
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read Drop leak fixture diagnostics");
    }
    assert!(
        !uses
            .dead(&inventory)
            .iter()
            .any(|node| node.symbol == "sync::drop_body_api"),
        "the reviewed construction boundary stopped activating its Drop contract"
    );
    std::fs::remove_dir_all(root).expect("remove Drop leak fixture");
}

#[test]
fn a_global_allocator_activates_only_its_exact_global_alloc_implementation() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn allocated() {}
pub fn dormant_allocator() {}
pub fn custom_allocator_trait() {}
struct Selected;
unsafe impl std::alloc::GlobalAlloc for Selected {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 {
        allocated();
        std::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) { allocated(); }
}
trait GlobalAlloc { fn allocate(&self); }
impl GlobalAlloc for Selected { fn allocate(&self) { custom_allocator_trait(); } }
struct Dormant;
unsafe impl std::alloc::GlobalAlloc for Dormant {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 {
        dormant_allocator();
        std::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
#[global_allocator]
static ALLOCATOR: Selected = Selected;
fn main() { let _ = Box::new(1_u8); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("allocator fixture inventory");
    let mut uses = Uses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument allocator fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read allocator fixture diagnostics");
    }
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(!dead.contains("allocated"));
    assert!(dead.contains("dormant_allocator"));
    assert!(dead.contains("custom_allocator_trait"));
    std::fs::remove_dir_all(root).expect("remove allocator fixture");
}

#[test]
fn a_global_allocator_type_alias_is_refused_without_exact_impl_identity() {
    let root = fixture(
        r#"struct Selected;
unsafe impl std::alloc::GlobalAlloc for Selected {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 { std::ptr::null_mut() }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
type Alias = Selected;
#[global_allocator]
static ALLOCATOR: Alias = Selected;
pub fn api() {}
fn main() { let _ = Box::new(1_u8); }
"#,
    );
    compile_fixture(&root, &[]);
    let error =
        Inventory::discover(&root).expect_err("an unresolved allocator type alias must be refused");
    assert!(format!("{error:#}").contains("unsupported type alias"));
    std::fs::remove_dir_all(root).expect("remove allocator-alias fixture");
}

#[test]
fn generic_global_allocator_identities_are_refused_before_type_arguments_can_collapse() {
    let root = fixture(
        r#"use std::marker::PhantomData;
struct Selected;
struct Other;
struct Wrapper<T>(PhantomData<T>);
unsafe impl std::alloc::GlobalAlloc for Wrapper<Selected> {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 { std::ptr::null_mut() }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
unsafe impl std::alloc::GlobalAlloc for Wrapper<Other> {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 { dormant(); std::ptr::null_mut() }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
#[global_allocator]
static ALLOCATOR: Wrapper<Selected> = Wrapper(PhantomData);
pub fn dormant() {}
fn main() { let _ = Box::new(1_u8); }
"#,
    );
    compile_fixture(&root, &[]);
    let error = Inventory::discover(&root)
        .expect_err("a generic global allocator identity must be refused");
    assert!(
        format!("{error:#}").contains("no exact nominal type"),
        "{error:#}"
    );
    std::fs::remove_dir_all(root).expect("remove generic allocator fixture");
}
