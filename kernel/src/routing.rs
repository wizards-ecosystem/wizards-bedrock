use alloc::vec::Vec;

use wost::arch::current::interrupt;
use wost::arch::current::paging::{self, ReleasableDevice};
use wost::sync::SpinLock;
use x86_apic::io;

use crate::acpi::{self, Interrupts};
use crate::arch::current::ioapic::IoApic;
use crate::mem::{DeviceRetirement, map_device, unmap_device_exact};
use crate::thread::{self, ThreadContext};
use crate::{info, warn};

pub(crate) fn route_keyboard(routing: Option<ExternalControllersMasked>) {
    let Some(routing) = routing else {
        warn!("keyboard: external controllers are unproven; interrupts and routing stay off");
        return;
    };
    let routing = match routing.into_routing() {
        Some(routing) if interrupt::are_enabled() => routing,
        Some(routing) => {
            if !rollback_keyboard_route(routing) {
                warn!("keyboard: masked routing mapping remains quarantined; staying polled");
                return;
            }
            info!("keyboard: interrupts stayed off; a poller thread drains it instead");
            return;
        }
        None => {
            info!("keyboard: no io apic owns its line; a poller thread drains it instead");
            return;
        }
    };

    if !rollback_keyboard_route(routing) {
        warn!("keyboard: masked routing owner remains quarantined; staying polled");
        return;
    }
    info!("keyboard: PCI interrupt isolation keeps every io-apic line masked; polling");
}

fn rollback_keyboard_route(mut routing: Routing) -> bool {
    let gsi = routing.interrupts.gsi_for(KEYBOARD_IRQ);
    if routing.io_apic.handles(gsi)
        && let Err(reason) = routing.io_apic.mask_and_verify(gsi)
    {
        warn!("keyboard: could not verify rollback mask for gsi {gsi}: {reason}");
        retain(routing);
        return false;
    }
    let Some(mut context) = thread::ThreadContext::claim() else {
        warn!("keyboard: rollback could not claim thread context");
        retain(routing);
        return false;
    };
    if !release(&mut context, routing) {
        warn!("keyboard: rollback could not retire its io-apic mapping");
        return false;
    }
    true
}

// Must run before `interrupts::start`, so every controller is masked while delivery is refused.
pub(crate) fn prepare_keyboard_routing(interrupts: Option<acpi::Interrupts>) -> Preparation {
    let Some(interrupts) = interrupts else {
        warn!("ioapic: no complete firmware controller inventory; interrupts stay off");
        return Preparation::unproven();
    };
    let gsi = interrupts.gsi_for(KEYBOARD_IRQ);
    let Some(mut context) = thread::ThreadContext::claim() else {
        warn!("ioapic: no thread context for mapping; interrupts stay off");
        return Preparation::unproven();
    };
    prepare(&mut context, interrupts, gsi)
}

const KEYBOARD_IRQ: u8 = 1;

static RETAINED: SpinLock<Option<Routing>> = SpinLock::new(None);

static RETAINED_MAPPING: SpinLock<Option<(paging::DeviceMapping, Vec<IoApic>)>> =
    SpinLock::new(None);

static RETAINED_REGISTERS: SpinLock<Option<(ReleasableDevice, Vec<IoApic>)>> = SpinLock::new(None);

#[derive(Debug)]
pub(crate) struct Preparation {
    routing: Option<Routing>,
    controllers_masked: bool,
}

#[derive(Debug)]
pub(crate) struct ExternalControllersMasked {
    routing: Option<Routing>,
}

#[derive(Debug)]
struct ControllersReleased {
    _private: (),
}

impl Preparation {
    fn ready(routing: Routing) -> Self {
        Self {
            routing: Some(routing),
            controllers_masked: true,
        }
    }

    const fn polled_safe(_released: ControllersReleased) -> Self {
        Self {
            routing: None,
            controllers_masked: true,
        }
    }

    const fn unproven() -> Self {
        Self {
            routing: None,
            controllers_masked: false,
        }
    }

    pub(crate) fn into_masked(self) -> Option<ExternalControllersMasked> {
        if !self.controllers_masked {
            return None;
        }
        Some(ExternalControllersMasked {
            routing: self.routing,
        })
    }
}

impl ExternalControllersMasked {
    pub(crate) fn into_routing(self) -> Option<Routing> {
        self.routing
    }
}

fn prepare(context: &mut ThreadContext, interrupts: Interrupts, gsi: u32) -> Preparation {
    let count = interrupts.io_apic_count();
    if count == 0 {
        warn!("ioapic: firmware supplied no controller inventory");
        return Preparation::unproven();
    }
    let mut controllers = Vec::new();
    let mut ranges = Vec::new();
    if controllers.try_reserve_exact(count).is_err() || ranges.try_reserve_exact(count).is_err() {
        warn!("ioapic: no memory for {count} routing candidates; interrupts stay off");
        return Preparation::unproven();
    }

    for description in interrupts.io_apics() {
        let controller = match probe_io_apic(context, description) {
            IoApicProbe::Ready(controller) => controller,
            IoApicProbe::Refused => return abandon_candidates(context, &mut controllers),
            IoApicProbe::Retained(owner) => {
                retain_probe_owner(owner, controllers);
                return Preparation::unproven();
            }
        };
        ranges.push(controller.range());
        controllers.push(controller);
    }

    let owner = match io::owner_of(&ranges, gsi) {
        Ok(Some(owner)) => owner,
        Ok(None) => {
            warn!("keyboard: no io apic owns gsi {gsi}; staying polled");
            return poll_after_release(context, &mut controllers);
        }
        Err(reason) => {
            warn!("ioapic: {reason}; staying polled");
            return poll_after_release(context, &mut controllers);
        }
    };
    let Some(selected) = interrupts.io_apics().nth(owner) else {
        warn!("ioapic: selected controller has no firmware description; interrupts stay off");
        return abandon_candidates(context, &mut controllers);
    };

    // Rebuilt only after every probe mapping is gone, so no probe state survives into ownership.
    if release_io_apics(context, &mut controllers).is_none() {
        warn!("ioapic: candidate mappings did not retire exactly; interrupts stay off");
        return Preparation::unproven();
    }
    let io_apic = match probe_io_apic(context, selected) {
        IoApicProbe::Ready(controller) => controller,
        IoApicProbe::Refused => return Preparation::unproven(),
        IoApicProbe::Retained(owner) => {
            retain_probe_owner(owner, controllers);
            return Preparation::unproven();
        }
    };
    Preparation::ready(Routing {
        io_apic,
        interrupts,
    })
}

fn abandon_candidates(context: &mut ThreadContext, controllers: &mut Vec<IoApic>) -> Preparation {
    if release_io_apics(context, controllers).is_none() {
        warn!("ioapic: candidate cleanup was incomplete; interrupts stay off");
    }
    Preparation::unproven()
}

fn poll_after_release(context: &mut ThreadContext, controllers: &mut Vec<IoApic>) -> Preparation {
    match release_io_apics(context, controllers) {
        Some(released) => Preparation::polled_safe(released),
        None => Preparation::unproven(),
    }
}

#[derive(Debug)]
enum IoApicProbe {
    Ready(IoApic),
    Refused,
    Retained(ProbeOwner),
}

#[derive(Debug)]
enum ProbeOwner {
    Mapping(paging::DeviceMapping),
    Registers(ReleasableDevice),
}

fn retain_probe_owner(owner: ProbeOwner, controllers: Vec<IoApic>) {
    match owner {
        ProbeOwner::Mapping(mapping) => retain_mapping(mapping, controllers),
        ProbeOwner::Registers(registers) => retain_registers(registers, controllers),
    }
}

fn probe_io_apic(context: &mut ThreadContext, description: acpi_tables::IoApic) -> IoApicProbe {
    let id = description.id();
    let mapping = match map_device(context, u64::from(description.address()), IO_APIC_WINDOW) {
        Some(Ok(mapping)) => mapping,
        Some(Err(reason)) => {
            warn!(
                "ioapic {}: registers at {:#x} not mapped: {reason}; interrupts stay off",
                id,
                description.address()
            );
            return IoApicProbe::Refused;
        }
        None => {
            warn!("ioapic: no active address space; interrupts stay off");
            return IoApicProbe::Refused;
        }
    };
    let registers = match mapping.claim_releasable() {
        Ok(registers) => registers,
        Err(mapping) => {
            warn!(
                "ioapic {}: mapped registers are not live; interrupts stay off",
                id
            );
            return IoApicProbe::Retained(ProbeOwner::Mapping(mapping));
        }
    };
    match IoApic::take(registers, description.gsi_base()) {
        Ok(controller) => IoApicProbe::Ready(controller),
        Err(failure) => {
            let reason = failure.reason;
            match unmap_device_exact(context, failure.registers) {
                DeviceRetirement::Retired => {
                    warn!(
                        "ioapic {}: registers unusable: {reason}; mapping retired; interrupts stay off",
                        id
                    );
                    IoApicProbe::Refused
                }
                DeviceRetirement::Retained(registers) => {
                    warn!(
                        "ioapic {}: registers unusable: {reason}; mapping retained; interrupts stay off",
                        id
                    );
                    IoApicProbe::Retained(ProbeOwner::Registers(registers))
                }
            }
        }
    }
}

fn release_io_apics(
    context: &mut ThreadContext,
    controllers: &mut Vec<IoApic>,
) -> Option<ControllersReleased> {
    while let Some(controller) = controllers.pop() {
        if let DeviceRetirement::Retained(registers) =
            unmap_device_exact(context, controller.into_releasable())
        {
            warn!("ioapic: candidate mapping remained owned during cleanup");
            retain_registers(registers, core::mem::take(controllers));
            return None;
        }
    }
    Some(ControllersReleased { _private: () })
}

// The index and data registers sit in the first 32 bytes; a page is the smallest mapping.
const IO_APIC_WINDOW: u64 = 4096;

#[derive(Debug)]
pub(crate) struct Routing {
    io_apic: IoApic,
    interrupts: Interrupts,
}

fn retain_mapping(mapping: paging::DeviceMapping, controllers: Vec<IoApic>) {
    let mut retained = RETAINED_MAPPING.lock_leaf();
    assert!(
        retained.is_none(),
        "routing continued after an exact mapping owner was quarantined"
    );
    *retained = Some((mapping, controllers));
}

fn retain_registers(registers: ReleasableDevice, controllers: Vec<IoApic>) {
    let mut retained = RETAINED_REGISTERS.lock_leaf();
    assert!(
        retained.is_none(),
        "routing continued after an exact register owner was quarantined"
    );
    *retained = Some((registers, controllers));
}

fn retain(routing: Routing) {
    let mut retained = RETAINED.lock_leaf();
    assert!(
        retained.is_none(),
        "routing continued after a controller was quarantined"
    );
    *retained = Some(routing);
}

fn release(context: &mut ThreadContext, routing: Routing) -> bool {
    match unmap_device_exact(context, routing.io_apic.into_releasable()) {
        DeviceRetirement::Retired => true,
        DeviceRetirement::Retained(registers) => {
            retain_registers(registers, Vec::new());
            false
        }
    }
}
