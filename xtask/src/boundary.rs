//! Repository checks for target-side hostile-input boundaries.

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    const BOUNDARY_LIBS: &[&str] = &[
        "acpi-tables",
        "elf64",
        "fat32",
        "nvme",
        "path-walk",
        "pci-config",
        "ps2-keys",
        "ring-proto",
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
        for package in BOUNDARY_LIBS {
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
