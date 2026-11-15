#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod physical;
pub mod xstate;

use core::fmt;

const ARCH_CAPABILITIES_BIT: u32 = 1 << 29;

const ARCH_CAPABILITIES_MSR: u32 = 0x0000_010A;

pub const SPEC_CTRL_MSR: u32 = 0x0000_0048;

const SPEC_CTRL_IBRS: u64 = 1 << 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cr4Additions(u64);

impl Cr4Additions {
    pub const fn extended_state() -> Self {
        Self((1 << 9) | (1 << 10) | (1 << 18))
    }

    pub const fn none() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn with_smep(self) -> Self {
        Self(self.0 | 1 << 20)
    }

    #[must_use]
    pub const fn with_smap(self) -> Self {
        Self(self.0 | 1 << 21)
    }

    #[must_use]
    pub const fn with_umip(self) -> Self {
        Self(self.0 | 1 << 11)
    }

    pub const fn apply(self, current: u64) -> u64 {
        current | self.0
    }
}

const HYPERVISOR_BIT: u32 = 1 << 31;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CpuidLeaf {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

impl CpuidLeaf {
    pub const fn new(eax: u32, ebx: u32, ecx: u32, edx: u32) -> Self {
        Self { eax, ebx, ecx, edx }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExecutionEnvironment {
    kind: ExecutionEnvironmentKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmaHandoffPlatform {
    QemuTcg,
    Vmware,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExecutionEnvironmentKind {
    QemuTcg,
    Vmware,
    Other,
}

impl ExecutionEnvironment {
    pub fn read(leaf_1_ecx: u32, read_hypervisor_base: impl FnOnce() -> CpuidLeaf) -> Self {
        if leaf_1_ecx & HYPERVISOR_BIT == 0 {
            return Self {
                kind: ExecutionEnvironmentKind::Other,
            };
        }

        let leaf = read_hypervisor_base();
        if leaf
            == CpuidLeaf::new(
                0x4000_0001,
                u32::from_le_bytes(*b"TCGT"),
                u32::from_le_bytes(*b"CGTC"),
                u32::from_le_bytes(*b"GTCG"),
            )
        {
            Self {
                kind: ExecutionEnvironmentKind::QemuTcg,
            }
        } else if leaf
            == CpuidLeaf::new(
                0x4000_0010,
                u32::from_le_bytes(*b"VMwa"),
                u32::from_le_bytes(*b"reVM"),
                u32::from_le_bytes(*b"ware"),
            )
        {
            Self {
                kind: ExecutionEnvironmentKind::Vmware,
            }
        } else {
            Self {
                kind: ExecutionEnvironmentKind::Other,
            }
        }
    }

    // A hypervisor leaf, unlike ACPI bytes in guest RAM, is out of every PCI requester's reach.
    pub const fn dma_handoff_platform(self) -> Option<DmaHandoffPlatform> {
        match self.kind {
            ExecutionEnvironmentKind::QemuTcg => Some(DmaHandoffPlatform::QemuTcg),
            ExecutionEnvironmentKind::Vmware => Some(DmaHandoffPlatform::Vmware),
            ExecutionEnvironmentKind::Other => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SharedKernelMappingReason {
    kind: SharedKernelMappingReasonKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SharedKernelMappingReasonKind {
    RdclNo,
    QemuTcg,
}

impl fmt::Display for SharedKernelMappingReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            SharedKernelMappingReasonKind::RdclNo => "rdcl_no",
            SharedKernelMappingReasonKind::QemuTcg => "qemu_tcg",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrequencyHints {
    tsc_hz: Option<u64>,
    base_hz: Option<u64>,
}

impl FrequencyHints {
    pub fn from_leaves(ratio: Option<(u32, u32, u32)>, base_mhz: Option<u32>) -> Self {
        let tsc_hz = match ratio {
            Some((denominator, numerator, crystal_hz))
                if denominator != 0 && numerator != 0 && crystal_hz != 0 =>
            {
                let product = u64::from(crystal_hz).checked_mul(u64::from(numerator));
                match product {
                    Some(product) => product
                        .checked_add(u64::from(denominator) / 2)
                        .map(|rounded| rounded / u64::from(denominator))
                        .filter(|hz| *hz != 0),
                    None => None,
                }
            }
            _ => None,
        };
        let base_mhz = base_mhz.unwrap_or(0) & u32::from(u16::MAX);
        let base_hz = if base_mhz == 0 {
            None
        } else {
            Some(u64::from(base_mhz) * 1_000_000)
        };
        Self { tsc_hz, base_hz }
    }

    pub const fn tsc_hz(self) -> Option<u64> {
        self.tsc_hz
    }

    // Leaf 0x16 is only a nominal base frequency, so it never becomes a clock rate.
    pub const fn base_hz(self) -> Option<u64> {
        self.base_hz
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Speculation {
    capabilities: Option<u64>,
}

impl Speculation {
    pub const fn undescribed() -> Self {
        Self { capabilities: None }
    }

    pub const fn described(bits: u64) -> Self {
        Self {
            capabilities: Some(bits),
        }
    }

    // The MSR read faults unless the feature bit says it exists.
    pub fn read(leaf_7_edx: u32, read_msr: impl FnOnce(u32) -> u64) -> Self {
        if leaf_7_edx & ARCH_CAPABILITIES_BIT == 0 {
            return Self::undescribed();
        }
        Self::described(read_msr(ARCH_CAPABILITIES_MSR))
    }

    const fn rdcl_no(&self) -> bool {
        self.has(1 << 0)
    }

    const fn enhanced_ibrs(&self) -> bool {
        self.has(1 << 1)
    }

    const fn rsba(&self) -> bool {
        self.has(1 << 2)
    }

    pub const fn mds_no(&self) -> bool {
        self.has(1 << 5)
    }

    pub const fn wants_boot_ibrs(&self) -> bool {
        self.enhanced_ibrs()
    }

    pub const fn spec_ctrl_after(&self, current: u64) -> u64 {
        if self.wants_boot_ibrs() {
            current | SPEC_CTRL_IBRS
        } else {
            current
        }
    }

    const fn has(&self, bit: u64) -> bool {
        matches!(self.capabilities, Some(bits) if bits & bit != 0)
    }
}

pub const fn shared_kernel_mapping_reason(
    speculation: &Speculation,
    execution: ExecutionEnvironment,
) -> Option<SharedKernelMappingReason> {
    if speculation.rdcl_no() {
        return Some(SharedKernelMappingReason {
            kind: SharedKernelMappingReasonKind::RdclNo,
        });
    }
    match execution.kind {
        ExecutionEnvironmentKind::QemuTcg => Some(SharedKernelMappingReason {
            kind: SharedKernelMappingReasonKind::QemuTcg,
        }),
        ExecutionEnvironmentKind::Vmware | ExecutionEnvironmentKind::Other => None,
    }
}

impl fmt::Display for Speculation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(bits) = self.capabilities else {
            return formatter.write_str("describes no speculation capabilities");
        };
        write!(formatter, "capabilities {bits:#x}")?;
        for (present, name) in [
            (self.rdcl_no(), "rdcl_no"),
            (self.enhanced_ibrs(), "ibrs_all"),
            (self.rsba(), "rsba"),
            (self.mds_no(), "mds_no"),
        ] {
            if present {
                write!(formatter, " {name}")?;
            }
        }
        Ok(())
    }
}
