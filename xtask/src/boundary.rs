//! Repository checks for target-side hostile-input boundaries.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    const AUDITED_FOUNDATION_LIBS: &[&str] = &[
        "acpi-tables",
        "eevdf",
        "elf64",
        "fat32",
        "frame-space",
        "handle-space",
        "memory-map",
        "mmio-range",
        "nvme",
        "panic-record",
        "path-walk",
        "pci-config",
        "pixel",
        "port-space",
        "ps2-keys",
        "ring-proto",
        "slab-layout",
        "stack-layout",
        "timebase",
        "trap-cause",
        "wizzy-handoff",
        "x86-apic",
        "x86-cpuid",
        "x86-paging",
        "x86-pit",
        "x86-tables",
    ];

    const PANIC_SHORTCUT_BOUNDARIES: &[&str] = &[
        "acpi-tables",
        "elf64",
        "fat32",
        "nvme",
        "panic-record",
        "path-walk",
        "pci-config",
        "ps2-keys",
        "ring-proto",
        "wizzy-handoff",
        "x86-apic",
    ];

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root")
            .to_path_buf()
    }

    fn compact(path: &Path) -> String {
        std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
            .split_whitespace()
            .collect()
    }

    fn documented_foundation_libraries(root: &Path) -> Vec<String> {
        let audit =
            std::fs::read_to_string(root.join("docs/017-pre-wizzy-hostile-surface-audit.md"))
                .expect("hostile-surface audit");
        let table = audit
            .split_once("## Host-testable validators")
            .expect("hostile-surface audit has no library section")
            .1
            .split_once("## Boot handoff and privileged adapters")
            .expect("hostile-surface audit library section has no end")
            .0;
        table
            .lines()
            .filter_map(|line| {
                line.strip_prefix("| `")
                    .and_then(|row| row.split_once("` |"))
                    .map(|(name, _)| name.to_string())
            })
            .collect()
    }

    #[test]
    fn every_foundation_library_has_an_explicit_boundary_policy() {
        let expected = crate::paths::HOST_PACKAGES
            .iter()
            .copied()
            .filter(|package| *package != "xtask")
            .collect::<Vec<_>>();
        assert_eq!(AUDITED_FOUNDATION_LIBS, expected);

        let root = root();
        assert_eq!(
            documented_foundation_libraries(&root),
            AUDITED_FOUNDATION_LIBS,
            "the hostile-surface audit does not account for every host library"
        );
        let boundary_rule = "deny(clippy::expect_used,clippy::panic,clippy::unwrap_used)";
        let shortcut_free = AUDITED_FOUNDATION_LIBS
            .iter()
            .copied()
            .filter(|package| {
                let path = root.join("libs").join(package).join("src/lib.rs");
                compact(&path).contains(boundary_rule)
            })
            .collect::<Vec<_>>();
        assert_eq!(PANIC_SHORTCUT_BOUNDARIES, shortcut_free);
    }

    /// User, firmware and device bytes never gain a convenient panic shortcut.
    #[test]
    fn target_boundaries_keep_panicking_shortcuts_out() {
        let root = root();
        let target_rule = "#![deny(clippy::expect_used,clippy::unwrap_used)]";
        let wost = root.join("wost/src/lib.rs");
        assert!(
            compact(&wost).contains(target_rule),
            "{} does not deny target-side expect/unwrap",
            wost.display()
        );
        let ports = root.join("wost/src/arch/x86_64/port.rs");
        assert!(
            !compact(&ports).contains("implPortSpan{"),
            "{} gives an uncalled dynamic lease a register API",
            ports.display()
        );
        let kernel = root.join("kernel/src/main.rs");
        let kernel_source = compact(&kernel);
        assert!(
            kernel_source.contains("cfg_attr(not(feature=\"selftest\")")
                && kernel_source.contains("deny(clippy::expect_used,clippy::unwrap_used)"),
            "{} does not deny production expect/unwrap",
            kernel.display()
        );

        let boundary_rule = "deny(clippy::expect_used,clippy::panic,clippy::unwrap_used)";
        for package in PANIC_SHORTCUT_BOUNDARIES {
            let path = root.join("libs").join(package).join("src/lib.rs");
            let source = compact(&path);
            assert!(
                source.contains("cfg_attr(not(test)") && source.contains(boundary_rule),
                "{} does not deny production panic shortcuts",
                path.display()
            );
        }
    }
}
