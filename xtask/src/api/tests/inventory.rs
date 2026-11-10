use super::*;

#[test]
fn procedural_attribute_and_derive_syntax_is_refused_before_inventory() {
    for source in [
        "#[generate_api]\npub struct Seed;\nfn main() {}\n",
        "#[derive(tool::GenerateApi)]\npub struct Seed;\nfn main() {}\n",
    ] {
        let root = fixture(source);
        let error =
            Inventory::discover(&root).expect_err("procedural attribute syntax must be refused");
        assert!(
            format!("{error:#}").contains("built-in trust boundary"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove attribute fixture");
    }
}

#[test]
fn cfg_attr_cannot_hide_a_semantic_runtime_root() {
    for attribute in [
        "unsafe(no_mangle)",
        "unsafe(export_name = \"exported\")",
        "used",
        "global_allocator",
    ] {
        let declaration = if attribute == "used" || attribute == "global_allocator" {
            format!("#[cfg_attr(all(), {attribute})]\npub static EXPORTED: usize = 0;\n")
        } else {
            format!(
                "#[cfg_attr(all(), {attribute})]\npub extern \"C\" fn exported() {{ api(); }}\n"
            )
        };
        let root = fixture(&format!(
            "#![warn(deprecated)]\npub fn api() {{}}\n{declaration}fn main() {{}}\n"
        ));
        let compiled = Command::new("rustc")
            .current_dir(&root)
            .args([
                "--edition=2024",
                "libs/fixture/src/main.rs",
                "--emit=metadata",
                "-o",
                "fixture.rmeta",
            ])
            .output()
            .expect("compile conditional-root fixture");
        if attribute != "global_allocator" {
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
        }
        let error = Inventory::discover(&root)
            .expect_err("a conditional semantic runtime root must be refused");
        assert!(
            format!("{error:#}").contains("conditionally controls a semantic runtime root"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove conditional-root fixture");
    }
}

#[test]
fn unicode_columns_instrument_the_exact_nested_item() {
    assert_eq!(
        byte_offset("a🧙z", LineColumn { line: 1, column: 2 }).expect("Unicode byte offset"),
        5
    );
    let root = fixture(
        "#![warn(deprecated)]\npub fn api() { let _ = \"🧙\"; fn nested() {} nested(); }\nfn main() { api(); }\n",
    );
    let inventory = Inventory::discover(&root).expect("Unicode fixture inventory");
    let nested = inventory
        .nodes
        .iter()
        .find(|node| node.symbol == "nested")
        .map(|node| node.id)
        .expect("nested item node");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument Unicode fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read Unicode fixture diagnostics");
    }
    assert!(uses.graphs[0].live(&inventory).contains(&nested));
    std::fs::remove_dir_all(root).expect("remove Unicode fixture");
}

#[test]
fn an_adjacent_call_is_outside_a_half_open_nested_item_span() {
    let root = fixture(
        "#![warn(deprecated)]\npub fn api() {}\nfn outer(){fn inner(){}api();}\nfn main(){outer();}\n",
    );
    let inventory = Inventory::discover(&root).expect("adjacent-span fixture inventory");
    let api = inventory
        .nodes
        .iter()
        .find(|node| node.symbol == "api")
        .map(|node| node.id)
        .expect("fixture API node");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument adjacent-span fixture");
        let output = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read adjacent-span diagnostics");
    }
    assert!(!uses.dead(&inventory).iter().any(|node| node.id == api));
    std::fs::remove_dir_all(root).expect("remove adjacent-span fixture");
}

#[test]
fn reexports_docs_and_same_names_are_resolved_by_the_compiler() {
    let source = r#"#![warn(deprecated)]
mod api {
    pub fn direct() {}
    pub fn reexported() {}
    pub fn reexport_only() {}
    pub fn import_only() {}
    pub fn documented() {}
    pub fn same_name() {}
    pub mod nested { pub fn same_name() {} }
}
pub use api::{reexport_only, reexported as public_name};
/// Mentions [`api::documented`] but does not call it.
fn docs_only() {}
fn main() {
    use api::import_only as unused_import;

    api::direct();
    public_name();
    api::nested::same_name();
    docs_only();
}
"#;
    let root = fixture(source);

    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let insertions = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let result = Command::new("rustc")
            .current_dir(&root)
            .args([
                "--edition=2024",
                "libs/fixture/src/main.rs",
                "--emit=metadata",
                "--error-format=short",
                "-o",
                "fixture.rmeta",
            ])
            .output()
            .expect("compile fixture");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8_lossy(&result.stderr);
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
    let dead: Vec<&str> = uses
        .dead(&inventory)
        .iter()
        .map(|node| node.label.as_str())
        .collect();
    assert!(dead.iter().any(|label| label.contains("reexport_only")));
    assert!(dead.iter().any(|label| label.contains("import_only")));
    assert!(dead.iter().any(|label| label.contains("documented")));
    assert!(dead.iter().any(|label| label.contains("fn api::same_name")));
    assert!(!dead.iter().any(|label| label.contains("reexported")));
    assert!(!dead.iter().any(|label| label.contains("nested::same_name")));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn host_tests_are_legitimate_compiler_resolved_callers() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn production() {}
pub fn test_only() {}
fn main() { production(); }
#[cfg(test)]
mod tests {
    #[test]
    fn observer() { super::test_only(); }
}
"#,
    );
    std::fs::create_dir_all(root.join("libs/fixture/tests"))
        .expect("integration fixture directory");
    std::fs::write(
        root.join("libs/fixture/tests/public.rs"),
        "pub fn integration_only() {}\n",
    )
    .expect("integration fixture source");
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    assert!(
        !inventory
            .nodes
            .iter()
            .any(|node| node.report && node.label.contains("integration_only"))
    );
    let test_only = inventory
        .nodes
        .iter()
        .find(|node| node.label.contains("fn test_only"))
        .expect("test-only item")
        .id;
    let mut uses = GraphUses::default();
    let mut compiled = GraphUses::default();
    for round in 0..inventory.rounds {
        let insertions = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let production = compile_fixture(&root, &[]);
        uses.observe(
            0,
            RootKind::Production,
            &inventory,
            &root,
            &production,
            &insertions,
        )
        .expect("read production diagnostics");
        compiled
            .observe(
                0,
                RootKind::Production,
                &inventory,
                &root,
                &production,
                &insertions,
            )
            .expect("read production diagnostics for complete graph");
        let verification = compile_fixture(&root, &["--test"]);
        if verification.contains(&format!("{MARKER}{test_only}")) {
            compiled
                .observe(
                    1,
                    RootKind::Verification,
                    &inventory,
                    &root,
                    &verification,
                    &insertions,
                )
                .expect("read verification diagnostics for classification");
        }
    }
    let dead = uses.dead(&inventory);
    assert!(dead.iter().any(|node| node.id == test_only));
    assert!(
        !compiled
            .dead(&inventory)
            .iter()
            .any(|node| node.id == test_only)
    );
    assert!(!dead.iter().any(|node| node.label.contains("fn production")));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn private_dormant_call_chains_do_not_become_production_roots() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn dormant_api() {}
pub fn reachable_api() {}
pub fn nested_dormant_api() {}
pub fn nested_reachable_api() {}
fn dormant_leaf() { dormant_api(); }
fn dormant_middle() { dormant_leaf(); }
fn reachable_leaf() { reachable_api(); }
fn reachable_middle() {
    fn nested_dormant() { nested_dormant_api(); }
    fn nested_reachable() { nested_reachable_api(); }
    nested_reachable();
    reachable_leaf();
}
fn main() { reachable_middle(); }
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
    assert!(dead.iter().all(|node| node.report));
    assert!(
        dead.iter()
            .any(|node| node.label.contains("fn dormant_api"))
    );
    assert!(
        dead.iter()
            .any(|node| node.label.contains("fn nested_dormant_api"))
    );
    assert!(
        !dead
            .iter()
            .any(|node| node.label.contains("fn reachable_api"))
    );
    assert!(
        !dead
            .iter()
            .any(|node| node.label.contains("fn nested_reachable_api"))
    );

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn a_module_local_main_is_not_a_runtime_root() {
    let root = fixture("#![warn(deprecated)]\nmod dormant;\nfn main() {}\n");
    std::fs::write(
        root.join("libs/fixture/src/dormant.rs"),
        "pub fn target() {}\nfn main() { target(); }\n",
    )
    .expect("out-of-line module source");
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
            .any(|node| node.symbol == "dormant::target"),
        "a module-local function named main became a runtime root"
    );
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn out_of_line_modules_inherit_scope_and_parent_availability() {
    let root = fixture(
        "#![warn(deprecated)]\nmod production;\n#[cfg(test)]\nmod verification;\nfn main() {}\n",
    );
    std::fs::write(
        root.join("libs/fixture/src/production.rs"),
        "pub fn called() {}\n",
    )
    .expect("production module source");
    std::fs::create_dir_all(root.join("libs/fixture/src/verification"))
        .expect("verification module directory");
    std::fs::write(
        root.join("libs/fixture/src/verification.rs"),
        "pub fn only() {}\npub mod child;\n",
    )
    .expect("verification module source");
    std::fs::write(
        root.join("libs/fixture/src/verification/child.rs"),
        "pub fn nested_only() {}\n",
    )
    .expect("nested verification module source");

    let inventory = Inventory::discover(&root).expect("fixture inventory");
    for (symbol, report) in [
        ("production::called", true),
        ("verification::only", false),
        ("verification::child::nested_only", false),
    ] {
        let node = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == symbol)
            .unwrap_or_else(|| panic!("missing {symbol}"));
        assert_eq!(node.report, report, "wrong availability for {symbol}");
    }

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn literal_path_modules_use_the_declared_scope() {
    let root = fixture(
        "#![warn(deprecated)]\n#[path = \"alternate/implementation.rs\"]\nmod routed;\nfn main() {}\n",
    );
    std::fs::create_dir_all(root.join("libs/fixture/src/alternate"))
        .expect("path module directory");
    std::fs::write(
        root.join("libs/fixture/src/alternate/implementation.rs"),
        "pub fn target() {}\n",
    )
    .expect("path module source");

    let inventory = Inventory::discover(&root).expect("fixture inventory");
    assert!(
        inventory
            .nodes
            .iter()
            .any(|node| node.symbol == "routed::target" && node.report)
    );

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn one_source_file_cannot_have_conflicting_module_contexts() {
    let root = fixture(
        "#![warn(deprecated)]\n#[path = \"shared.rs\"]\nmod first;\n#[path = \"shared.rs\"]\nmod second;\nfn main() {}\n",
    );
    std::fs::write(
        root.join("libs/fixture/src/shared.rs"),
        "pub fn target() {}\n",
    )
    .expect("shared module source");

    let error = Inventory::discover(&root).expect_err("conflicting contexts must be refused");
    assert!(format!("{error:#}").contains("conflicting module contexts"));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn ambiguous_default_module_routes_are_refused() {
    let root = fixture("#![warn(deprecated)]\nmod split;\nfn main() {}\n");
    std::fs::write(
        root.join("libs/fixture/src/split.rs"),
        "pub fn first() {}\n",
    )
    .expect("first module candidate");
    std::fs::create_dir_all(root.join("libs/fixture/src/split")).expect("second module directory");
    std::fs::write(
        root.join("libs/fixture/src/split/mod.rs"),
        "pub fn second() {}\n",
    )
    .expect("second module candidate");

    let error = Inventory::discover(&root).expect_err("ambiguous route must be refused");
    assert!(format!("{error:#}").contains("ambiguous module route"));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn unsafe_or_nonliteral_path_routes_are_refused() {
    for declaration in [
        "#[path = concat!(\"route\", \".rs\")] mod routed;",
        "#[path = \"../../../../outside.rs\"] mod routed;",
    ] {
        let root = fixture(&format!(
            "#![warn(deprecated)]\npub fn exposed() {{}}\n{declaration}\nfn main() {{}}\n"
        ));
        let error = Inventory::discover(&root).expect_err("unsafe route must be refused");
        assert!(format!("{error:#}").contains("module path"));
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }
}

#[test]
fn only_a_top_level_target_main_is_a_language_root() {
    let root = fixture(
        "#![warn(deprecated)]\npub fn exposed() {}\nfn enclosing() { fn main() {} main(); }\nfn main() {}\n",
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let main_roots = inventory
        .nodes
        .iter()
        .filter(|node| node.symbol == "main" && node.roots.production)
        .count();
    assert_eq!(main_roots, 1, "a nested main became a runtime root");

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn a_nested_runtime_root_does_not_activate_its_dormant_lexical_parent() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_nested_runtime() {}
pub fn from_dormant_parent() {}
fn dormant_parent() {
    #[unsafe(no_mangle)]
    extern "C" fn nested_runtime() { from_nested_runtime(); }
    from_dormant_parent();
}
fn main() {}
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
            .any(|node| node.label.contains("fn from_nested_runtime"))
    );
    assert!(
        dead.iter()
            .any(|node| node.label.contains("fn from_dormant_parent"))
    );

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn verification_graphs_include_runtime_roots_from_both_cfg_surfaces() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub fn from_shared_main() {}
pub fn from_verification_runtime() {}
#[cfg(feature = "selftest")]
#[unsafe(no_mangle)]
extern "C" fn verification_runtime() { from_verification_runtime(); }
fn main() {
    #[cfg(feature = "selftest")]
    from_shared_main();
}
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let mut uses = GraphUses::default();
    for round in 0..inventory.rounds {
        let instrumentation = inventory
            .instrument(&root, round)
            .expect("instrument fixture");
        let output = compile_fixture(&root, &["--cfg", "feature=\"selftest\""]);
        uses.observe(
            0,
            RootKind::Verification,
            &inventory,
            &root,
            &output,
            &instrumentation,
        )
        .expect("read verification diagnostics");
    }
    let dead = uses.dead(&inventory);
    for live in ["fn from_shared_main", "fn from_verification_runtime"] {
        assert!(
            !dead.iter().any(|node| node.label.contains(live)),
            "verification runtime did not reach {live}"
        );
    }

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn instrumentation_rounds_select_their_exact_partition() {
    let root = fixture(
        "pub enum Choice { First, Second }\npub trait Contract { type Item; fn act(&self); }\nfn main() {}\n",
    );
    let inventory = Inventory::discover(&root).expect("partition fixture inventory");

    let expected_antichain = inventory
        .nodes
        .iter()
        .filter(|node| node.instrument && node.code & 1 != 0)
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    let antichain = inventory
        .instrument(&root, 0)
        .expect("first antichain round");
    assert_eq!(antichain.ids, expected_antichain);

    let expected_variants = inventory
        .nodes
        .iter()
        .filter(|node| node.instrument && node.isolation == Some(Isolation::Variant))
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    let expected_trait_members = inventory
        .nodes
        .iter()
        .filter(|node| node.instrument && node.isolation == Some(Isolation::TraitMember))
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    assert!(!expected_variants.is_empty());
    assert!(!expected_trait_members.is_empty());
    let variants = inventory
        .instrument(&root, inventory.antichain_rounds)
        .expect("variant isolation round");
    let trait_members = inventory
        .instrument(&root, inventory.antichain_rounds + 1)
        .expect("trait-member isolation round");
    assert_eq!(variants.ids, expected_variants);
    assert_eq!(trait_members.ids, expected_trait_members);
    assert!(variants.ids.is_disjoint(&trait_members.ids));

    std::fs::remove_dir_all(root).expect("remove instrumentation partition fixture");
}

#[test]
fn an_uncalled_trait_implementation_is_not_a_production_root() {
    let root = fixture(
        r#"#![warn(deprecated)]
pub trait Observer { fn observe(&self); }
pub struct Installed;
impl Observer for Installed { fn observe(&self) {} }
pub trait Called { fn call(&self); }
pub struct Active;
impl Called for Active { fn call(&self) {} }
pub trait GenericBound {}
pub struct Generic<T>(T);
impl<T> Generic<T> where T: GenericBound { fn unused(&self) {} }
fn main() { Active.call(); }
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
    assert!(dead.iter().any(|label| label.contains("trait Observer")));
    assert!(dead.iter().any(|label| label.contains("struct Installed")));
    assert!(
        dead.iter()
            .any(|label| label.contains("trait method Observer::observe"))
    );
    assert!(!dead.iter().any(|label| label.contains("trait Called")));
    assert!(
        !dead
            .iter()
            .any(|label| label.contains("trait method Called::call"))
    );
    assert!(
        dead.iter()
            .any(|label| label.contains("trait GenericBound"))
    );
    assert!(dead.iter().any(|label| label.contains("struct Generic")));

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn nested_test_and_selftest_cfgs_stay_out_of_the_production_inventory() {
    let root = fixture(
        r#"#[cfg(any(test, feature = "selftest"))]
pub fn verification_any() {}
#[cfg(all(test, unix))]
pub fn verification_all() {}
#[cfg_attr(not(test), cfg(any()))]
pub fn verification_cfg_attr() {}
#[cfg_attr(not(feature = "selftest"), cfg(any()))]
pub fn verification_feature_attr() {}
#[cfg(any(test, unix))]
pub fn maybe_production() {}
#[cfg_attr(test, cfg(any()))]
pub fn production_when_not_testing() {}
"#,
    );
    let inventory = Inventory::discover(&root).expect("fixture inventory");
    let labels = inventory
        .nodes
        .iter()
        .filter(|node| node.report)
        .map(|node| node.label.as_str())
        .collect::<Vec<_>>();
    for excluded in [
        "verification_any",
        "verification_all",
        "verification_cfg_attr",
        "verification_feature_attr",
    ] {
        assert!(!labels.iter().any(|label| label.contains(excluded)));
    }
    for included in ["maybe_production", "production_when_not_testing"] {
        assert!(labels.iter().any(|label| label.contains(included)));
    }

    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn deprecation_suppression_cannot_hide_an_audit_marker() {
    for attribute in [
        "#![allow(deprecated)]",
        "#![expect(deprecated)]",
        "#![cfg_attr(test, allow(warnings))]",
        "macro_rules! hidden { () => { #[allow(deprecated)] fn masked() {} } }",
        "macro_rules! hidden { ($lint:ident) => { #[allow($lint)] fn masked() {} } }\nhidden!(deprecated);",
        "macro_rules! hidden { ($attribute:ident, $lint:ident) => { #[$attribute($lint)] fn masked() {} } }\nhidden!(allow, deprecated);",
    ] {
        let root = fixture(&format!("{attribute}\npub fn api() {{}}\n"));
        let error = reject_deprecation_suppression(&root).unwrap_err();
        assert!(format!("{error:#}").contains("suppresses deprecation diagnostics"));
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    let root = fixture(
        "fn allow(_: ()) {}\nfn deprecated() {}\nmacro_rules! harmless { () => { allow(deprecated()) } }\npub fn api() {}\n",
    );
    reject_deprecation_suppression(&root).expect("ordinary calls are not lint attributes");
    std::fs::remove_dir_all(&root).expect("remove fixture");
}

#[test]
fn antichain_codes_separate_every_ordered_pair() {
    let codes = antichain_codes(1_200);
    assert_eq!(codes.len(), 1_200);
    for (left_index, left) in codes.iter().enumerate() {
        for (right_index, right) in codes.iter().enumerate() {
            if left_index != right_index {
                assert_ne!(right & !left, 0);
            }
        }
    }
}

#[cfg(test)]
#[test]
fn optional_source_directory_metadata_errors_are_not_absence() {
    assert!(optional_path_is_present(Path::new("\0")).is_err());
}
