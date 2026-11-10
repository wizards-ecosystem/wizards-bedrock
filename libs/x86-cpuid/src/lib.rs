//! What `cpuid` and the capability MSRs say, decoded.
//!
//! The pattern D-30 and D-43 set and `010` §8 asks for again: the arithmetic
//! that reads a hardware answer lives where `cargo test` can reach it, and the
//! privileged crate keeps only the instruction. QEMU's TCG does not emulate
//! `IA32_ARCH_CAPABILITIES`, so synthetic capability words are the only way
//! the enhanced-IBRS branch is checked before it meets real hardware.
//!
//! # Examples
//!
//! ```
//! use x86_cpuid::Speculation;
//!
//! // A machine that reports immunity to Meltdown and has enhanced IBRS.
//! let posture = Speculation::described(0b11);
//! assert!(posture.rdcl_no());
//! assert!(posture.enhanced_ibrs());
//!
//! // One that describes nothing is not the same as one that reports danger.
//! let silent = Speculation::undescribed();
//! assert!(!silent.rdcl_no());
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod physical;
pub mod xstate;

/// `cpuid` leaf 7 subleaf 0, `EDX` bit 29: `IA32_ARCH_CAPABILITIES` exists.
pub const ARCH_CAPABILITIES_BIT: u32 = 1 << 29;

/// `IA32_ARCH_CAPABILITIES`.
pub const ARCH_CAPABILITIES_MSR: u32 = 0x0000_010A;

/// `IA32_SPEC_CTRL`.
pub const SPEC_CTRL_MSR: u32 = 0x0000_0048;

/// `IA32_SPEC_CTRL.IBRS`.
pub const SPEC_CTRL_IBRS: u64 = 1 << 0;

/// A restricted, add-only update to the kernel-managed `CR4` feature bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cr4Additions(u64);

impl Cr4Additions {
    /// Enable the architectural state-save and SSE operating-system doors.
    pub const fn extended_state() -> Self {
        Self((1 << 9) | (1 << 10) | (1 << 18))
    }

    /// Begin an update with no optional protection enabled.
    pub const fn none() -> Self {
        Self(0)
    }

    /// Add supervisor mode execution prevention.
    #[must_use]
    pub const fn with_smep(self) -> Self {
        Self(self.0 | 1 << 20)
    }

    /// Add supervisor mode access prevention.
    #[must_use]
    pub const fn with_smap(self) -> Self {
        Self(self.0 | 1 << 21)
    }

    /// Add user mode instruction prevention.
    #[must_use]
    pub const fn with_umip(self) -> Self {
        Self(self.0 | 1 << 11)
    }

    /// Apply the update without clearing any register bit.
    pub const fn apply(self, current: u64) -> u64 {
        current | self.0
    }
}

/// `cpuid` leaf 1, `ECX` bit 31: execution is under a hypervisor.
const HYPERVISOR_BIT: u32 = 1 << 31;

/// One four-register `cpuid` answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CpuidLeaf {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

impl CpuidLeaf {
    /// Build a leaf from its architectural register order.
    pub const fn new(eax: u32, ebx: u32, ecx: u32, edx: u32) -> Self {
        Self { eax, ebx, ecx, edx }
    }
}

/// CPU-resident evidence about the execution environment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExecutionEnvironment {
    kind: ExecutionEnvironmentKind,
}

/// Requester-inaccessible CPU identity for a qualified DMA-handoff platform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmaHandoffPlatform {
    /// The exact pinned QEMU TCG hypervisor leaf.
    QemuTcg,
    /// The exact VMware Workstation hypervisor leaf.
    Vmware,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExecutionEnvironmentKind {
    QemuTcg,
    Vmware,
    Other,
}

impl ExecutionEnvironment {
    /// Read the hypervisor base leaf only when leaf 1 advertises it.
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

    /// Bind a qualified DMA-handoff profile to CPU-resident platform evidence.
    ///
    /// Unlike ACPI bytes in guest RAM, the hypervisor base leaf cannot be
    /// rewritten by a PCI requester. The exact QEMU TCG and VMware profiles
    /// are the only release platforms permitted to assert the DMA-safe
    /// handoff contract.
    pub const fn dma_handoff_platform(self) -> Option<DmaHandoffPlatform> {
        match self.kind {
            ExecutionEnvironmentKind::QemuTcg => Some(DmaHandoffPlatform::QemuTcg),
            ExecutionEnvironmentKind::Vmware => Some(DmaHandoffPlatform::Vmware),
            ExecutionEnvironmentKind::Other => None,
        }
    }
}

/// Why a user page table may retain the kernel's mappings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SharedKernelMappingReason {
    kind: SharedKernelMappingReasonKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SharedKernelMappingReasonKind {
    RdclNo,
    QemuTcg,
}

impl core::fmt::Display for SharedKernelMappingReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Write::write_str(
            f,
            match self.kind {
                SharedKernelMappingReasonKind::RdclNo => "rdcl_no",
                SharedKernelMappingReasonKind::QemuTcg => "qemu_tcg",
            },
        )
    }
}

/// Frequency information reported by the basic `cpuid` leaves.
///
/// Leaf 0x15 is an architectural ratio to a crystal and can name the TSC's
/// rate exactly enough to build a conversion. Leaf 0x16 is only the nominal
/// processor base frequency, so it remains separate diagnostic evidence and
/// is never silently promoted into a clock rate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrequencyHints {
    tsc_hz: Option<u64>,
    base_hz: Option<u64>,
}

impl FrequencyHints {
    /// Decode optional leaf 0x15 and leaf 0x16 answers.
    ///
    /// `ratio` is `(denominator, numerator, crystal_hz)`. Zero fields mean the
    /// processor did not provide a usable ratio. Only the low sixteen bits of
    /// `base_mhz` are defined by leaf 0x16.
    pub fn from_leaves(ratio: Option<(u32, u32, u32)>, base_mhz: Option<u32>) -> Self {
        let tsc_hz = match ratio {
            Some((denominator, numerator, crystal_hz))
                if denominator != 0 && numerator != 0 && crystal_hz != 0 =>
            {
                let product = (crystal_hz as u64).checked_mul(numerator as u64);
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
            Some(base_mhz as u64 * 1_000_000)
        };
        Self { tsc_hz, base_hz }
    }

    /// The TSC rate derived from leaf 0x15's crystal ratio, if complete.
    pub const fn tsc_hz(self) -> Option<u64> {
        self.tsc_hz
    }

    /// Leaf 0x16's nominal processor base frequency, for diagnostics only.
    pub const fn base_hz(self) -> Option<u64> {
        self.base_hz
    }
}

/// What a processor says about speculative-execution mitigation.
///
/// Absence and safety are deliberately different states. A processor with no
/// `IA32_ARCH_CAPABILITIES` is saying nothing, not saying it is safe, and
/// treating the two alike is how a mitigation gets skipped on the machines
/// that need it most.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Speculation {
    capabilities: Option<u64>,
}

impl Speculation {
    /// A processor that reports no capability word.
    pub const fn undescribed() -> Self {
        Self { capabilities: None }
    }

    /// A processor whose `IA32_ARCH_CAPABILITIES` reads `bits`.
    pub const fn described(bits: u64) -> Self {
        Self {
            capabilities: Some(bits),
        }
    }

    /// Decode from the raw `cpuid` answer and a reader for the MSR.
    ///
    /// The MSR is read only when the feature bit says it exists, which is the
    /// whole reason this is one function rather than two: reading it
    /// unconditionally is a `#GP` on every processor made before 2018.
    pub fn read(leaf_7_edx: u32, read_msr: impl FnOnce(u32) -> u64) -> Self {
        if leaf_7_edx & ARCH_CAPABILITIES_BIT == 0 {
            return Self::undescribed();
        }
        Self::described(read_msr(ARCH_CAPABILITIES_MSR))
    }

    /// `RDCL_NO` — the processor claims immunity to Meltdown.
    ///
    /// The fact D-81 rests on when it declines KPTI: a mitigation whose cost
    /// scales with syscall rate is not worth paying on a machine that says it
    /// does not need it.
    pub const fn rdcl_no(&self) -> bool {
        self.has(1 << 0)
    }

    /// `IBRS_ALL` — enhanced IBRS, so one write at boot lasts.
    ///
    /// Where it is present, branch-target isolation is a mode the hardware
    /// stays in rather than something the kernel re-asserts on every entry,
    /// which is why enabling it is a boot-time write and not an entry cost.
    pub const fn enhanced_ibrs(&self) -> bool {
        self.has(1 << 1)
    }

    /// `RSBA` — the processor may use an alternate return predictor.
    pub const fn rsba(&self) -> bool {
        self.has(1 << 2)
    }

    /// `MDS_NO` — the processor claims immunity to microarchitectural data
    /// sampling, so the `VERW` on the way back to user space buys nothing.
    pub const fn mds_no(&self) -> bool {
        self.has(1 << 5)
    }

    /// Whether the kernel should write `IBRS` into `IA32_SPEC_CTRL` at boot.
    ///
    /// Only with enhanced IBRS. Without it the alternative is per-entry IBRS,
    /// which is a real cost and a decision for a phase that can measure it —
    /// so the answer here is "no", not "do it the expensive way".
    pub const fn wants_boot_ibrs(&self) -> bool {
        self.enhanced_ibrs()
    }

    /// `IA32_SPEC_CTRL` as it should be left, given its current value.
    ///
    /// Every other bit is preserved: a mitigation firmware enabled is not the
    /// kernel's to clear.
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

/// Return the affirmative proof that permits shared kernel mappings.
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

impl core::fmt::Display for Speculation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let Some(bits) = self.capabilities else {
            return core::fmt::Write::write_str(f, "describes no speculation capabilities");
        };
        write!(f, "capabilities {bits:#x}")?;
        for (present, name) in [
            (self.rdcl_no(), "rdcl_no"),
            (self.enhanced_ibrs(), "ibrs_all"),
            (self.rsba(), "rsba"),
            (self.mds_no(), "mds_no"),
        ] {
            if present {
                write!(f, " {name}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
