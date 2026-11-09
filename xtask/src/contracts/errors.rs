use std::collections::{BTreeMap, BTreeSet};

use syn::visit::{self, Visit};
use syn::{Expr, ItemEnum, ItemStruct, Macro, Type};

use crate::paths::Paths;

use super::{discarded_address_space_reclaims, rust_sources_below};

#[derive(Default)]
struct PublicErrorContracts {
    enums: BTreeMap<String, bool>,
    displays: BTreeSet<String>,
    errors: BTreeSet<String>,
}

fn nominal_type_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

impl<'ast> Visit<'ast> for PublicErrorContracts {
    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        let name = item.ident.to_string();
        if matches!(item.vis, syn::Visibility::Public(_)) && name.ends_with("Error") {
            let extensible = item
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("non_exhaustive"));
            assert!(
                self.enums.insert(name.clone(), extensible).is_none(),
                "duplicate public error enum {name} in one source file"
            );
        }
        visit::visit_item_enum(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let Some((_, trait_path, _)) = &item.trait_ else {
            visit::visit_item_impl(self, item);
            return;
        };
        let Some(name) = nominal_type_name(&item.self_ty) else {
            visit::visit_item_impl(self, item);
            return;
        };
        match trait_path.segments.last().map(|segment| &segment.ident) {
            Some(trait_name) if trait_name == "Display" => {
                self.displays.insert(name);
            }
            Some(trait_name) if trait_name == "Error" => {
                self.errors.insert(name);
            }
            _ => {}
        }
        visit::visit_item_impl(self, item);
    }
}

#[test]
fn public_error_enums_have_stable_standard_contracts() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    for root in ["kernel/src", "libs", "wost/src"] {
        for source in rust_sources_below(&paths.root().join(root)) {
            let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
            let text = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
            let syntax = syn::parse_file(&text)
                .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
            let mut contracts = PublicErrorContracts::default();
            contracts.visit_file(&syntax);
            for (name, extensible) in contracts.enums {
                if !extensible {
                    violations.push(format!(
                        "{}::{name} is missing #[non_exhaustive]",
                        relative.display()
                    ));
                }
                if !contracts.displays.contains(&name) {
                    violations.push(format!("{}::{name} is missing Display", relative.display()));
                }
                if !contracts.errors.contains(&name) {
                    violations.push(format!(
                        "{}::{name} is missing core::error::Error",
                        relative.display()
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "public error contracts changed:\n{}",
        violations.join("\n")
    );
}

#[derive(Default)]
struct PublicFailures(Vec<String>);

impl<'ast> Visit<'ast> for PublicFailures {
    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        let name = item.ident.to_string();
        if matches!(item.vis, syn::Visibility::Public(_))
            && name.ends_with("Failure")
            && !item.attrs.iter().any(|attribute| {
                if !attribute.path().is_ident("must_use") {
                    return false;
                }
                let syn::Meta::NameValue(meta) = &attribute.meta else {
                    return false;
                };
                let Expr::Lit(value) = &meta.value else {
                    return false;
                };
                let syn::Lit::Str(reason) = &value.lit else {
                    return false;
                };
                !reason.value().trim().is_empty()
            })
        {
            self.0.push(name);
        }
        visit::visit_item_struct(self, item);
    }
}

#[test]
fn public_failure_wrappers_are_must_use() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    for root in ["kernel/src", "libs", "wost/src"] {
        for source in rust_sources_below(&paths.root().join(root)) {
            let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
            let text = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
            let syntax = syn::parse_file(&text)
                .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
            let mut failures = PublicFailures::default();
            failures.visit_file(&syntax);
            violations.extend(
                failures
                    .0
                    .into_iter()
                    .map(|name| format!("{}::{name}", relative.display())),
            );
        }
    }
    assert!(
        violations.is_empty(),
        "public failure wrappers missing #[must_use = \"reason\"]:\n{}",
        violations.join("\n")
    );
}

#[test]
fn production_invariants_are_not_debug_only() {
    let paths = Paths::discover().expect("workspace paths");
    let mut pending = ["kernel/src", "wost/src"]
        .map(|directory| paths.root().join(directory))
        .into_iter()
        .collect::<Vec<_>>();

    while let Some(path) = pending.pop() {
        for entry in crate::paths::directory_entries(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
        {
            let source = entry.path();
            if entry.is_dir() {
                pending.push(source);
            } else if source
                .extension()
                .is_some_and(|extension| extension == "rs")
            {
                let text = std::fs::read_to_string(&source)
                    .unwrap_or_else(|error| panic!("reading {}: {error}", source.display()));
                let syntax = syn::parse_file(&text)
                    .unwrap_or_else(|error| panic!("parsing {}: {error}", source.display()));
                let mut assertions = DebugAssertions::default();
                assertions.visit_file(&syntax);
                assert!(
                    assertions.found == 0,
                    "{} makes an invariant debug-only",
                    source.display()
                );
            }
        }
    }
}

#[test]
fn known_fallible_results_are_not_discarded() {
    let paths = Paths::discover().expect("workspace paths");
    let forbidden = [
        ("kernel/src/user.rs", "let _ = interrupt::answer"),
        ("kernel/src/nvme.rs", "let _ = self.fail_submitted"),
        ("kernel/src/main.rs", "let _ = vfs::enumerate"),
        (
            "kernel/src/main.rs",
            "let _ = wost::arch::current::port::claim",
        ),
        ("kernel/src/selftest.rs", "let _ = vfs::enumerate"),
        ("kernel/src/selftest.rs", "let _ = pci::enumerate"),
        ("kernel/src/selftest.rs", "let _ = task::set_thread_base"),
        (
            "kernel/src/selftest.rs",
            "let _ = user::release_gated_cross",
        ),
        ("kernel/src/selftest.rs", "let _ = mem::unmap_for_test"),
        ("kernel/src/selftest.rs", "let _ = runs_until"),
        ("kernel/src/selftest.rs", "let _ = yields_until"),
        ("kernel/src/arch/x86_64/i8042.rs", "let _ = self.flush"),
        (
            "kernel/src/thread.rs",
            "let _ = IDLE_STATE.compare_exchange",
        ),
        ("wost/src/boot.rs", "let _ = self.region.write_u32"),
        ("wost/src/boot.rs", "let _ = self.region.fill_u32"),
        ("wost/src/boot.rs", ".unwrap_or(0)"),
        ("kernel/src/user.rs", ".saturating_add(abandoned)"),
    ];

    for (relative, discarded) in forbidden {
        for path in super::module_sources(&paths.root().join(relative)) {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
            assert!(
                !source.contains(discarded),
                "{} silently discards {discarded:?}",
                path.display()
            );
        }
    }

    for source in rust_sources_below(&paths.root().join("kernel/src")) {
        let relative = source.strip_prefix(paths.root()).unwrap_or(&source);
        let text = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("reading {}: {error}", relative.display()));
        let discarded = discarded_address_space_reclaims(&text)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", relative.display()));
        assert!(
            discarded.is_empty(),
            "{} discards AddressSpace::reclaim on lines {discarded:?}",
            relative.display()
        );
    }
}

#[test]
fn fallible_operation_bools_stay_must_use() {
    let paths = Paths::discover().expect("workspace paths");
    let required = [
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused firmware read copied no bytes\"]\n    pub fn read",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused frame fill changed no bytes\"]\n    pub fn fill",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused frame clear changed no bytes\"]\n    pub fn zero",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused segment fill changed no bytes\"]\n    pub fn fill",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused segment clear changed no bytes\"]\n    pub fn zero",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused segment write changed no bytes\"]\n    pub fn write_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused segment read copied no bytes\"]\n    pub fn read_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused DMA-segment fill changed no bytes\"]\n    pub fn fill",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused DMA-segment clear changed no bytes\"]\n    pub fn zero",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused DMA-segment write changed no bytes\"]\n    pub fn write_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused DMA-segment read copied no bytes\"]\n    pub fn read_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused shared-memory write changed no bytes\"]\n    pub fn write_volatile_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused shared-memory read copied no bytes\"]\n    pub fn read_volatile_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused ring-payload write changed no bytes\"]\n    pub fn write_payload_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused ring-payload read copied no bytes\"]\n    pub fn read_payload_at",
        ),
        (
            "wost/src/mm.rs",
            "#[must_use = \"a refused shared-memory publication stored no index\"]\n    pub fn store_release_u32",
        ),
        (
            "wost/src/boot.rs",
            "#[must_use = \"a refused framebuffer shadow was not attached\"]\n    pub fn attach_back_buffer",
        ),
        (
            "kernel/src/console.rs",
            "#[must_use = \"a refused console shadow was not attached\"]\n    pub(crate) fn attach_back_buffer",
        ),
        (
            "wost/src/arch/x86_64/cpu.rs",
            "#[must_use = \"a refused PAT commit restores the previous entry on drop\"]\n    pub fn commit",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            "#[must_use = \"a refused resume changed no trap frame\"]\npub(super) fn resume_usercopy",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            "#[must_use = \"a refused resume changed no trap frame\"]\npub(super) fn resume_write_protection",
        ),
        (
            "wost/src/arch/x86_64/interrupt.rs",
            "#[must_use = \"a refused answer changed no trap frame\"]\npub fn answer",
        ),
        (
            "wost/src/arch/x86_64/pci.rs",
            "#[must_use = \"a refused PCI initialization left configuration space unavailable\"]\npub fn init",
        ),
        (
            "wost/src/arch/x86_64/segment.rs",
            "#[must_use = \"a refused double-fault stack install changed no TSS entry\"]\npub fn set_double_fault_stack",
        ),
        (
            "wost/src/arch/x86_64/usercopy.rs",
            "#[must_use = \"a refused user-copy recovery changed no trap frame\"]\npub fn recover",
        ),
        (
            "wost/src/arch/x86_64/task.rs",
            "#[must_use = \"a refused thread-base write changed no register\"]\npub fn set_thread_base",
        ),
        (
            "wost/src/mm/heap.rs",
            "#[must_use = \"a refused slot return changed no heap state\"]\n    unsafe fn give_slot",
        ),
        (
            "wost/src/mm/heap.rs",
            "#[must_use = \"a refused slab refill changed no heap state\"]\n    unsafe fn refill_with",
        ),
        (
            "kernel/src/handle.rs",
            "#[must_use = \"a refused quota transition changed no count\"]\n    fn step",
        ),
        (
            "kernel/src/timepage.rs",
            "#[must_use = \"a deferred publication must be observed\"]\npub(crate) enum Publication",
        ),
    ];

    for (relative, marker) in required {
        let source = std::fs::read_to_string(paths.root().join(relative))
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        assert!(
            source.contains(marker),
            "{relative} lets an operation refusal be discarded"
        );
    }
}

#[derive(Default)]
struct DebugAssertions {
    found: usize,
}

impl<'ast> Visit<'ast> for DebugAssertions {
    fn visit_macro(&mut self, item: &'ast Macro) {
        if item.path.segments.last().is_some_and(|segment| {
            segment.ident == "debug_assert"
                || segment.ident == "debug_assert_eq"
                || segment.ident == "debug_assert_ne"
        }) {
            self.found += 1;
        }
        visit::visit_macro(self, item);
    }
}
