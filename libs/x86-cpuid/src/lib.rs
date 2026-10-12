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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub const fn with_smep(self) -> Self {
        Self(self.0 | 1 << 20)
    }

    /// Add supervisor mode access prevention.
    pub const fn with_smap(self) -> Self {
        Self(self.0 | 1 << 21)
    }

    /// Add user mode instruction prevention.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionEnvironment {
    kind: ExecutionEnvironmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Whether CPU state binds the boot to a qualified virtual platform.
    ///
    /// Unlike ACPI bytes in guest RAM, the hypervisor base leaf cannot be
    /// rewritten by a PCI requester. The exact QEMU TCG and VMware profiles
    /// are the only release platforms permitted to assert the DMA-safe
    /// handoff contract.
    pub const fn authenticates_dma_handoff(self) -> bool {
        matches!(
            self.kind,
            ExecutionEnvironmentKind::QemuTcg | ExecutionEnvironmentKind::Vmware
        )
    }
}

/// Why a user page table may retain the kernel's mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedKernelMappingReason {
    kind: SharedKernelMappingReasonKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
mod tests {
    use super::*;

    const QEMU_TCG: ExecutionEnvironment = ExecutionEnvironment {
        kind: ExecutionEnvironmentKind::QemuTcg,
    };
    const VMWARE: ExecutionEnvironment = ExecutionEnvironment {
        kind: ExecutionEnvironmentKind::Vmware,
    };
    const OTHER: ExecutionEnvironment = ExecutionEnvironment {
        kind: ExecutionEnvironmentKind::Other,
    };
    const RDCL_NO_REASON: SharedKernelMappingReason = SharedKernelMappingReason {
        kind: SharedKernelMappingReasonKind::RdclNo,
    };
    const QEMU_TCG_REASON: SharedKernelMappingReason = SharedKernelMappingReason {
        kind: SharedKernelMappingReasonKind::QemuTcg,
    };

    const fn qemu_tcg_leaf() -> CpuidLeaf {
        CpuidLeaf::new(0x4000_0001, 0x5447_4354, 0x4354_4743, 0x4743_5447)
    }

    const fn vmware_leaf() -> CpuidLeaf {
        CpuidLeaf::new(0x4000_0010, 0x6177_4d56, 0x4d56_6572, 0x6572_6177)
    }

    #[test]
    fn cr4_updates_are_add_only_and_exact() {
        const FIRMWARE: u64 = (1 << 17) | (1 << 23) | 1;
        assert_eq!(
            Cr4Additions::extended_state().apply(FIRMWARE),
            FIRMWARE | (1 << 9) | (1 << 10) | (1 << 18)
        );

        for selection in 0..8 {
            let mut additions = Cr4Additions::none();
            if selection & 1 != 0 {
                additions = additions.with_smep();
            }
            if selection & 2 != 0 {
                additions = additions.with_smap();
            }
            if selection & 4 != 0 {
                additions = additions.with_umip();
            }
            let expected = FIRMWARE
                | if selection & 1 != 0 { 1 << 20 } else { 0 }
                | if selection & 2 != 0 { 1 << 21 } else { 0 }
                | if selection & 4 != 0 { 1 << 11 } else { 0 };
            assert_eq!(additions.apply(FIRMWARE), expected);
        }

        const HELD: u64 = (1 << 20) | (1 << 21) | (1 << 11);
        let repeated = Cr4Additions::none()
            .with_smep()
            .with_smep()
            .with_smap()
            .with_smap()
            .with_umip()
            .with_umip();
        assert_eq!(repeated.apply(0), HELD, "each door is idempotent");
        assert_eq!(
            repeated.apply(HELD),
            HELD,
            "applying never clears a held bit"
        );
    }

    #[test]
    fn frequency_leaves_keep_exact_and_nominal_evidence_distinct() {
        let hints = FrequencyHints::from_leaves(Some((3, 125, 24_000_000)), Some(0xfeed_0e10));
        assert_eq!(hints.tsc_hz(), Some(1_000_000_000));
        assert_eq!(hints.base_hz(), Some(3_600_000_000));

        for ratio in [
            None,
            Some((0, 125, 24_000_000)),
            Some((3, 0, 24_000_000)),
            Some((3, 125, 0)),
            Some((u32::MAX, 1, 1)),
        ] {
            assert_eq!(FrequencyHints::from_leaves(ratio, None).tsc_hz(), None);
        }
        assert_eq!(FrequencyHints::from_leaves(None, Some(0)).base_hz(), None);
        assert_eq!(
            FrequencyHints::from_leaves(Some((4, 2, 1)), None).tsc_hz(),
            Some(1),
            "the crystal ratio rounds to the nearest hertz"
        );
    }

    /// Saying nothing is not the same as saying safe.
    ///
    /// The distinction this type exists for. A processor with no capability
    /// word is every AMD part before Zen 2 and every Intel part before 2018 —
    /// treating its silence as `RDCL_NO` would skip a mitigation on exactly
    /// the machines that need one.
    #[test]
    fn silence_is_not_safety() {
        let silent = Speculation::undescribed();
        assert!(!silent.rdcl_no());
        assert!(!silent.enhanced_ibrs());
        assert!(!silent.mds_no());
        assert!(!silent.wants_boot_ibrs());

        let zero = Speculation::described(0);
        assert!(!zero.rdcl_no());
        assert_ne!(silent, zero, "no word and an all-clear word are different");
    }

    #[test]
    fn shared_kernel_mappings_need_rdcl_no_or_exact_qemu_tcg() {
        for speculation in [
            Speculation::undescribed(),
            Speculation::described(0),
            Speculation::described(0x44),
        ] {
            assert_eq!(
                shared_kernel_mapping_reason(&speculation, OTHER),
                None,
                "{speculation} was treated as affirmative RDCL evidence"
            );
        }

        for bits in [1, 0x2f, u64::MAX] {
            assert_eq!(
                shared_kernel_mapping_reason(&Speculation::described(bits), OTHER),
                Some(RDCL_NO_REASON)
            );
        }
        assert_eq!(
            shared_kernel_mapping_reason(&Speculation::undescribed(), QEMU_TCG),
            Some(QEMU_TCG_REASON)
        );
    }

    #[test]
    fn the_hypervisor_leaf_is_not_read_without_the_feature_bit() {
        for leaf_1_ecx in [0, 1, !HYPERVISOR_BIT] {
            assert_eq!(
                ExecutionEnvironment::read(leaf_1_ecx, || {
                    panic!("read the hypervisor leaf without its feature bit")
                }),
                OTHER
            );
        }
    }

    #[test]
    fn only_exact_qualified_hypervisor_leaves_authenticate_dma_handoff() {
        for (exact, expected) in [(qemu_tcg_leaf(), QEMU_TCG), (vmware_leaf(), VMWARE)] {
            for leaf_1_ecx in [HYPERVISOR_BIT, u32::MAX] {
                let observed = ExecutionEnvironment::read(leaf_1_ecx, || exact);
                assert_eq!(observed, expected);
                assert!(observed.authenticates_dma_handoff());
            }

            for register in 0..4 {
                for bit in 0..32 {
                    let mut words = [exact.eax, exact.ebx, exact.ecx, exact.edx];
                    words[register] ^= 1 << bit;
                    let near_miss = CpuidLeaf::new(words[0], words[1], words[2], words[3]);
                    let observed = ExecutionEnvironment::read(HYPERVISOR_BIT, || near_miss);
                    assert_eq!(observed, OTHER, "register {register} bit {bit} was ignored");
                    assert!(!observed.authenticates_dma_handoff());
                }
            }
        }
        assert!(!OTHER.authenticates_dma_handoff());
    }

    #[test]
    fn shared_mapping_reasons_have_stable_policy_names() {
        assert_eq!(RDCL_NO_REASON.to_string(), "rdcl_no");
        assert_eq!(QEMU_TCG_REASON.to_string(), "qemu_tcg");
    }

    /// Each capability bit is read on its own, and not from its neighbours.
    #[test]
    fn every_capability_bit_is_read_on_its_own() {
        /// One bit and the accessor that is supposed to read exactly it.
        type Reader = (u64, fn(&Speculation) -> bool);

        let readers: [Reader; 4] = [
            (1 << 0, Speculation::rdcl_no),
            (1 << 1, Speculation::enhanced_ibrs),
            (1 << 2, Speculation::rsba),
            (1 << 5, Speculation::mds_no),
        ];
        for (bit, read) in readers {
            assert!(
                read(&Speculation::described(bit)),
                "bit {bit:#x} did not read as set"
            );
            assert!(
                !read(&Speculation::described(!bit)),
                "bit {bit:#x} read as set from the others"
            );
        }
    }

    /// The MSR is not read unless the feature bit says it exists.
    ///
    /// Reading `IA32_ARCH_CAPABILITIES` on a processor that does not have it
    /// is a general-protection fault, so this is the one property here that is
    /// a crash rather than a wrong answer.
    #[test]
    fn the_capability_msr_is_not_read_when_absent() {
        let posture = Speculation::read(0, |_| panic!("read the msr without the feature bit"));
        assert_eq!(posture, Speculation::undescribed());

        let posture = Speculation::read(ARCH_CAPABILITIES_BIT, |msr| {
            assert_eq!(msr, ARCH_CAPABILITIES_MSR, "read the wrong msr");
            0b11
        });
        assert!(posture.rdcl_no() && posture.enhanced_ibrs());
    }

    /// Enabling IBRS preserves every other bit of `IA32_SPEC_CTRL`.
    ///
    /// Firmware may have enabled a mitigation of its own; clearing it would be
    /// the kernel silently weakening the machine it booted on.
    #[test]
    fn enabling_ibrs_preserves_what_firmware_set() {
        let enhanced = Speculation::described(1 << 1);
        assert!(enhanced.wants_boot_ibrs());
        assert_eq!(enhanced.spec_ctrl_after(0), SPEC_CTRL_IBRS);
        assert_eq!(
            enhanced.spec_ctrl_after(0b1100),
            0b1100 | SPEC_CTRL_IBRS,
            "an unrelated bit was cleared"
        );
        assert_eq!(
            enhanced.spec_ctrl_after(SPEC_CTRL_IBRS),
            SPEC_CTRL_IBRS,
            "already-set ibrs changed something"
        );
    }

    /// Without enhanced IBRS nothing is written, rather than the expensive
    /// alternative being chosen silently.
    #[test]
    fn without_enhanced_ibrs_nothing_is_written() {
        for posture in [
            Speculation::undescribed(),
            Speculation::described(0),
            Speculation::described(1 << 0),
        ] {
            assert!(!posture.wants_boot_ibrs());
            assert_eq!(posture.spec_ctrl_after(0b1010), 0b1010);
        }
    }

    /// The report names what it found, and says so plainly when it found
    /// nothing.
    #[test]
    fn the_report_names_what_it_found() {
        assert_eq!(
            format!("{}", Speculation::undescribed()),
            "describes no speculation capabilities"
        );
        assert_eq!(
            format!("{}", Speculation::described(0b10_0011)),
            "capabilities 0x23 rdcl_no ibrs_all mds_no"
        );
    }

    /// The four numbers the SDM fixes, transcribed.
    ///
    /// Every test above spells them by name on both sides of its comparison,
    /// so each reduces to an identity: `msr == ARCH_CAPABILITIES_MSR` where
    /// the value *is* that constant; `bit & bit != 0`, true of every single
    /// bit; `x | IBRS == x | IBRS`. All four survived being changed, and
    /// `SPEC_CTRL_MSR` was referenced by no test at all (D-124).
    #[test]
    fn the_speculation_constants_are_the_manual_s() {
        // SDM Vol. 4 table 2-2: IA32_SPEC_CTRL is MSR 0x48, and
        // IA32_ARCH_CAPABILITIES is 0x10A.
        assert_eq!(SPEC_CTRL_MSR, 0x48);
        assert_eq!(ARCH_CAPABILITIES_MSR, 0x10A);
        // Vol. 2A, CPUID leaf 7 sub-leaf 0: EDX bit 29 says the capability
        // MSR exists.
        assert_eq!(ARCH_CAPABILITIES_BIT, 0x2000_0000);
        assert_eq!(HYPERVISOR_BIT, 0x8000_0000);
        // IA32_SPEC_CTRL bit 0 is IBRS.
        assert_eq!(SPEC_CTRL_IBRS, 1);

        // And the bit is read from the position it names: a word with only
        // bit 29 set describes the capability, and one with bit 28 does not.
        assert_eq!(
            Speculation::read(0x2000_0000, |_| 0),
            Speculation::described(0)
        );
        assert_eq!(
            Speculation::read(0x1000_0000, |_| 0),
            Speculation::undescribed()
        );
    }
}
