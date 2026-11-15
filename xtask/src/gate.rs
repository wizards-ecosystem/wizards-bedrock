//! The complete composition of the self-contained local gate.

use std::fmt::Write;

use crate::cargo::Profile;
use crate::paths;

/// The crates a pass selects: the workspace less the kernel crates, or a fixed set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Packages {
    Host,
    Only(&'static [&'static str]),
}

/// One crate graph used by every Cargo-driven gate stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CargoPass {
    pub(crate) name: &'static str,
    pub(crate) packages: Packages,
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
}

/// One ordered stage of `cargo xtask ci`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    Format,
    Clippy,
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
#[derive(Debug)]
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
}

const CARGO_PASSES: &[CargoPass] = &[
    CargoPass {
        name: "host",
        packages: Packages::Host,
        target: None,
        features: None,
    },
    CargoPass {
        name: "kernel shipped",
        packages: Packages::Only(paths::KERNEL_PACKAGES),
        target: Some(paths::KERNEL_TARGET),
        features: None,
    },
    CargoPass {
        name: "kernel self-test",
        packages: Packages::Only(paths::KERNEL_PACKAGES),
        target: Some(paths::KERNEL_TARGET),
        features: Some(paths::SELFTEST_FEATURE_SPEC),
    },
    CargoPass {
        name: "kernel refuse-pci",
        packages: Packages::Only(paths::KERNEL_PACKAGES),
        target: Some(paths::KERNEL_TARGET),
        features: Some(paths::REFUSE_PCI_FEATURE_SPEC),
    },
];

const STEPS: &[Step] = &[
    Step::Format,
    Step::Clippy,
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
