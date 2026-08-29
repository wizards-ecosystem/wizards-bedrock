//! The complete composition of the self-contained local gate.

use std::fmt::Write;
use std::path::Path;

use crate::cargo::Profile;
use crate::paths;
use anyhow::{Context, Result, ensure};

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
    Documentation,
    HostTests,
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
            Self::Documentation => "doc",
            Self::HostTests => "test",
            Self::Vendor => "verify-vendor",
            Self::SelfTest(Profile::Release) => "test-boot --release",
            Self::SelfTest(Profile::Dev) => "test-boot --debug",
            Self::ShippedBoot => "boot-check --release",
            Self::DegradedBoot => "degraded-check --release",
        }
    }

    const fn purpose(self) -> &'static str {
        match self {
            Self::Format => "refuse unformatted source",
            Self::Clippy => "lint every registered Cargo graph",
            Self::Documentation => "build docs for every registered Cargo graph",
            Self::HostTests => "run host tests and doctests",
            Self::Vendor => "verify every vendored digest",
            Self::SelfTest(Profile::Release) => "run all machine checks on the release kernel",
            Self::SelfTest(Profile::Dev) => "run all machine checks with overflow checks enabled",
            Self::ShippedBoot => "boot, exercise and settle the shipped image",
            Self::DegradedBoot => "prove the no-PCI and no-x2APIC refusals",
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

    fn steps(&self) -> &'static [Step] {
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
    Step::Documentation,
    Step::HostTests,
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
    const START: &str = "<!-- CI-REGISTRY:START -->";
    const END: &str = "<!-- CI-REGISTRY:END -->";

    let readme = std::fs::read_to_string(root.join("README.md")).context("reading README.md")?;
    let documented = readme
        .split_once(START)
        .context("README has no CI registry start marker")?
        .1
        .split_once(END)
        .context("README has no CI registry end marker")?
        .0
        .trim();
    ensure!(
        documented == markdown().trim(),
        "README's CI registry rendering differs from the executable registry"
    );
    Ok(())
}
