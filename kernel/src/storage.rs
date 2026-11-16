use core::sync::atomic::Ordering;

use wost::arch::current::pci as platform_pci;

use crate::{devfs, fs, info, interrupts, iommu, mem, nvme, pci, thread, tmpfs, vfs, warn};

pub(crate) fn survey_the_bus(
    dma_non_requester: Option<pci_config::QualifiedNonRequester>,
    dma_quiesced: Option<pci_config::QuiescedIsaRequester>,
) {
    let early_boundary = thread::ThreadContext::claim()
        .is_some_and(|mut context| iommu::establish_fabric_boundary(&mut context));
    if !early_boundary {
        warn!("iommu: no early default-deny PCI boundary; storage and interrupts stay closed");
    }

    match pci::snapshot_topology(dma_non_requester, dma_quiesced) {
        Ok(()) => info!("pci: complete segment-zero topology sealed"),
        Err(reason) => warn!("pci: DMA topology refused: {reason}"),
    }

    let mut selection = Selection::Seeking;
    let mut storage = 0;
    let surveyed = pci::enumerate(|function| {
        if function.class.is_nvme() {
            storage += 1;
            if matches!(selection, Selection::Seeking) {
                selection = try_controller(function.at, storage);
            }
        }
    });

    let Some(survey) = surveyed else {
        warn!("pci: configuration space did not answer");
        return;
    };
    info!(
        "pci: {} functions across {} bus(es), {} bridge(s), highest {:#04x}, {storage} nvme",
        survey.functions, survey.buses, survey.bridges, survey.highest_bus
    );

    let interrupt_boundary = iommu::ensure_interrupt_boundary();
    if !interrupt_boundary {
        warn!("iommu: PCI interrupt isolation is incomplete; maskable interrupts stay off");
    }

    match selection {
        Selection::Selected => mount_the_volume(),
        Selection::Seeking if storage == 0 => {
            info!("pci: no nvme controller; nothing to claim")
        }
        Selection::Seeking => info!("nvme: no usable controller"),
        Selection::Halted => {
            warn!("nvme: selection stopped because a candidate could not be retired")
        }
    }
    interrupts::PCI_INTERRUPT_BOUNDARY.store(interrupt_boundary, Ordering::Release);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Selection {
    Seeking,
    Selected,
    Halted,
}

#[derive(Debug)]
enum CandidateStage {
    Prepared(pci::Device, nvme::DisabledController, Isolation),
    Admitted(pci::Device, nvme::BusMasterAdmittedController),
    Retire(pci::Device),
    Settled(Selection),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Isolation {
    Pending,
    Established,
}

impl CandidateStage {
    // Selection stays halted until a stage puts its result back.
    fn take(&mut self) -> Self {
        core::mem::replace(self, Self::Settled(Selection::Halted))
    }

    fn take_prepared(&mut self) -> Option<(pci::Device, nvme::DisabledController)> {
        match self.take() {
            Self::Prepared(device, prepared, Isolation::Established) => Some((device, prepared)),
            other => {
                *self = other;
                None
            }
        }
    }

    fn take_admitted(&mut self) -> Option<(pci::Device, nvme::BusMasterAdmittedController)> {
        match self.take() {
            Self::Admitted(device, admitted) => Some((device, admitted)),
            other => {
                *self = other;
                None
            }
        }
    }
}

// Each stage returns its owners here, so one stage frame at a time sits on the boot stack.
fn try_controller(at: pci_config::Address, ordinal: usize) -> Selection {
    let mut context = thread::ThreadContext::require();

    let mut stage = claim_candidate(at, ordinal, &mut context);
    let device = loop {
        if matches!(&stage, CandidateStage::Prepared(_, _, Isolation::Pending)) {
            isolate_candidate(&mut stage);
            continue;
        }
        stage = match stage {
            CandidateStage::Prepared(..) => admit_candidate(&mut stage, &mut context),
            CandidateStage::Admitted(..) => start_candidate(&mut stage, &mut context, ordinal),
            CandidateStage::Retire(device) => break device,
            CandidateStage::Settled(selection) => return selection,
        };
    };
    retire_candidate(device, ordinal)
}

fn claim_candidate(
    at: pci_config::Address,
    ordinal: usize,
    context: &mut thread::ThreadContext,
) -> CandidateStage {
    let mut device = match pci::lease(at) {
        Ok(device) => device,
        Err(reason) => return CandidateStage::Settled(refused_lease(at, reason)),
    };
    let (vendor, model) = device.identity();
    info!(
        "nvme: claimed {:02x}:{:02x}.{} — {vendor:04x}:{model:04x}",
        at.bus(),
        at.device(),
        at.function()
    );

    let registers = match admit_controller_window(&mut device, context) {
        ControllerWindowAdmission::Admitted(registers) => registers,
        ControllerWindowAdmission::Refused => return CandidateStage::Retire(device),
        ControllerWindowAdmission::Quarantined => {
            pci::retain_failed_release(device);
            return CandidateStage::Settled(Selection::Halted);
        }
    };
    let prepared = match nvme::prepare(context, registers) {
        nvme::PrepareOutcome::Prepared(prepared) => prepared,
        nvme::PrepareOutcome::Failed(failure) => {
            return failed_preparation(failure, device, ordinal);
        }
    };
    CandidateStage::Prepared(device, prepared, Isolation::Pending)
}

// Borrows the stage, so the prepared controller stays out of the VT-d establishment frame.
fn isolate_candidate(stage: &mut CandidateStage) {
    let outcome = match stage {
        CandidateStage::Prepared(device, _, Isolation::Pending) => iommu::establish(device),
        _ => return,
    };
    match outcome {
        iommu::EstablishOutcome::Established => {}
        iommu::EstablishOutcome::Unavailable => warn!("storage: no DMA isolation"),
        iommu::EstablishOutcome::Quarantined => {
            quarantine_isolation(stage);
            return;
        }
    }
    let CandidateStage::Prepared(_, _, isolation) = stage else {
        panic!("the isolation stage lost its prepared candidate");
    };
    *isolation = Isolation::Established;
}

fn quarantine_isolation(stage: &mut CandidateStage) {
    let CandidateStage::Prepared(device, prepared, Isolation::Pending) = stage.take() else {
        panic!("an isolation quarantine lost its prepared candidate");
    };
    warn!("storage: DMA isolation state is uncertain; selection halted");
    nvme::keep_iommu_quarantine(prepared, device);
    *stage = CandidateStage::Settled(Selection::Halted);
}

fn refused_lease(at: pci_config::Address, reason: pci::ClaimError) -> Selection {
    let bus = at.bus();
    let device_number = at.device();
    let function = at.function();
    match reason {
        pci::ClaimError::ProbeRestore(recovery) => match recovery.retry() {
            pci::ProbeRecoveryOutcome::Recovered => {
                warn!(
                    "nvme: {:02x}:{:02x}.{} probe failed but restored on retry",
                    bus, device_number, function
                );
                Selection::Seeking
            }
            pci::ProbeRecoveryOutcome::Refused(failure) => {
                warn!(
                    "nvme: {:02x}:{:02x}.{} probe restoration remains uncertain: {:?}",
                    bus,
                    device_number,
                    function,
                    failure.reason()
                );
                pci::retain_failed_probe(failure.into_recovery());
                Selection::Halted
            }
            pci::ProbeRecoveryOutcome::FunctionRetained => {
                warn!(
                    "nvme: {:02x}:{:02x}.{} probe restored but its function claim remains retained",
                    bus, device_number, function
                );
                Selection::Halted
            }
        },
        pci::ClaimError::FunctionRetained => {
            warn!(
                "nvme: {:02x}:{:02x}.{} preparation failed with its function claim retained",
                bus, device_number, function
            );
            Selection::Halted
        }
        reason => {
            warn!(
                "nvme: {:02x}:{:02x}.{} could not be claimed: {reason}",
                bus, device_number, function
            );
            Selection::Seeking
        }
    }
}

fn failed_preparation(
    failure: nvme::PrepareFailure,
    device: pci::Device,
    ordinal: usize,
) -> CandidateStage {
    let reason = failure.reason();
    let Some(quarantine) = failure.into_quarantine() else {
        return CandidateStage::Retire(device);
    };
    warn!(
        "nvme: candidate {ordinal} left uncertain controller state after {}",
        reason
    );
    nvme::keep_prepare_quarantine(quarantine, device);
    CandidateStage::Settled(Selection::Halted)
}

fn admit_candidate(
    stage: &mut CandidateStage,
    context: &mut thread::ThreadContext,
) -> CandidateStage {
    let Some((mut device, prepared)) = stage.take_prepared() else {
        return stage.take();
    };
    let admitted = match prepared.admit_bus_master(&mut device) {
        nvme::BusMasterAdmission::Admitted(admitted) => admitted,
        nvme::BusMasterAdmission::Refused(failure) => {
            warn!(
                "nvme: bus master refused after verified disable: {}",
                failure.reason()
            );
            match failure.into_disabled().retire(&mut device, context) {
                nvme::ControllerRetirement::Retired => {}
                nvme::ControllerRetirement::Quarantined(quarantine) => {
                    nvme::keep_controller_quarantine(quarantine, device);
                    return CandidateStage::Settled(Selection::Halted);
                }
            }
            return CandidateStage::Retire(device);
        }
    };
    info!("nvme: bus master on after verified controller disable");
    CandidateStage::Admitted(device, admitted)
}

#[inline(never)]
fn allocate_cleared_startup_buffer() -> Option<wost::mm::DmaSegment> {
    let mut segment = wost::mm::allocate_contiguous(1, 1)?;
    if !segment.zero() {
        return None;
    }
    Some(wost::mm::DmaSegment::from(segment))
}

fn start_candidate(
    stage: &mut CandidateStage,
    context: &mut thread::ThreadContext,
    ordinal: usize,
) -> CandidateStage {
    let Some((mut device, admitted)) = stage.take_admitted() else {
        return stage.take();
    };
    let mut controller = match nvme::start(context, &mut device, admitted) {
        Ok(controller) => controller,
        Err(failure) => {
            let reason = failure.reason();
            match failure.into_retirement() {
                nvme::ControllerRetirement::Retired => return CandidateStage::Retire(device),
                nvme::ControllerRetirement::Quarantined(quarantine) => {
                    warn!(
                        "nvme: candidate {ordinal} left uncertain controller state after {}",
                        reason
                    );
                    nvme::keep_controller_quarantine(quarantine, device);
                    return CandidateStage::Settled(Selection::Halted);
                }
            }
        }
    };

    let Some(mut buffer) = allocate_cleared_startup_buffer() else {
        warn!("nvme: no cleared memory for a read buffer");
        match controller.retire(&mut device, context) {
            nvme::ControllerRetirement::Retired => {}
            nvme::ControllerRetirement::Quarantined(quarantine) => {
                nvme::keep_controller_quarantine(quarantine, device);
                return CandidateStage::Settled(Selection::Halted);
            }
        }
        return CandidateStage::Retire(device);
    };
    match controller.read_blocks(0, 1, &mut buffer) {
        Ok(()) => info!("nvme: read block 0"),
        Err(reason) => {
            warn!("nvme: block 0 could not be read: {reason}");
            let retirement = controller.retire(&mut device, context);
            iommu::report_faults();
            match retirement {
                nvme::ControllerRetirement::Retired => {}
                nvme::ControllerRetirement::Quarantined(quarantine) => {
                    nvme::keep_controller_quarantine(quarantine, device);
                    drop(buffer);
                    return CandidateStage::Settled(Selection::Halted);
                }
            }
            drop(buffer);
            return CandidateStage::Retire(device);
        }
    }
    if let Err(reason) = device.commit() {
        warn!("nvme: pci command commit refused: {reason}");
        pci::retain_failed_commit(device);
        nvme::keep_uncommitted(controller, buffer);
        return CandidateStage::Settled(Selection::Halted);
    }
    nvme::keep(controller, buffer);
    CandidateStage::Settled(Selection::Selected)
}

fn retire_candidate(mut device: pci::Device, ordinal: usize) -> Selection {
    if let Err(reason) = device.disable_bus_master_for_retirement() {
        warn!("nvme: candidate {ordinal} bus-master fence was refused once: {reason}");
        if let Err(reason) = device.disable_bus_master_for_retirement() {
            warn!(
                "nvme: candidate {ordinal} could not prove bus mastering off after retry: {reason}; selection halted"
            );
            pci::retain_failed_release(device);
            return Selection::Halted;
        }
    }
    if let Err(reason) = device.settle_bus_master_fence_for_retirement() {
        warn!("nvme: candidate {ordinal} safe command restoration was refused once: {reason}");
        if let Err(reason) = device.settle_bus_master_fence_for_retirement() {
            warn!(
                "nvme: candidate {ordinal} could not settle its bus-master fence after retry: {reason}; selection halted"
            );
            pci::retain_failed_release(device);
            return Selection::Halted;
        }
    }
    // Bus mastering is proved off while the function claim still holds it, before the domain goes.
    if !iommu::release_candidate() {
        warn!("nvme: candidate {ordinal} could not return its dma domain; selection halted");
        pci::retain_failed_release(device);
        return Selection::Halted;
    }
    if let Some((reason, device)) = device.release().into_retained() {
        warn!(
            "nvme: candidate {ordinal} pci lease retirement was refused once: {}",
            reason
        );
        if let Some((reason, device)) = device.release().into_retained() {
            warn!(
                "nvme: candidate {ordinal} could not return its pci lease after retry: {}; selection halted",
                reason
            );
            pci::retain_failed_release(device);
            return Selection::Halted;
        }
    }
    info!("nvme: candidate {ordinal} retired cleanly; trying next");
    Selection::Seeking
}

#[derive(Debug)]
enum ControllerWindowAdmission {
    Admitted(platform_pci::MappedBar),
    Refused,
    Quarantined,
}

fn admit_controller_window(
    device: &mut pci::Device,
    context: &mut thread::ThreadContext,
) -> ControllerWindowAdmission {
    // BAR0 is the register file; a controller whose first window is not memory is unreachable.
    let Some(window) = device.window(0) else {
        warn!("nvme: no bar0; nothing to map");
        return ControllerWindowAdmission::Refused;
    };
    let pci_config::Bar::Memory { address, wide, .. } = window.bar else {
        warn!("nvme: bar0 is not a memory window");
        return ControllerWindowAdmission::Refused;
    };
    info!(
        "nvme: bar0 is {} bytes of {}-bit memory at {address:#x}",
        window.size,
        if wide { 64 } else { 32 }
    );
    let admitted = match device.admit_memory_bar(0) {
        Ok(admitted) => admitted,
        Err(reason) => {
            warn!("nvme: bar0 physical range refused: {reason}");
            return ControllerWindowAdmission::Refused;
        }
    };
    let (page_start, page_end, physical_bits) = admitted.physical_admission();
    let mapping = match mem::map_pci_bar(context, admitted) {
        Ok(mapping) => mapping,
        Err(failure) => {
            match failure.reason() {
                Some(reason) => warn!("nvme: bar0 mapping refused: {reason}"),
                None => warn!("nvme: no active address space for bar0"),
            }
            if let Err(failure) = platform_pci::release_admitted_bar(failure.into_admitted()) {
                warn!("nvme: bar0 authority return refused: {failure}");
                pci::retain_bar_authority(failure);
                return ControllerWindowAdmission::Quarantined;
            }
            return ControllerWindowAdmission::Refused;
        }
    };
    let disabled = match mapping.claim_disabled() {
        Ok(disabled) => disabled,
        Err(mapping) => {
            warn!("nvme: the installed bar0 mapping is absent from the active root");
            pci::retain_bar_mapping(mapping);
            return ControllerWindowAdmission::Quarantined;
        }
    };
    let mapped = match device.enable_memory_bar(disabled) {
        pci::BarDecodeOutcome::Enabled(mapped) => mapped,
        pci::BarDecodeOutcome::Refused(failure) => {
            let reason = failure.reason();
            let retirement =
                mem::unmap_pci_bar_exact(context, failure.into_bar().into_releasable());
            warn!("nvme: memory-space decode refused: {reason}; mapping {retirement:?}");
            return match retirement {
                mem::PciBarRetirement::Retired => ControllerWindowAdmission::Refused,
                mem::PciBarRetirement::Quarantined => ControllerWindowAdmission::Quarantined,
            };
        }
    };
    info!(
        "nvme: bar0 pages {page_start:#x}..{page_end:#x} admitted below MAXPHYADDR {physical_bits}"
    );
    info!("nvme: memory-space decode on; bus master remains off");
    ControllerWindowAdmission::Admitted(mapped)
}

fn mount_the_volume() {
    let shape = fs::mount().map(|result| result.map(|volume| volume.shape()));
    match shape {
        None => {
            info!("fat32: no controller, so no volume");
            return;
        }
        Some(Err(reason)) => {
            warn!("fat32: the volume would not mount: {reason:?}");
            return;
        }
        Some(Ok((clusters, cluster_bytes))) => {
            info!("fat32: mounted — {clusters} clusters of {cluster_bytes} bytes");
        }
    }

    let Some(root) = vfs::root() else {
        return;
    };

    let mut files = 0usize;
    let mut directories = 0usize;
    let mut first = [0u8; 16];
    let mut first_length = 0usize;
    if let Err(reason) = vfs::enumerate(root, |name, is_directory| {
        if is_directory {
            directories += 1;
        } else {
            files += 1;
            if first_length == 0 && name.len() <= first.len() {
                first[..name.len()].copy_from_slice(name);
                first_length = name.len();
            }
        }
        true
    }) {
        warn!("fat32: the root would not list: {reason:?}");
        return;
    }
    info!("fat32: {files} file(s) and {directories} directory(ies) in the root");

    if first_length == 0 {
        return;
    }
    let name = &first[..first_length];
    match vfs::resolve(root, name) {
        Ok(vfs::Node::File(file)) => {
            let mut head = [0u8; 32];
            match vfs::read(file, 0, &mut head) {
                Ok(read) => info!(
                    "fat32: {} — read {read} of its bytes",
                    core::str::from_utf8(name).unwrap_or("?")
                ),
                Err(reason) => warn!("fat32: that file would not read: {reason:?}"),
            }
        }
        Ok(vfs::Node::Directory(_)) => warn!("fat32: a listed file resolved to a directory"),
        Ok(vfs::Node::Device(_)) => warn!("fat32: a listed file resolved to a device"),
        Err(reason) => warn!("fat32: a listed name would not resolve: {reason:?}"),
    }
}

pub(crate) fn survey_the_filesystems() {
    tmpfs::init();

    let mut files = 0usize;
    let mut directories = 0usize;
    let listed = vfs::enumerate(vfs::tmp_root(), |_, is_directory| {
        if is_directory {
            directories += 1;
        } else {
            files += 1;
        }
        true
    });
    match listed {
        Ok(()) => {
            info!("tmpfs: {files} file(s) and {directories} directory(ies) at the root");
        }
        Err(reason) => warn!("tmpfs: the root would not list: {reason:?}"),
    }

    for device in devfs::DEVICES {
        match vfs::resolve(vfs::Directory::Dev, device.name()) {
            Ok(vfs::Node::Device(found)) => info!(
                "devfs: {} is {}",
                core::str::from_utf8(found.name()).unwrap_or("?"),
                found.describes()
            ),
            Ok(_) => warn!("devfs: a device resolved to something with bytes in it"),
            Err(reason) => warn!("devfs: a device would not resolve: {reason:?}"),
        }
    }
}
