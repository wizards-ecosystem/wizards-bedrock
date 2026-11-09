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
    for (exact, expected, platform) in [
        (qemu_tcg_leaf(), QEMU_TCG, DmaHandoffPlatform::QemuTcg),
        (vmware_leaf(), VMWARE, DmaHandoffPlatform::Vmware),
    ] {
        for leaf_1_ecx in [HYPERVISOR_BIT, u32::MAX] {
            let observed = ExecutionEnvironment::read(leaf_1_ecx, || exact);
            assert_eq!(observed, expected);
            assert_eq!(observed.dma_handoff_platform(), Some(platform));
        }

        for register in 0..4 {
            for bit in 0..32 {
                let mut words = [exact.eax, exact.ebx, exact.ecx, exact.edx];
                words[register] ^= 1 << bit;
                let near_miss = CpuidLeaf::new(words[0], words[1], words[2], words[3]);
                let observed = ExecutionEnvironment::read(HYPERVISOR_BIT, || near_miss);
                assert_eq!(observed, OTHER, "register {register} bit {bit} was ignored");
                assert_eq!(observed.dma_handoff_platform(), None);
            }
        }
    }
    assert_eq!(OTHER.dma_handoff_platform(), None);
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
