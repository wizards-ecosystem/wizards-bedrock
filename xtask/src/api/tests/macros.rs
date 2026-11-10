use super::*;

#[test]
fn opaque_macro_expansions_cannot_manufacture_a_call_chain() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub mod api {
    pub fn expanded() {}
    pub fn dormant() {}
}
#[macro_export]
macro_rules! invoke_expanded { () => { $crate::api::expanded() } }
macro_rules! never_expanded { () => { $crate::api::dormant() } }
fn main() { invoke_expanded!(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let insertions = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &insertions,
        )
        .expect("read fixture diagnostics");
    }
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.label.as_str())
        .collect::<Vec<_>>();
    assert!(dead.iter().any(|label| label.contains("fn api::expanded")));
    assert!(
        !dead
            .iter()
            .any(|label| label.contains("macro invoke_expanded"))
    );
    assert!(dead.iter().any(|label| label.contains("fn api::dormant")));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn macro_arguments_cannot_hide_dormant_lexical_callers() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn forwarded() {}
pub fn asserted() {}
pub fn evaluated() -> bool { true }
macro_rules! hide { ($path:path) => {{ fn dormant() { $path(); } }} }
fn main() {
    let _ = hide!(forwarded);
    assert!({ fn dormant() { asserted(); } true });
    assert!(evaluated());
}
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let insertions = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &insertions,
        )
        .expect("read fixture diagnostics");
    }
    let dead = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.symbol.as_str())
        .collect::<BTreeSet<_>>();
    assert!(dead.contains("forwarded"));
    assert!(dead.contains("asserted"));
    assert!(!dead.contains("evaluated"));
    std::fs::remove_dir_all(root).expect("remove macro-argument fixture");
}

#[test]
fn locally_shadowed_standard_macros_are_not_transparent_callers() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn hidden() -> bool { true }
macro_rules! assert { ($condition:expr) => { fn dormant() { let _ = $condition; } } }
fn main() { assert!(hidden()); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("shadowed-macro inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let insertions = inventory
            .instrument(&root, round)
            .expect("instrument shadowed-macro fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &insertions,
        )
        .expect("read shadowed-macro diagnostics");
    }
    assert!(
        uses.dead(&inventory)
            .iter()
            .any(|node| node.symbol == "hidden"),
        "a locally shadowed standard macro manufactured a caller"
    );
    std::fs::remove_dir_all(root).expect("remove shadowed-standard-macro fixture");
}

#[test]
fn noncanonical_language_root_bindings_disable_macro_transparency() {
    let context = ModuleContext {
        scope: Vec::new(),
        availability: Availability::Production,
        package: "fixture".to_string(),
        crate_name: "fixture".to_string(),
        target_root: PathBuf::from("libs/fixture/src/main.rs"),
        target_kind: CargoTargetKind::Bin,
        language_main: true,
    };
    for source in [
        "extern crate evil as std;\nuse std::assert;\n",
        "extern crate self as core;\nuse core::assert;\n",
        "use evil as alloc;\nuse alloc::assert;\n",
        "use evil::std;\nuse std::assert;\n",
        "mod core {}\nuse core::assert;\n",
    ] {
        let syntax = syn::parse_file(source).expect("parse hostile root binding");
        let mut policy = MacroShadowPolicy {
            shadows_standard: false,
            reserved_root_rebinding: None,
            context: &context,
            relative: &context.target_root,
        };
        policy.visit_file(&syntax);
        assert!(policy.shadows_standard, "trusted hostile binding: {source}");
    }

    let syntax = syn::parse_file("extern crate core;\nuse core::assert;\n")
        .expect("parse canonical standard binding");
    let mut policy = MacroShadowPolicy {
        shadows_standard: false,
        reserved_root_rebinding: None,
        context: &context,
        relative: &context.target_root,
    };
    policy.visit_file(&syntax);
    assert!(!policy.shadows_standard);
}

#[test]
fn a_rebound_core_root_cannot_forge_the_approved_global_asm_macro() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::write(
        root.join("wost/src/lib.rs"),
        r#"#![no_std]
extern crate self as core;
pub fn hidden() {}
pub mod arch {
    macro_rules! global_asm {
        ($($tokens:tt)*) => { pub fn manufactured() { $crate::hidden(); } };
    }
    pub(crate) use global_asm;
}
::core::arch::global_asm!("ignored");
"#,
    )
    .expect("fake global-asm fixture");
    compile_wost_fixture(&root);
    let error =
        Inventory::discover(&root).expect_err("a rebound core root must not authorize global_asm");
    assert!(format!("{error:#}").contains("rebinds reserved language root `core`"));
    std::fs::remove_dir_all(root).expect("remove fake-global-asm fixture");
}

#[test]
fn an_item_macro_disables_unqualified_transparency_across_its_caller_target() {
    let root = fixture("pub fn foundation() {}\nfn main() {}\n");
    std::fs::create_dir_all(root.join("kernel/src")).expect("caller fixture directory");
    std::fs::write(
        root.join("kernel/src/main.rs"),
        r#"macro_rules! manufacture_shadow {
    () => { macro_rules! assert { ($value:expr) => { fn dormant() { let _ = $value; } } } }
}
manufacture_shadow!();
mod caller;
fn main() { caller::run(); }
"#,
    )
    .expect("caller target root");
    std::fs::write(
        root.join("kernel/src/caller.rs"),
        r#"pub fn hidden() -> bool { true }
pub fn run() {
    assert!(hidden());
    ::std::assert!(hidden());
}
"#,
    )
    .expect("caller target module");
    let targets = vec![
        CargoTarget {
            package: "fixture".to_string(),
            package_id: "fixture".to_string(),
            name: "fixture".to_string(),
            relative: PathBuf::from("libs/fixture/src/main.rs"),
            kind: CargoTargetKind::Bin,
            test: true,
            executable: true,
        },
        CargoTarget {
            package: "caller".to_string(),
            package_id: "caller".to_string(),
            name: "caller".to_string(),
            relative: PathBuf::from("kernel/src/main.rs"),
            kind: CargoTargetKind::Bin,
            test: true,
            executable: true,
        },
    ];
    let context = targets[1].context();
    let syntax =
        syn::parse_file("manufacture_shadow!();\n").expect("unknown item-macro policy fixture");
    let mut policy = MacroShadowPolicy {
        shadows_standard: false,
        reserved_root_rebinding: None,
        context: &context,
        relative: &context.target_root,
    };
    policy.visit_file(&syntax);
    assert!(
        policy.shadows_standard,
        "an unknown item macro preserved unqualified standard-macro trust"
    );
    let inventory =
        Inventory::discover_with_targets(&root, &targets).expect("caller macro fixture inventory");
    let file = inventory
        .files
        .iter()
        .position(|file| file.relative == Path::new("kernel/src/caller.rs"))
        .expect("classified caller module");
    assert!(inventory.is_opaque_macro_use(file, LineColumn { line: 3, column: 4 }));
    assert!(inventory.is_opaque_macro_use(file, LineColumn { line: 4, column: 4 }));
    std::fs::remove_dir_all(root).expect("remove caller macro fixture");
}

#[test]
fn a_statement_macro_can_manufacture_a_later_block_scoped_shadow() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn hidden() -> bool { true }
pub fn absolute() -> bool { true }
macro_rules! manufacture_shadow {
    () => { macro_rules! assert { ($value:expr) => { fn dormant() { let _ = $value; } } } }
}
fn main() {
    manufacture_shadow!();
    assert!(hidden());
    ::std::assert!(absolute());
}
"#,
    );
    let inventory = Inventory::discover(&root).expect("statement macro fixture inventory");
    let file = inventory
        .files
        .iter()
        .position(|file| file.relative == Path::new("libs/fixture/src/main.rs"))
        .expect("classified statement macro source");
    assert!(inventory.is_opaque_macro_use(file, LineColumn { line: 9, column: 4 }));
    assert!(inventory.is_opaque_macro_use(
        file,
        LineColumn {
            line: 10,
            column: 4
        }
    ));
    std::fs::remove_dir_all(root).expect("remove statement macro fixture");
}

#[test]
fn absolute_standard_prefixes_require_canonical_reserved_roots() {
    for alias in ["std", "core", "alloc"] {
        let source = r#"#![no_std]
#![warn(deprecated)]
extern crate self as ALIAS;
#[macro_export]
macro_rules! assert { ($value:expr) => {{ fn dormant() { let _ = $value; } }} }
pub fn hidden() -> bool { true }
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { ::ALIAS::assert!(hidden()); }
"#
        .replace("ALIAS", alias);
        let root = fixture(&source);
        compile_fixture(&root, &["--crate-type", "lib"]);
        let error = Inventory::discover(&root)
            .expect_err("a reserved language-root rebinding must be refused")
            .to_string();
        assert!(
            error.contains("rebinds reserved language root"),
            "unexpected `::{alias}` rebinding verdict: {error}"
        );
        std::fs::remove_dir_all(root).expect("remove absolute alias fixture");
    }
}

#[test]
fn item_macros_cannot_hide_generated_public_surface() {
    let root = fixture(
        r#"macro_rules! generate_api { () => { pub fn hidden() {} } }
generate_api!();
fn main() {}
"#,
    );
    let error = Inventory::discover(&root).unwrap_err();
    let report = format!("{error:#}");
    assert!(report.contains("public expansion the API audit cannot inventory"));
    assert!(report.contains("generate_api at line 2"));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn shadowed_runtime_macro_names_cannot_hide_public_surface() {
    let root = fixture(
        "macro_rules! global_asm { () => { pub fn hidden_api() {} } }\nglobal_asm!();\nfn main() {}\n",
    );
    let error = Inventory::discover(&root).expect_err("shadowed runtime macro must be refused");
    let report = format!("{error:#}");
    assert!(report.contains("public expansion the API audit cannot inventory"));
    assert!(report.contains("global_asm at line 2"));
    std::fs::remove_dir_all(root).expect("remove shadowed macro fixture");
}
