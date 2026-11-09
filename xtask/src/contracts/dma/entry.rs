use syn::{Fields, Item, Type, Visibility, visit::Visit};

use crate::contracts::source_tree;
use crate::test_support::{function_after, source_through};

use super::{
    CallNames, ReferencedPaths, authority_alias_count, call_position, exact_derives, function,
    method, parse, root, source,
};

#[test]
fn memory_initialization_requires_the_exhaustive_entry_dma_boundary() {
    let main = source("kernel/src/main.rs");
    let boot = main
        .split_once("fn kernel_main(boot: BootInfo) -> ! {")
        .expect("kernel entry")
        .1
        .split_once("struct Handoff")
        .expect("kernel entry end")
        .0;
    let proof = boot
        .find("let early_dma = iommu::secure_handoff(&boot);")
        .expect("entry DMA proof");
    let console = boot
        .find("let serial = dev::init_uart();")
        .expect("UART setup");
    let memory = boot
        .find("mem::init(boot.memory_map, boot.direct_map, early_dma);")
        .expect("proof-consuming memory initialization");
    assert!(proof < console && console < memory);

    let iommu = source("kernel/src/iommu.rs");
    let boundary = iommu
        .split_once("pub(crate) fn secure_handoff")
        .expect("secure handoff")
        .1
        .split_once("static DMAR")
        .expect("secure handoff end")
        .0;
    let platform = boundary
        .find("cpu::dma_handoff_platform().ok_or(EarlyDmaError::UnauthenticatedPlatform)?")
        .expect("requester-inaccessible platform evidence");
    assert!(boundary.contains("cpu::DmaHandoffPlatform::QemuTcg => HandoffFencePolicy::strict()"));
    assert!(
        boundary.contains("cpu::DmaHandoffPlatform::Vmware => HandoffFencePolicy::vmware_440bx()")
    );
    let table = boundary
        .find("acpi::find_unique_early")
        .expect("unique early handoff table");
    let ports = boundary.find("platform_pci::init()").expect("port claim");
    let fence = boundary
        .find("platform_pci::fence_dma_handoff(policy)")
        .expect("exhaustive requester fence");
    let ordering = boundary
        .find("compiler_fence(Ordering::SeqCst)")
        .expect("compiler ordering after requester fence");
    assert!(platform < table && table < ports && ports < fence && fence < ordering);
    let iommu_syntax = parse("kernel/src/iommu.rs");
    let token = iommu_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "EarlyDmaBoundary" => Some(item),
            _ => None,
        })
        .expect("entry DMA proof token");
    assert!(matches!(token.vis, Visibility::Restricted(_)));
    assert!(exact_derives(&token.attrs, &["Debug"]));
    assert!(!iommu.contains("impl Clone for EarlyDmaBoundary"));
    assert!(!iommu.contains("impl Copy for EarlyDmaBoundary"));

    let acpi = source("kernel/src/acpi.rs");
    let unique = acpi
        .split_once("pub(crate) fn find_unique_early")
        .expect("allocation-free unique table lookup")
        .1
        .split_once("pub(crate) type Interrupts")
        .expect("allocation-free unique table lookup end")
        .0;
    assert!(unique.contains("let mut root_table = [0u8; EARLY_ROOT_BYTES];"));
    assert!(unique.contains("return Err(EarlyTableError::RootEntry);"));
    assert!(unique.contains("if found.replace(table).is_some()"));
    assert!(unique.contains("return Err(EarlyTableError::Duplicate);"));

    let memory = source("kernel/src/mem.rs");
    let init = memory
        .split_once("pub(crate) fn init(")
        .expect("memory initialization")
        .1
        .split_once(") {")
        .expect("memory initialization signature")
        .0;
    assert!(init.contains("_dma: EarlyDmaBoundary"));

    let pci = source("libs/pci-config/src/fence.rs");
    let pci_crate = source_tree(&root().join("libs/pci-config/src"));
    let fence = function_after(&pci, "pub fn fence_dma_handoff(").expect("pure entry fence");
    assert!(fence.contains("for bus in 0..=u8::MAX"));
    assert!(fence.contains("for device in 0..DEVICES"));
    assert!(fence.contains("for function in 0..FUNCTIONS"));
    assert_eq!(fence.matches("HandoffAccess::ReadIdentity").count(), 4);
    assert_eq!(fence.matches("HandoffAccess::ReadClass").count(), 4);
    assert!(fence.contains("let expected = before & !command::BUS_MASTER;"));
    assert!(fence.contains("if found != expected"));
    assert!(fence.contains(".classify_non_requester(at, identity, class, before, found)"));
    assert!(fence.contains("if current != identity"));
    assert!(fence.contains("if current_class != class"));

    let profile = function_after(&pci, "fn classify_non_requester(")
        .expect("qualified non-requester profile");
    for exact in [
        "at.bus() == 0",
        "at.device() == 0",
        "at.function() == 0",
        "identity == VMWARE_440BX_IDENTITY",
        "class & CLASS_WITHOUT_REVISION == HOST_BRIDGE_CLASS",
        "before == VMWARE_HOST_COMMAND",
        "found == VMWARE_HOST_COMMAND",
    ] {
        assert!(profile.contains(exact), "missing `{exact}`");
    }
    assert!(
        !pci_crate.contains("pub fn classify_non_requester("),
        "only the exhaustive entry fence may mint non-requester authority"
    );
    assert!(pci.contains("pub fn take_non_requester(&mut self)"));
    let pci_syntax = parse("libs/pci-config/src/lib.rs");
    let non_requester = pci_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "QualifiedNonRequester" => Some(item),
            _ => None,
        })
        .expect("qualified non-requester authority");
    assert!(matches!(non_requester.vis, Visibility::Public(_)));
    assert!(exact_derives(&non_requester.attrs, &["Debug"]));
    assert!(
        non_requester
            .fields
            .iter()
            .all(|field| matches!(field.vis, Visibility::Inherited))
    );
    assert!(!pci_crate.contains("impl Clone for QualifiedNonRequester"));
    assert!(!pci_crate.contains("impl Copy for QualifiedNonRequester"));

    let kernel_pci = source("kernel/src/pci.rs");
    let snapshot = function_after(
        &kernel_pci,
        "pub(crate) fn snapshot_topology(\n    mut non_requester: Option<QualifiedNonRequester>,\n    mut quiesced: Option<QuiescedIsaRequester>,\n)",
    )
    .expect("entry-proof-bound topology snapshot");
    assert!(!snapshot.contains("classify_non_requester("));
    assert!(snapshot.contains("authority.address() == entry.at"));
    assert!(snapshot.contains("topology.observe_non_requester("));
    assert!(snapshot.contains("quiesced.take_if(|authority| authority.address() == entry.at)"));
    assert!(snapshot.contains("topology.observe_quiesced_requester("));
    assert!(snapshot.contains("if let Some(authority) = quiesced {"));
    assert!(snapshot.contains("TopologyError::QuiescedRequesterMissing"));

    // D-426: the entry-quiesced ISA requester stays in every inventory, so a
    // unit-wide fence must hold it by exact identity, command and a fresh
    // 8237 remask rather than by a BME clear it cannot perform.
    let requester_fence = source_through(
        &kernel_pci,
        "pub(crate) fn fence_dma_requester(at: Address) -> DmaFenceOutcome {",
        "fn fence_quiesced_requester(",
    )
    .expect("sibling requester fence");
    let quiesced_lookup = requester_fence
        .find("TOPOLOGY.lock_leaf().quiesced_requester(at)")
        .expect("quiesced requester lookup");
    let bus_master = requester_fence
        .find("pci::fence_bus_master(&function)")
        .expect("ordinary BME fence");
    assert!(quiesced_lookup < bus_master);
    for exact in [
        "return fence_quiesced_requester(function, record);",
        "identity == Some(record.identity())",
        "command == Some(record.command())",
        "pci::isa_dma_quiet()",
        "DmaFenceOutcome::Refused(DmaFenceError::Quiesce)",
    ] {
        assert!(requester_fence.contains(exact), "missing `{exact}`");
    }
    let isa = pci
        .split_once("fn classify_isa_requester(")
        .expect("quiesced ISA requester profile")
        .1
        .split_once("impl QualifiedNonRequester")
        .expect("quiesced ISA requester profile end")
        .0;
    for exact in [
        "self.quiesces_isa_dma()",
        "(at.bus(), at.device(), at.function()) == (0, 7, 0)",
        "identity == VMWARE_PIIX4_ISA_IDENTITY",
        "class & ISA_CLASS_MASK == ISA_BRIDGE_CLASS",
        "before == VMWARE_ISA_COMMAND",
        "found == VMWARE_ISA_COMMAND",
    ] {
        assert!(isa.contains(exact), "missing `{exact}`");
    }
    assert!(fence.contains(".classify_isa_requester(at, identity, class, before, found)"));
    assert!(fence.contains("status & ISA_DMA_PENDING_REQUESTS != 0"));
    let quiesced_authority = pci_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "QuiescedIsaRequester" => Some(item),
            _ => None,
        })
        .expect("quiesced ISA requester authority");
    assert!(exact_derives(&quiesced_authority.attrs, &["Debug"]));
    assert!(
        quiesced_authority
            .fields
            .iter()
            .all(|field| matches!(field.vis, Visibility::Inherited))
    );
    let wost_pci = source("wost/src/arch/x86_64/pci.rs");
    assert!(
        wost_pci.contains("HandoffAccess::QuiesceIsaDma => isa_dma.as_mut().map(IsaDma::quiesce),")
    );
    let claim = wost_pci
        .find("port::isa_dma_controllers()")
        .expect("8237 claim");
    let lock = wost_pci
        .find("let mut configuration = CONFIGURATION.lock_leaf();\n        if configuration.functions.len() != 0 {")
        .expect("fence configuration lock");
    assert!(
        claim < lock,
        "the 8237 claim precedes the configuration guard"
    );
    assert!(snapshot.contains("if let Some(authority) = non_requester {"));
    assert!(snapshot.contains("TopologyError::QualifiedNonRequesterMissing"));
    assert!(
        kernel_pci.contains(
            "topology_contains_function(at) && TOPOLOGY.lock_leaf().contains_requester(at)"
        )
    );

    let inventory = function(&iommu_syntax, "requester_is_present");
    let mut inventory_paths = ReferencedPaths::default();
    inventory_paths.visit_item_fn(inventory);
    assert!(
        inventory_paths
            .names
            .iter()
            .any(|name| name == "topology_contains_requester")
    );
    assert!(
        inventory_paths
            .names
            .iter()
            .all(|name| name != "topology_contains_function")
    );

    let coverage = function(&iommu_syntax, "covers_every_present_requester");
    let mut coverage_paths = ReferencedPaths::default();
    coverage_paths.visit_item_fn(coverage);
    assert!(
        coverage_paths
            .names
            .iter()
            .any(|name| name == "topology_contains_requester")
    );
    assert!(
        coverage_paths
            .names
            .iter()
            .all(|name| name != "topology_contains_function")
    );

    let main = source("kernel/src/main.rs");
    assert!(main.contains("let dma_non_requester = early_dma.take_non_requester();"));
    assert!(main.contains("let dma_quiesced = early_dma.take_quiesced_requester();"));
    assert!(main.contains("survey_the_bus(dma_non_requester, dma_quiesced);"));
    assert!(main.contains("pci::snapshot_topology(dma_non_requester, dma_quiesced)"));
}

#[test]
fn pci_interrupts_are_default_deny_before_bus_mastering_or_sti() {
    let vtd_source = source("wost/src/arch/x86_64/vtd.rs");
    let vtd = parse("wost/src/arch/x86_64/vtd.rs");
    let proof = vtd
        .items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == "InterruptIsolation" => Some(item),
            _ => None,
        })
        .expect("missing interrupt-isolation proof");
    assert!(matches!(proof.vis, Visibility::Public(_)));
    assert!(exact_derives(&proof.attrs, &["Debug"]));
    let Fields::Named(fields) = &proof.fields else {
        panic!("InterruptIsolation must name its exact function");
    };
    assert_eq!(fields.named.len(), 2);
    let function = fields.named.first().expect("interrupt function field");
    assert!(matches!(function.vis, Visibility::Inherited));
    assert!(matches!(&function.ty, Type::Path(path)
        if path.path.segments.last().is_some_and(|segment| segment.ident == "Address")));
    assert_eq!(authority_alias_count(&vtd, &["InterruptIsolation"]), 0);

    for requirement in [
        "if !platform_interrupt_remapping {",
        "if !extended.interrupt_remapping() || !extended.queued_invalidation() {",
        "if !extended.extended_interrupt_mode() {",
        "VtdError::FirmwareFacilitiesActive { status }",
        "if !translation_enabled(status)",
        "|| !interrupt_remapping_enabled(status)",
        "|| compatibility_interrupts_enabled(status)",
        "|| status & gsts::QIES != 0",
        "|| !fault_event_control::masked(fault_control)",
        "|| root_table_address(pointer) != self.root.address()",
        "observed != expected",
        "|| !interrupt_table_has_two_entries(observed)",
        "if self.interrupts.read_u64(index) != Some(0) {",
        "if observed == pointer",
    ] {
        assert!(
            vtd_source.contains(requirement),
            "VT-d interrupt boundary lost `{requirement}`"
        );
    }

    let mut transition = CallNames::default();
    transition.visit_impl_item_fn(method(&vtd, "Vtd", "transition").expect("VT-d transition"));
    let fault_mask = call_position(&transition, "mask_fault_interrupt");
    let root = call_position(&transition, "set_root_table");
    let interrupt_table = call_position(&transition, "set_interrupt_table");
    let interrupt_cache = call_position(&transition, "invalidate_interrupt_cache");
    let interrupt_enable = call_position(&transition, "switch_interrupt_remapping");
    let translation_enable = call_position(&transition, "switch_translation");
    let drain = call_position(&transition, "drain_dma");
    assert!(
        fault_mask < root
            && root < interrupt_table
            && interrupt_table < interrupt_cache
            && interrupt_cache < interrupt_enable
            && interrupt_enable < translation_enable
            && translation_enable < drain
    );
    assert_eq!(
        vtd_source.matches("if !takeover_ready(status) {").count(),
        2,
        "read-only takeover preflight must be repeated after table allocation and before the first unit write"
    );
    let establishment = function_after(&vtd_source, "    pub fn establish_deny_all(")
        .expect("deny-all establishment");
    let allocation = establishment
        .find("IommuTable::new(TABLE_LIMIT, walks)")
        .expect("final deny-all table allocation");
    let repeated_preflight = establishment
        .find("if !takeover_ready(status) {")
        .expect("post-allocation takeover preflight");
    let construction = establishment
        .find("let mut unit = Self {")
        .expect("first hardware-owning value construction");
    assert!(allocation < repeated_preflight && repeated_preflight < construction);
    assert!(!vtd_source.contains("fn quiesce("));
    let invalidation = vtd_source
        .split_once("fn invalidate_interrupt_cache(")
        .expect("interrupt-cache invalidation")
        .1
        .split_once("fn switch_queued_invalidation(")
        .expect("interrupt-cache invalidation end")
        .0;
    let queue_enable = invalidation
        .find("self.switch_queued_invalidation(true)?")
        .expect("QI enable");
    let queue_readback = invalidation
        .find("let observed_queue = read_u64(&self.registers, vtd::IQA)?;")
        .expect("IQA readback");
    let tail = invalidation
        .find("write_u64(&mut self.registers, vtd::IQT, INVALIDATION_TAIL)?;")
        .expect("QI tail publication");
    let queue_disable = invalidation
        .find("let disabled = self.switch_queued_invalidation(false);")
        .expect("QI disable");
    assert!(queue_enable < queue_readback && queue_readback < tail && tail < queue_disable);
    let fault_mask = vtd_source
        .split_once("fn mask_fault_interrupt(")
        .expect("fault-event interrupt mask")
        .1
        .split_once("fn switch_translation(")
        .expect("fault-event interrupt mask end")
        .0;
    assert!(fault_mask.contains("let prior = read_u32(&self.registers, vtd::FECTL)?;"));
    assert!(fault_mask.contains("prior | fault_event_control::MASK"));
    assert!(fault_mask.contains("fault_event_control::masked(observed)"));

    let iommu = source("kernel/src/iommu.rs");
    for requirement in [
        "if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {",
        "if dmar.sole_unit_segment() != Some(SEGMENT) {",
        "if !covers_every_present_requester(&requesters) {",
        "for bus in 0..=u8::MAX {",
        "if !requesters.iter().any(|covered| covered == requester) {",
        "pub(crate) fn establish_fabric_boundary(context: &mut ThreadContext) -> bool {",
        "acpi::find_unique_table(firmware, root, Sdt::DMA_COVERAGE)",
        "acpi::find_unique_table(firmware, root, Sdt::DMAR)",
        "DmaCoverage::new(&table)",
        "dmar.sole_complete_unit_register_base(SEGMENT)",
        "dmar.sole_rmrr_free_unit_register_base(SEGMENT)",
        "Vtd::establish_deny_all(registers, true)",
        "pub(crate) fn ensure_interrupt_boundary() -> bool {",
        "pci::TopologyPresence::Empty | pci::TopologyPresence::Present => {}",
        ".filter(|owner| owner.state == InstalledState::Ready && owner.boundary_active())",
    ] {
        assert!(
            iommu.contains(requirement),
            "kernel interrupt boundary lost `{requirement}`"
        );
    }
    assert_eq!(
        iommu
            .matches("if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {")
            .count(),
        2,
        "both early fabric establishment and sealed-topology lookup must refuse missing interrupt-remapping authority or x2APIC opt-out"
    );
    assert_eq!(iommu.matches("Vtd::establish_deny_all(").count(), 1);
    let early_register =
        function_after(&iommu, "fn early_register_base(").expect("static early VT-d authority");
    assert!(
        early_register
            .contains("coverage.segment() == SEGMENT && coverage.register_base() == register_base")
    );
    assert!(iommu.contains("pci::TopologyPresence::Unsealed => return false,"));
    assert!(iommu.contains(
        "INSTALLED\n        .lock()\n        .as_ref()\n        .is_some_and(|unit| unit.state == InstalledState::Ready && unit.boundary_active())"
    ));
    let acpi = source("libs/acpi-tables/src/requesters.rs");
    let early_unit = source_through(
        &acpi,
        "pub fn sole_rmrr_free_unit_register_base(",
        "pub fn sole_complete_unit_register_base(",
    )
    .expect("static whole-fabric unit query");
    assert!(early_unit.contains("DmarStructure::Rmrr { segment: reserved, .. }"));
    assert!(early_unit.contains("include_pci_all: true"));

    let acpi = source("kernel/src/acpi.rs");
    let unique = source_through(
        &acpi,
        "pub(crate) fn find_unique_table(",
        "fn find_table_with_multiplicity(",
    )
    .expect("unique ACPI authority lookup");
    assert!(unique.contains("if require_unique {"));
    assert!(unique.contains("return None;"));

    let pci = source("wost/src/arch/x86_64/pci.rs");
    assert!(pci.contains("if interrupts.function() != domain.function() {"));

    let kernel_pci = source("kernel/src/pci.rs");
    assert!(kernel_pci.contains(
        "pub(crate) enum TopologyPresence {\n    Unsealed,\n    Empty,\n    Present,\n}"
    ));
    assert!(
        kernel_pci.contains(
            "if !topology_is_sealed() {\n        return TopologyPresence::Unsealed;\n    }"
        )
    );

    let main = source("kernel/src/main.rs");
    assert!(main.contains("if !iommu::interrupt_boundary_active() {"));
    assert!(main.contains("pci::TopologyPresence::Empty"));
    assert!(main.contains("if let Err(reason) = apic::enable_after_pending_audit() {"));
    let survey = main
        .split_once(
            "fn survey_the_bus(\n    dma_non_requester: Option<pci_config::QualifiedNonRequester>,\n    dma_quiesced: Option<pci_config::QuiescedIsaRequester>,\n)",
        )
        .expect("PCI survey")
        .1
        .split_once("enum StorageSelection")
        .expect("PCI survey end")
        .0;
    let early = survey
        .find("iommu::establish_fabric_boundary(&mut context)")
        .expect("early VT-d owner");
    let snapshot = survey
        .find("pci::snapshot_topology(dma_non_requester, dma_quiesced)")
        .expect("PCI topology snapshot");
    let enumeration = survey.find("pci::enumerate(").expect("PCI enumeration");
    assert!(early < snapshot && snapshot < enumeration);
    let start = main
        .split_once("fn start_interrupts(")
        .expect("initial interrupt admission")
        .1
        .split_once("const fn external_interrupts_are_admitted")
        .expect("initial interrupt admission end")
        .0;
    let remapping = start
        .find("if !iommu::interrupt_boundary_active() {")
        .expect("PCI interrupt boundary before sti");
    let pending = start
        .find("apic::enable_after_pending_audit()")
        .expect("pending-vector audit before sti");
    let ready = start
        .find("MASKABLE_INTERRUPTS_READY.store(true")
        .expect("interrupt-ready publication");
    assert!(remapping < pending && pending < ready);

    let apic = source("wost/src/arch/x86_64/apic.rs");
    assert!(apic.contains(
        "for (bits, register) in requests.iter_mut().zip(register::INTERRUPT_REQUEST) {"
    ));
    assert!(apic.contains(
        "if let Some(vector) = pending_vector(&access) {\n        return Err(PendingInterruptError::Pending(vector));\n    }"
    ));
    let audit = apic
        .split_once("pub fn enable_after_pending_audit()")
        .expect("local APIC pending audit")
        .1
        .split_once("fn initialize_controller")
        .expect("local APIC pending audit end")
        .0;
    let irr = audit
        .find("pending_vector(&access)")
        .expect("all-bank IRR audit");
    let refusal = audit
        .find("PendingInterruptError::Pending")
        .expect("pending-vector refusal");
    let sti = audit.find("interrupt::enable()").expect("initial sti");
    assert!(irr < refusal && refusal < sti);
    let route = main
        .split_once("fn route_keyboard(")
        .expect("keyboard routing boundary")
        .1
        .split_once("enum KeyboardRouteError")
        .expect("keyboard routing boundary end")
        .0;
    assert!(
        route.contains("if !matches!(pci::topology_presence(), pci::TopologyPresence::Empty) {"),
        "PCI presence must take the live keyboard-routing rollback branch"
    );
    let boundary = route
        .find("pci::topology_presence()")
        .expect("PCI topology-state routing branch");
    let rollback = route[boundary..]
        .find("rollback_keyboard_route(routing)")
        .map(|position| boundary + position)
        .expect("PCI routing rollback");
    let returned = route[rollback..]
        .find("return;")
        .map(|position| rollback + position)
        .expect("PCI routing rollback return");
    let commit = route
        .rfind("commit_keyboard_route(routing)")
        .expect("non-PCI keyboard routing commit");
    assert!(boundary < rollback && rollback < returned && returned < commit);

    let input = source("kernel/src/input.rs");
    assert_eq!(
        input
            .matches("crate::time::is_calibrated() && interrupt::are_enabled()")
            .count(),
        2,
        "a poller may sleep only while timer interrupts can wake it"
    );
}
