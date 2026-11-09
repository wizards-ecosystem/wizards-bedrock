use crate::qemu;

use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    // Q35 leaves a bus-zero-only walk looking plausible, so the proof counts
    // buses rather than functions (D-168).
    Mutation {
        id: 212,
        file: "kernel/src/pci.rs",
        find: "                if entry.class.is_bridge()
                    && entry.header.is_bridge()",
        replace: "                if false
                    && entry.header.is_bridge()",
        breaks: "enumeration descends through bridges",
        reddens: Caught::Check("the bus walk descends through a bridge"),
    },
    // The ports a device decodes left free for anyone to claim, which is the
    // arrangement D-77 said proves the wrong thing.
    Mutation {
        id: 214,
        file: "kernel/src/pci.rs",
        find: "        let owned = wost::arch::current::port::claim_spans::<BARS>(&requested[..port_count])",
        replace: "        let owned = wost::arch::current::port::claim_spans::<BARS>(&requested[..0])",
        breaks: "a device claim covers the ports it decodes",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // Consume the span handles without returning their bits; the witness
    // reacquires the same BDF, so registry-only cleanup cannot pass.
    Mutation {
        id: 215,
        file: "kernel/src/pci.rs",
        find: "        if let Err(failure) = wost::arch::current::port::release_spans(ports) {
            self.ports = failure.spans;
            return ReleaseOutcome::refused(ReleaseError::Ports, self);
        }",
        replace: "        drop(ports);",
        breaks: "a transient pci lease returns every dynamic io span",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // Record the BDF before its fallible device/resource preparation. Any
    // later refusal then poisons a permanent registry slot.
    Mutation {
        id: 235,
        file: "kernel/src/pci.rs",
        find: "fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    if let Err(failure) = pci::release_function(function) {
        retain_failed_function(failure.into_function());
        return ClaimError::FunctionRetained;
    }
    reason
}",
        replace: "fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    drop(function);
    reason
}",
        breaks: "a refused device preparation leaves its registry unchanged",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    // Re-export the private closure that can hand any safe kernel module the
    // running `AddressSpace`, including its reclaim and arbitrary-unmap doors.
    Mutation {
        id: 301,
        file: "kernel/src/mem.rs",
        find: "fn with_active_space<T>(",
        replace: "pub(crate) fn with_active_space<T>(",
        breaks: "the running address space has no broad safe borrow",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::no_item_mem_exposes_names_the_running_address_space",
        },
    },
    Mutation {
        id: 316,
        file: "kernel/src/mem.rs",
        find: "    match space.reclaim(None) {",
        replace: "    match {\n        drop(space);\n        Ok::<usize, ReclaimFailure>(0)\n    } {",
        breaks: "a failed boot-space build reclaims its partially built tables",
        reddens: Caught::Check("a failed boot-space build returns every table frame"),
    },
    // D-390 makes PCI configuration mandatory before memory initialization,
    // so removing the unconditional survey is the reachable behavior change.
    Mutation {
        id: 324,
        file: "kernel/src/main.rs",
        find: "    survey_the_bus(dma_non_requester, dma_quiesced);\n    survey_the_filesystems();\n    start_interrupts(routing.as_ref());",
        replace: "    survey_the_bus(dma_non_requester, dma_quiesced);\n    start_interrupts(routing.as_ref());",
        breaks: "every admitted boot publishes its hardware-free namespaces",
        reddens: Caught::BootMarker(qemu::DEGRADED_FILESYSTEM_MARKER),
    },
    // Restore everything but stop enumeration anyway, reproducing the
    // first-candidate-only selector without conflating it with cleanup.
    Mutation {
        id: 332,
        file: "kernel/src/main.rs",
        find: "    info!(\"nvme: candidate {ordinal} retired cleanly; trying next\");
    StorageSelection::Seeking
}",
        replace: "    info!(\"nvme: candidate {ordinal} retired cleanly; trying next\");
    StorageSelection::Halted
}",
        breaks: "nvme selection advances after an exact candidate retirement",
        reddens: Caught::Check("a later nvme controller survives an earlier failure"),
    },
    Mutation {
        id: 565,
        file: "kernel/src/main.rs",
        find: "#[cfg(feature = \"refuse-pci\")]\nfn refuse_pci_for_degraded_boot() {\n    assert!(\n        wost::arch::current::port::pci_configuration().is_some(),\n        \"the degraded boot could not reserve the PCI configuration ports\"\n    );\n}",
        replace: "#[cfg(feature = \"refuse-pci\")]\nfn refuse_pci_for_degraded_boot() {}",
        breaks: "the degraded image silently retains live PCI configuration ports",
        reddens: Caught::BootMarker(qemu::NO_PCI_HANDOFF_PANIC),
    },
    Mutation {
        id: 567,
        file: "kernel/src/main.rs",
        find: "cpu: extended state unavailable; ring 3 stays closed",
        replace: "cpu: extended state unavailable",
        breaks: "the degraded CPU no longer proves the extended-state veto it exercised",
        reddens: Caught::BootMarker(qemu::DEGRADED_XSTATE_MARKER),
    },
    Mutation {
        id: 607,
        file: "kernel/src/main.rs",
        find: "        assert_eq!(\n            timepage::publish(mode, time::origin_ticks(), time::conversion()),",
        replace: "        debug_assert_eq!(\n            timepage::publish(mode, time::origin_ticks(), time::conversion()),",
        breaks: "release boot silently accepts refusal from an initialized time page",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 840,
        file: "kernel/src/pci.rs",
        find: "        Ok(()) => release_clean_function(function, reason),",
        replace: "        Ok(()) => {\n            drop(function);\n            reason\n        },",
        breaks: "a clean post-claim PCI refusal releases its function authority",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    Mutation {
        id: 928,
        file: "kernel/src/pci.rs",
        find: "fn claim_as(at: Address, lifetime: ClaimLifetime) -> Result<Device, ClaimError> {",
        replace: "fn claim_as(at: Address, lifetime: bool) -> Result<Device, ClaimError> {",
        breaks: "PCI claim lifetime becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 929,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {",
        replace: "fn start_interrupts(external_controllers: bool) {",
        breaks: "interrupt startup receives controller-mask proof as an unnamed boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 930,
        file: "kernel/src/main.rs",
        find: "const fn external_interrupts_are_admitted(mask: Option<&mem::ExternalControllersMasked>) -> bool {",
        replace: "const fn external_interrupts_are_admitted(mask: bool) -> bool {",
        breaks: "external-interrupt admission receives controller state as an unnamed boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 931,
        file: "kernel/src/main.rs",
        find: "fn route_keyboard(routing: Option<mem::ExternalControllersMasked>) {",
        replace: "fn route_keyboard(routing: Option<bool>) {",
        breaks: "keyboard routing receives unnamed boolean external-controller authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 939,
        file: "kernel/src/mem.rs",
        find: "#[derive(Debug)]\npub(crate) struct ExternalControllersMasked {",
        replace: "#[derive(Debug, Clone)]\npub(crate) struct ExternalControllersMasked {",
        breaks: "external-controller mask authority can be duplicated after one hardware proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 940,
        file: "kernel/src/mem.rs",
        find: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}",
        replace: "pub(crate) struct ExternalControllersMasked {\n    pub(crate) routing: Option<Routing>,\n}",
        breaks: "kernel peers can forge external-controller mask authority through a visible field",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 941,
        file: "kernel/src/mem.rs",
        find: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}\n\nstruct ControllersReleased {",
        replace: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}\n\nuse self::ExternalControllersMasked as ForgedMask;\n\nstruct ControllersReleased {",
        breaks: "external-controller mask authority gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 942,
        file: "kernel/src/mem.rs",
        find: "impl ExternalControllersMasked {\n    /// The selected keyboard controller, when the proven inventory has one.\n    pub(crate) fn into_routing(self) -> Option<Routing> {\n        self.routing\n    }\n}",
        replace: "impl ExternalControllersMasked {\n    /// The selected keyboard controller, when the proven inventory has one.\n    pub(crate) fn into_routing(self) -> Option<Routing> {\n        self.routing\n    }\n}\n\nfn forged_mask_authority() -> ExternalControllersMasked {\n    ExternalControllersMasked { routing: None }\n}",
        breaks: "external-controller mask authority gains a mint outside its checked transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 943,
        file: "kernel/src/mem.rs",
        find: "            controllers_masked: false,",
        replace: "            controllers_masked: true,",
        breaks: "an unproven external-controller inventory manufactures mask authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 944,
        file: "kernel/src/mem.rs",
        find: "struct ControllersReleased {\n    _private: (),\n}",
        replace: "#[derive(Clone)]\nstruct ControllersReleased {\n    _private: (),\n}",
        breaks: "one exact controller cleanup can duplicate its release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 945,
        file: "kernel/src/mem.rs",
        find: "impl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        replace: "impl RoutingPreparation {\n    const fn polled_without_release() -> Self {\n        Self {\n            routing: None,\n            controllers_masked: true,\n        }\n    }\n\n    fn ready(routing: Routing) -> Self {",
        breaks: "polled routing can manufacture mask authority without controller-release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 946,
        file: "kernel/src/mem.rs",
        find: "impl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        replace: "fn forged_routing_preparation() -> RoutingPreparation {\n    RoutingPreparation {\n        routing: None,\n        controllers_masked: true,\n    }\n}\n\nimpl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        breaks: "routing preparation gains a raw constructor outside its audited transitions",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 947,
        file: "kernel/src/mem.rs",
        find: "struct ControllersReleased {\n    _private: (),\n}\n\nimpl RoutingPreparation {",
        replace: "struct ControllersReleased {\n    _private: (),\n}\n\nfn forged_controller_release() -> ControllersReleased {\n    ControllersReleased { _private: () }\n}\n\nimpl RoutingPreparation {",
        breaks: "controller-release evidence gains a mint outside exact cleanup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 948,
        file: "kernel/src/mem.rs",
        find: "            retain_routing_registers(registers, core::mem::take(controllers));\n            return None;",
        replace: "            retain_routing_registers(registers, core::mem::take(controllers));\n            return Some(ControllersReleased { _private: () });",
        breaks: "failed controller cleanup still mints release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 949,
        file: "kernel/src/mem.rs",
        find: "            warn!(\"ioapic: candidate mapping remained owned during cleanup\");\n            retain_routing_registers(registers, core::mem::take(controllers));",
        replace: "            assert!(controllers.is_empty());\n            retain_routing_registers(registers, core::mem::take(controllers));",
        breaks: "routing cleanup can panic before retaining a refused device owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 950,
        file: "kernel/src/main.rs",
        find: "        Some(routing) if interrupt::are_enabled() => routing,",
        replace: "        Some(routing) if true => routing,",
        breaks: "keyboard routing proceeds without observing enabled maskable interrupts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 951,
        file: "kernel/src/main.rs",
        find: "        Some(routing) if interrupt::are_enabled() => routing,",
        replace: "        Some(routing) if { let _ = interrupt::are_enabled(); true } => routing,",
        breaks: "keyboard routing observes interrupt state but ignores its value",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 952,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "fn enable_before_mask_proof() {\n    interrupt::enable();\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    enable_before_mask_proof();\n    if let Err(reason) = pic::silence() {",
        breaks: "an interrupt-enable helper bypasses external-controller mask admission",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 953,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "fn retain_interrupt_enable() {\n    let _ = interrupt::enable;\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        breaks: "interrupt enable gains an indirect function reference outside its audited calls",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 954,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "use wost::arch::current::interrupt::enable as alias_enable;\n\nfn call_aliased_interrupt_enable() {\n    alias_enable();\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        breaks: "interrupt enable gains an imported alias outside mask admission",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 955,
        file: "kernel/src/main.rs",
        find: "    mask.is_some()",
        replace: "    mask.is_none()",
        breaks: "missing external-controller mask authority is admitted while real authority is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 1061,
        file: "kernel/src/mem.rs",
        find: "#[derive(Debug)]\npub(crate) struct ExternalControllersMasked {",
        replace: "#[derive(Debug, Default)]\npub(crate) struct ExternalControllersMasked {",
        breaks: "default construction forges external-controller mask authority without a hardware proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 1298,
        file: "kernel/src/main.rs",
        find: "        page_start < page_end && page_end <= 1_u64 << physical_bits,",
        replace: "        false && page_start < page_end && page_end <= 1_u64 << physical_bits,",
        breaks: "the live NVMe BAR witness withholds its admitted physical-width result",
        reddens: Caught::Check("a live nvme bar obeys maxphyaddr"),
    },
    Mutation {
        id: 1347,
        file: "kernel/src/main.rs",
        find: "        #[cfg(feature = \"selftest\")]\n        gsi,",
        replace: "        gsi,",
        breaks: "the routed-GSI evidence argument reaches the shipped call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1549,
        file: "kernel/src/main.rs",
        find: "        iommu::EstablishOutcome::Unavailable => warn!(\"storage: no DMA isolation\"),",
        replace: "        iommu::EstablishOutcome::Unavailable => warn!(\"storage: DMA isolation unavailable\"),",
        breaks: "the unisolated boot loses its exact refusal record",
        reddens: Caught::BootMarker(qemu::NO_ISOLATION_MARKER),
    },
    Mutation {
        id: 1570,
        file: "kernel/src/main.rs",
        find: "    *isolated = true;\n}",
        replace: "    *isolated = true;\n    *stage = admit_candidate(stage, &mut thread::ThreadContext::require());\n}",
        breaks: "the isolation stage runs the admit stage inside its own frame, so both debug frames share the boot stack",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::only_the_candidate_sequencer_calls_its_stages",
        },
    },
    Mutation {
        id: 1589,
        file: "kernel/src/iommu.rs",
        find: "    reserved.next().is_none()",
        replace: "    let _ = reserved.next();\n    true",
        breaks: "a storage candidate turns a firmware RMRR into DMA mapping authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::firmware_reserved_dma_regions_never_become_mapping_authority",
        },
    },
    Mutation {
        id: 1656,
        file: "kernel/src/main.rs",
        find: "            let retirement = controller.retire(&mut device, context);\n            iommu::report_faults();",
        replace: "            iommu::report_faults();\n            let retirement = controller.retire(&mut device, context);",
        breaks: "device-controlled VT-d diagnostics run before the failed controller reaches its DMA fence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::device_fault_reporting_is_bounded_and_follows_retirement",
        },
    },
    Mutation {
        id: 1665,
        file: "kernel/src/iommu.rs",
        find: "    for requester in requesters.iter() {",
        replace: "    for requester in requesters.iter().skip(1) {",
        breaks: "the first governed requester is omitted from the unit-wide DMA fence boundary",
        reddens: Caught::Check(
            "every shared vtd requester was fenced across controller fallback",
        ),
    },
    Mutation {
        id: 1674,
        file: "kernel/src/main.rs",
        find: "    if !iommu::release_candidate(context) {\n        warn!(\"nvme: candidate {ordinal} could not return its dma domain; selection halted\");\n        pci::retain_failed_release(device);\n        return StorageSelection::Halted;\n    }\n    if let Some((reason, device)) = device.release(context).into_retained() {",
        replace: "    if let Some((reason, device)) = device.release(context).into_retained() {",
        breaks: "the selected function authority is returned before the unit-wide VT-d disable transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1679,
        file: "kernel/src/main.rs",
        find: "fn isolate_candidate(stage: &mut CandidateStage) {\n    let outcome = match stage {",
        replace: "fn isolate_candidate(stage: &mut CandidateStage) {\n    #[cfg(debug_assertions)]\n    let stack_regression = [0u8; 4096];\n    #[cfg(not(debug_assertions))]\n    let stack_regression = [0u8; 16 * 1024];\n    core::hint::black_box(&stack_regression);\n    let outcome = match stage {",
        breaks: "unit isolation again consumes a large candidate-stack frame instead of borrowing its owners in a thin sibling stage",
        reddens: Caught::Check("nvme candidates preserve two pages of boot-stack headroom"),
    },
    Mutation {
        id: 1689,
        file: "kernel/src/iommu.rs",
        find: "    if !pci::topology_is_sealed() {\n        return Err(LookupError::UnsafeTopology);\n    }",
        replace: "    let _unsealed_topology = pci::topology_is_sealed();",
        breaks: "DMAR authority can be derived before the complete PCI topology is sealed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dmar_authority_uses_one_complete_sealed_pci_topology",
        },
    },
    Mutation {
        id: 1690,
        file: "kernel/src/pci.rs",
        find: "    TOPOLOGY.lock_leaf().reset();\n    for bus in 0..=u8::MAX {",
        replace: "    TOPOLOGY.lock_leaf().reset();\n    for bus in 0..u8::MAX {",
        breaks: "the sealed PCI snapshot omits every function on bus 255",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dmar_authority_uses_one_complete_sealed_pci_topology",
        },
    },
    Mutation {
        id: 1701,
        file: "kernel/src/pci.rs",
        find: "pub(crate) fn snapshot_topology(\n    mut non_requester: Option<QualifiedNonRequester>,\n    mut quiesced: Option<QuiescedIsaRequester>,\n) -> Result<(), TopologyError> {\n    TOPOLOGY.lock_leaf().reset();",
        replace: "pub(crate) fn snapshot_topology(\n    mut non_requester: Option<QualifiedNonRequester>,\n    mut quiesced: Option<QuiescedIsaRequester>,\n) -> Result<(), TopologyError> {\n    let stack_topology = PciTopology::new();\n    core::hint::black_box(&stack_topology);\n    TOPOLOGY.lock_leaf().reset();",
        breaks: "the complete PCI topology consumes a large fixed boot-stack frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dmar_authority_uses_one_complete_sealed_pci_topology",
        },
    },
    Mutation {
        id: 1719,
        file: "kernel/src/iommu.rs",
        find: "    let Ok(dmar) = Dmar::new(table) else {\n        return false;\n    };\n    if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {",
        replace: "    let Ok(dmar) = Dmar::new(table) else {\n        return false;\n    };\n    if false && (!dmar.interrupt_remapping() || dmar.x2apic_opt_out()) {",
        breaks: "kernel VT-d lookup ignores firmware's missing IR support or x2APIC opt-out",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1720,
        file: "kernel/src/iommu.rs",
        find: "    if !covers_every_present_requester(&requesters) {",
        replace: "    if false && !covers_every_present_requester(&requesters) {",
        breaks: "interrupt authority is accepted for a VT-d unit that misses live PCI requesters",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1721,
        file: "kernel/src/iommu.rs",
        find: "fn covers_every_present_requester(\n    requesters: &RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,\n) -> bool {\n    for bus in 0..=u8::MAX {",
        replace: "fn covers_every_present_requester(\n    requesters: &RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,\n) -> bool {\n    for bus in 0..u8::MAX {",
        breaks: "the interrupt boundary never checks live requesters on PCI bus 255",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1723,
        file: "kernel/src/main.rs",
        find: "    if !iommu::interrupt_boundary_active() {\n        warn!(\"iommu: complete PCI interrupt boundary missing; interrupts stay off\");\n        return;\n    }",
        replace: "    if false && !iommu::interrupt_boundary_active() {\n        warn!(\"iommu: complete PCI interrupt boundary missing; interrupts stay off\");\n        return;\n    }",
        breaks: "the CPU enables maskable interrupts while live PCI requesters lack interrupt isolation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1724,
        file: "kernel/src/main.rs",
        find: "    if !matches!(pci::topology_presence(), pci::TopologyPresence::Empty) {",
        replace: "    if false && !matches!(pci::topology_presence(), pci::TopologyPresence::Empty) {",
        breaks: "a PCI boot routes the legacy keyboard line outside the default-deny remapping table",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1731,
        file: "kernel/src/main.rs",
        find: "        if !rollback_keyboard_route(routing) {\n            warn!(\"keyboard: masked routing owner remains quarantined; staying polled\");\n            return;\n        }",
        replace: "        let _unchecked_masked_routing = routing;",
        breaks: "the PCI keyboard path reports polling without returning its masked IOAPIC owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1733,
        file: "kernel/src/pci.rs",
        find: "    if !topology_is_sealed() {\n        return TopologyPresence::Unsealed;\n    }",
        replace: "    if !topology_is_sealed() {\n        return TopologyPresence::Empty;\n    }",
        breaks: "a malformed unsealed PCI inventory is mistaken for a machine with no requesters",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1736,
        file: "kernel/src/iommu.rs",
        find: "    if dmar.sole_unit_segment() != Some(SEGMENT) {\n        return Err(LookupError::IncompleteInterruptBoundary);\n    }",
        replace: "    let _unchecked_remapping_units = dmar.sole_unit_segment();",
        breaks: "interrupt admission ignores a second or foreign-segment VT-d unit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1793,
        file: "kernel/src/main.rs",
        find: "    if !segment.zero() {",
        replace: "    if false {",
        breaks: "the startup read maps an uncleared allocator page for device DMA",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1800,
        file: "kernel/src/main.rs",
        find: "    Some(wost::mm::DmaSegment::from(segment))",
        replace: "    let segment = wost::mm::allocate_contiguous(1, 1)?;\n    Some(wost::mm::DmaSegment::from(segment))",
        breaks: "startup clears one allocation but grants DMA authority over a second uncleared one",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1804,
        file: "kernel/src/main.rs",
        find: "    let early_boundary = thread::ThreadContext::claim()\n        .is_some_and(|mut context| iommu::establish_fabric_boundary(&mut context));\n    if !early_boundary {\n        warn!(\"iommu: no early default-deny PCI boundary; storage and interrupts stay closed\");\n    }\n\n    match pci::snapshot_topology(dma_non_requester, dma_quiesced) {",
        replace: "    match pci::snapshot_topology(dma_non_requester, dma_quiesced) {\n        Ok(()) => info!(\"pci: complete segment-zero topology sealed\"),\n        Err(reason) => warn!(\"pci: DMA topology refused: {reason}\"),\n    }\n\n    let early_boundary = thread::ThreadContext::claim()\n        .is_some_and(|mut context| iommu::establish_fabric_boundary(&mut context));\n    if !early_boundary {\n        warn!(\"iommu: no early default-deny PCI boundary; storage and interrupts stay closed\");\n    }\n\n    match pci::snapshot_topology(dma_non_requester, dma_quiesced) {",
        breaks: "PCI topology is observed before the default-deny DMA owner exists",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1805,
        file: "kernel/src/iommu.rs",
        find: "        pci::TopologyPresence::Empty | pci::TopologyPresence::Present => {}",
        replace: "        pci::TopologyPresence::Empty => return true,\n        pci::TopologyPresence::Present => {}",
        breaks: "an empty PCI snapshot substitutes for a real installed VT-d owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1814,
        file: "kernel/src/iommu.rs",
        find: "        Establishment::Retained { reason, isolation } => {\n            warn!(\"iommu: the early default-deny takeover is uncertain: {reason}\");\n            install(InstalledUnit {\n                register_base,\n                isolation,\n                fences: UnitFences::empty(),\n                state: InstalledState::Quarantined,\n            });",
        replace: "        Establishment::Retained { reason, isolation } => {\n            warn!(\"iommu: the early default-deny takeover is uncertain: {reason}\");\n            install(InstalledUnit {\n                register_base,\n                isolation,\n                fences: UnitFences::empty(),\n                state: InstalledState::Ready,\n            });",
        breaks: "a failed post-enable takeover drain is recorded as a reusable early owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1815,
        file: "kernel/src/iommu.rs",
        find: "    if owner.state != InstalledState::Ready {\n        warn!(\"iommu: the early owner is quarantined and cannot be retargeted\");",
        replace: "    if false && owner.state != InstalledState::Ready {\n        warn!(\"iommu: the early owner is quarantined and cannot be retargeted\");",
        breaks: "a quarantined early owner can be revived by a later candidate retarget",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1816,
        file: "kernel/src/iommu.rs",
        find: "    (coverage.segment() == SEGMENT && coverage.register_base() == register_base)\n        .then_some(register_base)",
        replace: "    (coverage.segment() == SEGMENT).then_some(register_base)",
        breaks: "a platform coverage attestation can name a different physical remapping unit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1817,
        file: "kernel/src/iommu.rs",
        find: "acpi::find_unique_table(firmware, root, Sdt::DMA_COVERAGE)",
        replace: "acpi::find_table(firmware, root, Sdt::DMA_COVERAGE)",
        breaks: "the first of multiple platform coverage attestations can authorize takeover",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1818,
        file: "kernel/src/iommu.rs",
        find: "acpi::find_unique_table(firmware, root, Sdt::DMAR)",
        replace: "acpi::find_table(firmware, root, Sdt::DMAR)",
        breaks: "the first of multiple DMAR tables can corroborate early takeover authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1819,
        file: "kernel/src/acpi.rs",
        find: "                if require_unique {\n                    warn!(\"acpi: duplicate {signature:?} tables refuse unique authority\");\n                    return None;\n                }",
        replace: "                if false && require_unique {\n                    warn!(\"acpi: duplicate {signature:?} tables refuse unique authority\");\n                    return None;\n                }",
        breaks: "unique ACPI authority lookup silently returns the first duplicate table",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1827,
        file: "kernel/src/mem.rs",
        find: "    _dma: EarlyDmaBoundary,",
        replace: "    _dma: (),",
        breaks: "memory initialization no longer consumes the entry DMA proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1828,
        file: "kernel/src/iommu.rs",
        find: "#[derive(Debug)]\npub(crate) struct EarlyDmaBoundary {",
        replace: "#[derive(Debug, Clone)]\npub(crate) struct EarlyDmaBoundary {",
        breaks: "the one-shot entry DMA proof can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1829,
        file: "kernel/src/acpi.rs",
        find: "        if found.replace(table).is_some() {",
        replace: "        if false && found.replace(table).is_some() {",
        breaks: "the entry boundary accepts the first of multiple DMA handoff authorities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1834,
        file: "kernel/src/iommu.rs",
        find: "    let platform = cpu::dma_handoff_platform().ok_or(EarlyDmaError::UnauthenticatedPlatform)?;",
        replace: "    let platform = cpu::DmaHandoffPlatform::Vmware;",
        breaks: "writable ACPI bytes authenticate their own DMA handoff without CPU-resident platform evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1835,
        file: "kernel/src/acpi.rs",
        find: "            return Err(EarlyTableError::RootEntry);",
        replace: "            continue;",
        breaks: "an unreadable ACPI root target can hide a competing DMA handoff authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1836,
        file: "kernel/src/iommu.rs",
        find: "    compiler_fence(Ordering::SeqCst);",
        replace: "    let _ = Ordering::SeqCst;",
        breaks: "LTO may move privileged memory mutation above nomem entry-fence port instructions",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1840,
        file: "kernel/src/iommu.rs",
        find: "        cpu::DmaHandoffPlatform::Vmware => HandoffFencePolicy::vmware_440bx(),",
        replace: "        cpu::DmaHandoffPlatform::Vmware => HandoffFencePolicy::strict(),",
        breaks: "the authenticated VMware platform no longer selects its exact qualified non-requester profile",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1847,
        file: "kernel/src/pci.rs",
        find: "                    topology.observe_non_requester(\n                        authority,\n                        FunctionObservation::new(\n                            entry.at,\n                            entry.identity,\n                            entry.class_bits,\n                            entry.command,\n                            entry.class,\n                            entry.header,\n                            bus_numbers,\n                        ),\n                    )?;",
        replace: "                    let _ = authority;\n                    topology.observe(entry.at, entry.class, entry.header, bus_numbers)?;",
        breaks: "the exact VMware host bridge re-enters the later VT-d requester inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1852,
        file: "kernel/src/iommu.rs",
        find: "        .is_some_and(pci::topology_contains_requester)",
        replace: "        .is_some_and(pci::topology_contains_function)",
        breaks: "VT-d inventory repopulates a platform-qualified non-requester from mere PCI presence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1853,
        file: "kernel/src/iommu.rs",
        find: "                if !pci::topology_contains_requester(at) {",
        replace: "                if !pci::topology_contains_function(at) {",
        breaks: "whole-unit coverage treats the qualified VMware host bridge as a DMA requester again",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1859,
        file: "kernel/src/main.rs",
        find: "    let dma_non_requester = early_dma.take_non_requester();",
        replace: "    let dma_non_requester = None;",
        breaks: "the exact entry-fence authority is discarded before the later topology snapshot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1860,
        file: "kernel/src/pci.rs",
        find: "    if let Some(authority) = non_requester {",
        replace: "    if let Some(authority) = Option::<QualifiedNonRequester>::None {",
        breaks: "an entry-qualified non-requester may disappear before the sealed snapshot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2284,
        file: "kernel/src/pci.rs",
        find: "    if let Some(record) = quiesced {\n        return fence_quiesced_requester(function, record);\n    }",
        replace: "    let _ = quiesced;",
        breaks: "a unit-wide fence tries to clear the quiesced requester's read-only BME",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2285,
        file: "kernel/src/pci.rs",
        find: "    if identity == Some(record.identity())",
        replace: "    if identity.is_some()",
        breaks: "the unit-wide fence holds a different function at the quiesced address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2286,
        file: "kernel/src/pci.rs",
        find: "        && command == Some(record.command())",
        replace: "        && command.is_some()",
        breaks: "the unit-wide fence holds the quiesced requester after its command changed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2287,
        file: "kernel/src/pci.rs",
        find: "        && pci::isa_dma_quiet()\n",
        replace: "        && true\n",
        breaks: "the unit-wide fence holds the quiesced requester without rechecking its 8237s",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2295,
        file: "kernel/src/main.rs",
        find: "at its masked 8237 controllers; it remains a requester\"",
        replace: "at its masked 8237 controllers\"",
        breaks: "the kernel's ISA quiesce record drifts from the VMware verifier marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::the_vmware_isa_dma_marker_is_required_by_both_selectors",
        },
    },
    Mutation {
        id: 2462,
        file: "kernel/src/pci.rs",
        find: "if self.claimed[usize::from(bus)] || self.len == MAX_BUSES {",
        replace: "if self.len == MAX_BUSES {",
        breaks: "a repeated bridge secondary takes a worklist slot again",
        reddens: Caught::Check("a repeated bridge secondary cannot hide a later bus"),
    },
];
