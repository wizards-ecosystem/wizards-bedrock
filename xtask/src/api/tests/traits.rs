use super::*;

#[test]
fn opaque_formatting_macros_do_not_activate_trait_bodies() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_display() {}
pub fn from_debug() {}
struct Both;
impl std::fmt::Display for Both {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        from_display();
        output.write_str("display")
    }
}
impl std::fmt::Debug for Both {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        from_debug();
        output.write_str("debug")
    }
}
fn main() { let _ = format!("{}", Both); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = Uses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read fixture diagnostics");
    }
    let dead = uses.dead(&inventory);
    assert!(dead.iter().any(|node| node.symbol == "from_display"));
    assert!(dead.iter().any(|node| node.symbol == "from_debug"));
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn dependency_trait_imports_do_not_authorize_local_aliases() {
    let syntax = syn::parse_file(
        "use dependency::Observer;\nuse crate::local::Observer as LocalObserver;\n",
    )
    .expect("parse imports");
    let imports = TraitImports::collect(&syntax.items);

    assert!(imports.external_trait(&[], "Observer"));
    assert!(!imports.external_trait(&[], "LocalObserver"));
}

#[test]
fn standard_trait_imports_have_external_authority() {
    let syntax = syn::parse_file("use core::fmt::Display;\n").expect("parse standard import");
    let imports = TraitImports::collect(&syntax.items);
    assert!(imports.external_trait(&[], "Display"));
}

#[test]
fn dependency_roots_cannot_forge_implicit_standard_traits() {
    for (source, drop_path, allocator_path) in [
        (
            "extern crate evil as std;\n",
            Some("std::ops::Drop"),
            "std::alloc::GlobalAlloc",
        ),
        (
            "extern crate evil as core;\n",
            Some("core::ops::Drop"),
            "core::alloc::GlobalAlloc",
        ),
        (
            "extern crate evil as alloc;\n",
            None,
            "alloc::alloc::GlobalAlloc",
        ),
    ] {
        let syntax = syn::parse_file(source).expect("parse dependency root alias");
        let imports = TraitImports::collect(&syntax.items);
        if let Some(drop_path) = drop_path {
            assert_eq!(imports.implicit_standard_trait(&[], drop_path), None);
        }
        assert_eq!(imports.implicit_standard_trait(&[], allocator_path), None);
    }

    let syntax = syn::parse_file("extern crate core;\n").expect("parse canonical root");
    let imports = TraitImports::collect(&syntax.items);
    assert_eq!(
        imports.implicit_standard_trait(&[], "core::ops::Drop"),
        Some(ImplicitTrait::Drop)
    );
    assert_eq!(
        imports.implicit_standard_trait(&[], "core::alloc::GlobalAlloc"),
        Some(ImplicitTrait::GlobalAlloc)
    );
}

#[test]
fn same_named_types_in_other_crates_do_not_ambiguate_trait_dispatch() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn target() {}
trait Observe { fn observe(&self); }
struct Subject;
impl Observe for Subject { fn observe(&self) { target(); } }
fn main() { Observe::observe(&Subject); }
"#,
    );
    std::fs::create_dir_all(root.join("libs/other/src")).expect("other source directory");
    std::fs::write(root.join("libs/other/src/lib.rs"), "pub struct Subject;\n")
        .expect("other same-name type");
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = Uses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("resolve same-name fixture diagnostics");
    }
    assert!(
        !uses
            .dead(&inventory)
            .iter()
            .any(|node| node.symbol == "target")
    );
    std::fs::remove_dir_all(root).expect("remove same-name fixture");
}

#[test]
fn crate_modules_and_alias_chains_are_not_external_traits() {
    let syntax = syn::parse_file(
        r#"mod local { pub trait Observer {} }
use local as route;
use route::Observer as LocalObserver;
"#,
    )
    .expect("parse local imports");
    let imports = TraitImports::collect(&syntax.items);

    assert!(!imports.external_trait(&[], "LocalObserver"));
}

#[test]
fn local_declarations_shadow_prelude_trait_names() {
    let syntax = syn::parse_file("trait Drop {}\n").expect("parse local trait");
    let imports = TraitImports::collect(&syntax.items);

    assert!(!imports.external_trait(&[], "Drop"));
}

#[test]
fn every_edition_2024_prelude_trait_is_external() {
    let imports = TraitImports::default();

    for name in [
        "AsMut",
        "AsRef",
        "Clone",
        "Copy",
        "Default",
        "DoubleEndedIterator",
        "Drop",
        "Eq",
        "ExactSizeIterator",
        "Extend",
        "Fn",
        "FnMut",
        "FnOnce",
        "From",
        "FromIterator",
        "Future",
        "Hash",
        "Into",
        "IntoFuture",
        "IntoIterator",
        "Iterator",
        "Ord",
        "PartialEq",
        "PartialOrd",
        "Send",
        "Sized",
        "Sync",
        "ToOwned",
        "TryFrom",
        "TryInto",
        "Unpin",
    ] {
        assert!(imports.external_trait(&[], name), "missed {name}");
    }
}

#[test]
fn prelude_traits_remain_resolved_beneath_local_globs() {
    let syntax = syn::parse_file("mod tests { use super::*; }\n").expect("parse glob import");
    let imports = TraitImports::collect(&syntax.items);

    assert!(imports.external_trait(&["tests".to_string()], "Drop"));
}

#[test]
fn local_globs_can_shadow_prelude_trait_names() {
    let syntax = syn::parse_file("trait Drop {}\nmod tests { use super::*; }\n")
        .expect("parse shadowing glob import");
    let imports = TraitImports::collect(&syntax.items);

    assert!(!imports.external_trait(&["tests".to_string()], "Drop"));
}

#[test]
fn workspace_qualified_trait_implementations_bind_to_the_declaring_crate() {
    let root = fixture(
        r#"#![warn(deprecated)]
extern crate self as fixture;
pub fn from_workspace_impl() {}
pub struct Installed;
pub mod route { pub use crate::Installed; }
pub trait Observer { fn observe(&self); }
#[path = "../../../xtask/src/observer.rs"]
mod observer;
fn main() { observer::run(); }
"#,
    );
    std::fs::create_dir_all(root.join("xtask/src")).expect("external caller directory");
    std::fs::write(
        root.join("xtask/src/observer.rs"),
        r#"use fixture::route::Installed;
impl fixture::Observer for Installed {
    fn observe(&self) { fixture::from_workspace_impl(); }
}
pub fn run() { fixture::Observer::observe(&Installed); }
"#,
    )
    .expect("external trait implementation");
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = Uses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("resolve workspace-qualified trait implementation");
    }
    let dead = uses.dead(&inventory);
    assert!(!dead.iter().any(|node| node.symbol == "from_workspace_impl"));
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn an_unresolved_trait_implementation_refuses_a_clean_verdict() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_compound_self() {}
struct Part;
trait Action { fn act(&self); }
impl Action for (Part, Part) { fn act(&self) { from_compound_self(); } }
fn main() { (Part, Part).act(); }
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut refusal = None;
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &[]);
        if let Err(error) = Uses::default().observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        ) {
            refusal = Some(format!("{error:#}"));
            break;
        }
    }
    assert!(
        refusal
            .as_deref()
            .is_some_and(|error| error.contains("unresolved trait implementation"))
    );
    std::fs::remove_dir_all(&root).expect("remove fixture");
}
