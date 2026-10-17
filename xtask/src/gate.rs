//! The complete composition of the self-contained local gate.

use std::fmt::Write;
use std::path::Path;

use crate::cargo::Profile;
use crate::paths;
use anyhow::{Context, Result, ensure};

const README_BYTES: u64 = 64 * 1024;
const REGISTRY_START: &str = "<!-- CI-REGISTRY:START -->";
const REGISTRY_END: &str = "<!-- CI-REGISTRY:END -->";

/// One crate graph used by every Cargo-driven gate stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CargoPass {
    pub(crate) name: &'static str,
    pub(crate) packages: &'static [&'static str],
    pub(crate) target: Option<&'static str>,
    pub(crate) features: Option<&'static str>,
}

impl CargoPass {
    pub(crate) fn label(self) -> String {
        match (self.target, self.features) {
            (None, _) => self.name.to_string(),
            (Some(target), None) => format!("{} ({target})", self.name),
            (Some(target), Some(features)) => {
                format!("{} ({target} +{features})", self.name)
            }
        }
    }

    fn documented_target(self) -> &'static str {
        self.target.unwrap_or("host")
    }

    fn documented_features(self) -> &'static str {
        self.features.unwrap_or("none")
    }
}

/// One ordered stage of `cargo xtask ci`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Format,
    Clippy,
    ApiAudit,
    Documentation,
    HostTests,
    SupplyChain,
    Vendor,
    SelfTest(Profile),
    ShippedBoot,
    DegradedBoot,
}

impl Step {
    pub(crate) const fn invocation(self) -> &'static str {
        match self {
            Self::Format => "fmt --check",
            Self::Clippy => "clippy",
            Self::ApiAudit => "api-audit",
            Self::Documentation => "doc",
            Self::HostTests => "test",
            Self::SupplyChain => "supply-chain",
            Self::Vendor => "verify-vendor",
            Self::SelfTest(Profile::Release) => "test-boot --release",
            Self::SelfTest(Profile::Dev) => "test-boot --debug",
            Self::ShippedBoot => "boot-check --release",
            Self::DegradedBoot => "degraded-check --release",
        }
    }

    pub(crate) const fn evidence_key(self) -> &'static str {
        match self {
            Self::Format => "result.gate.fmt-check",
            Self::Clippy => "result.gate.clippy",
            Self::ApiAudit => "result.gate.api-audit",
            Self::Documentation => "result.gate.doc",
            Self::HostTests => "result.gate.test",
            Self::SupplyChain => "result.gate.supply-chain",
            Self::Vendor => "result.gate.verify-vendor",
            Self::SelfTest(Profile::Release) => "result.gate.test-boot-release",
            Self::SelfTest(Profile::Dev) => "result.gate.test-boot-debug",
            Self::ShippedBoot => "result.gate.boot-check-release",
            Self::DegradedBoot => "result.gate.degraded-check-release",
        }
    }

    const fn purpose(self) -> &'static str {
        match self {
            Self::Format => "refuse unformatted source",
            Self::Clippy => "lint every registered Cargo graph",
            Self::ApiAudit => "prove every public foundation API has a caller",
            Self::Documentation => "build docs for every registered Cargo graph",
            Self::HostTests => "run host tests and doctests",
            Self::SupplyChain => "bind locked bytes, licenses, scope and SPDX",
            Self::Vendor => "verify every vendored digest",
            Self::SelfTest(Profile::Release) => "run all machine checks on the release kernel",
            Self::SelfTest(Profile::Dev) => "run all machine checks with overflow checks enabled",
            Self::ShippedBoot => "boot, exercise and settle the shipped image",
            Self::DegradedBoot => "prove the no-PCI, no-x2APIC and no-IOMMU refusals",
        }
    }
}

/// The two dimensions every consumer renders from.
pub(crate) struct Registry {
    cargo_passes: &'static [CargoPass],
    steps: &'static [Step],
}

impl Registry {
    pub(crate) const fn cargo_passes(&self) -> &'static [CargoPass] {
        self.cargo_passes
    }

    pub(crate) const fn steps(&self) -> &'static [Step] {
        self.steps
    }

    pub(crate) fn verified_steps(&self, root: &Path) -> Result<&'static [Step]> {
        verify_documentation(root)?;
        Ok(self.steps)
    }

    #[cfg(test)]
    pub(crate) const fn steps_for_test(&self) -> &'static [Step] {
        self.steps
    }
}

const CARGO_PASSES: &[CargoPass] = &[
    CargoPass {
        name: "host",
        packages: paths::HOST_PACKAGES,
        target: None,
        features: None,
    },
    CargoPass {
        name: "kernel shipped",
        packages: paths::KERNEL_PACKAGES,
        target: Some(paths::KERNEL_TARGET),
        features: None,
    },
    CargoPass {
        name: "kernel self-test",
        packages: paths::KERNEL_PACKAGES,
        target: Some(paths::KERNEL_TARGET),
        features: Some(paths::SELFTEST_FEATURE_SPEC),
    },
    CargoPass {
        name: "kernel refuse-pci",
        packages: paths::KERNEL_PACKAGES,
        target: Some(paths::KERNEL_TARGET),
        features: Some(paths::REFUSE_PCI_FEATURE_SPEC),
    },
];

const STEPS: &[Step] = &[
    Step::Format,
    Step::Clippy,
    Step::ApiAudit,
    Step::Documentation,
    Step::HostTests,
    Step::SupplyChain,
    Step::Vendor,
    Step::SelfTest(Profile::Release),
    Step::SelfTest(Profile::Dev),
    Step::ShippedBoot,
    Step::DegradedBoot,
];

pub(crate) const REGISTRY: Registry = Registry {
    cargo_passes: CARGO_PASSES,
    steps: STEPS,
};

pub(crate) fn help_section() -> String {
    let mut rendered = String::from("CI REGISTRY (in execution order):\n    Cargo graphs:\n");
    for pass in REGISTRY.cargo_passes() {
        let _ = writeln!(rendered, "        {}", pass.label());
    }
    rendered.push_str("    Gate stages:\n");
    for step in REGISTRY.steps() {
        let _ = writeln!(
            rendered,
            "        {:<26} {}",
            step.invocation(),
            step.purpose()
        );
    }
    rendered
}

fn markdown() -> String {
    let mut rendered = String::from(
        "Cargo graphs used by each Cargo-driven gate stage:\n\n\
         | Graph | Target | Features |\n\
         |---|---|---|\n",
    );
    for pass in REGISTRY.cargo_passes() {
        let _ = writeln!(
            rendered,
            "| {} | `{}` | `{}` |",
            pass.name,
            pass.documented_target(),
            pass.documented_features()
        );
    }
    rendered.push_str(
        "\nOrdered `cargo xtask ci` stages:\n\n\
         | Order | Invocation | Purpose |\n\
         |---:|---|---|\n",
    );
    for (index, step) in REGISTRY.steps().iter().enumerate() {
        let _ = writeln!(
            rendered,
            "| {} | `cargo xtask {}` | {} |",
            index + 1,
            step.invocation(),
            step.purpose()
        );
    }
    rendered
}

pub(crate) fn verify_documentation(root: &Path) -> Result<()> {
    let readme_path = root.join("README.md");
    let readme = crate::qualification::read_bounded_text(&readme_path, README_BYTES, "CI README")?;
    let documented = readme
        .split_once(REGISTRY_START)
        .context("README has no CI registry start marker")?
        .1
        .split_once(REGISTRY_END)
        .context("README has no CI registry end marker")?
        .0
        .trim();
    ensure!(
        documented == markdown().trim(),
        "README's CI registry rendering differs from the executable registry"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{README_BYTES, REGISTRY_END, REGISTRY_START, markdown, verify_documentation};

    fn fixture(name: &str) -> PathBuf {
        crate::test_support::reserve_temp_directory(&format!("wizard-gate-{name}"))
    }

    fn valid_readme() -> String {
        format!(
            "before\n{REGISTRY_START}\n{}{REGISTRY_END}\nafter\n",
            markdown()
        )
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("create README link");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("create README link");
    }

    #[test]
    fn ci_readme_refuses_a_redirected_file() {
        let fixture = fixture("readme-link");
        let repository = fixture.join("repository");
        let sentinel = fixture.join("sentinel");
        std::fs::create_dir(&repository).expect("create repository fixture");
        std::fs::write(&sentinel, valid_readme()).expect("write external sentinel");
        let readme = repository.join("README.md");
        create_file_symlink(&sentinel, &readme);

        let error = verify_documentation(&repository)
            .expect_err("a redirected README was accepted")
            .to_string();

        assert!(error.contains("not a real file"), "{error}");
        std::fs::remove_file(readme).expect("remove README link");
        std::fs::remove_dir_all(fixture).expect("remove gate fixture");
    }

    #[test]
    fn ci_readme_refuses_oversize_before_parsing() {
        let fixture = fixture("readme-oversize");
        let readme = fixture.join("README.md");
        std::fs::File::create(&readme)
            .expect("create oversized README")
            .set_len(README_BYTES + 1)
            .expect("size oversized README");

        let error = verify_documentation(&fixture)
            .expect_err("an oversized README was accepted")
            .to_string();

        assert!(error.contains("exceeding the 65536-byte"), "{error}");
        std::fs::remove_dir_all(fixture).expect("remove gate fixture");
    }

    #[test]
    fn a_bounded_ci_readme_preserves_registry_semantics() {
        let fixture = fixture("readme-control");
        let readme = fixture.join("README.md");
        std::fs::write(&readme, valid_readme()).expect("write valid README");
        verify_documentation(&fixture).expect("accept valid registry document");

        std::fs::write(
            &readme,
            "<!-- CI-REGISTRY:START -->\nwrong\n<!-- CI-REGISTRY:END -->\n",
        )
        .expect("write mismatched README");
        let error = verify_documentation(&fixture)
            .expect_err("a mismatched registry was accepted")
            .to_string();
        assert!(error.contains("differs from the executable registry"));
        std::fs::remove_dir_all(fixture).expect("remove gate fixture");
    }
}
