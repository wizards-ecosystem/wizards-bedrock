use alloc::vec::Vec;

use boot_record::{Fact, Record};
use wost::arch::current::interrupt;
use wost::arch::current::paging::{self, ReleasableDevice};
use wost::sync::SpinLock;
use x86_apic::io;

use crate::acpi::Interrupts;
use crate::arch::current::ioapic::IoApic;
use crate::mem::{DeviceRetirement, map_device, unmap_device_exact};
use crate::thread::ThreadContext;
use crate::{info, warn};

pub(crate) fn report_keyboard(masked: Option<ExternalControllersMasked>) {
    let Some(masked) = masked else {
        warn!("keyboard: external controllers are unproven; interrupts and routing stay off");
        return;
    };
    match masked.keyboard_line {
        KeyboardLine::Unowned => {
            info!("keyboard: no io apic owns its line; a poller thread drains it instead");
        }
        KeyboardLine::Owned if interrupt::are_enabled() => {
            info!("keyboard: PCI interrupt isolation keeps every io-apic line masked; polling");
            info!("{}", Record::Fact(Fact::KeyboardLinesMasked));
        }
        KeyboardLine::Owned => {
            info!("keyboard: interrupts stayed off; a poller thread drains it instead");
        }
    }
}

// Must run before `interrupts::start`, so every controller is masked while delivery is refused.
pub(crate) fn mask_external_controllers(
    context: &mut ThreadContext,
    interrupts: Option<Interrupts>,
) -> Option<ExternalControllersMasked> {
    let Some(interrupts) = interrupts else {
        warn!("ioapic: no complete firmware controller inventory; interrupts stay off");
        return None;
    };
    let gsi = interrupts.gsi_for(KEYBOARD_IRQ);
    mask(context, &interrupts, gsi)
}

const KEYBOARD_IRQ: u8 = 1;

static RETAINED_MAPPING: SpinLock<Option<(paging::DeviceMapping, Vec<IoApic>)>> =
    SpinLock::new(None);

static RETAINED_REGISTERS: SpinLock<Option<(ReleasableDevice, Vec<IoApic>)>> = SpinLock::new(None);

#[derive(Debug)]
pub(crate) struct ExternalControllersMasked {
    keyboard_line: KeyboardLine,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KeyboardLine {
    Owned,
    Unowned,
}

fn mask(
    context: &mut ThreadContext,
    interrupts: &Interrupts,
    gsi: u32,
) -> Option<ExternalControllersMasked> {
    let count = interrupts.io_apic_count();
    if count == 0 {
        warn!("ioapic: firmware supplied no controller inventory");
        return None;
    }
    let mut controllers = Vec::new();
    let mut ranges = Vec::new();
    if controllers.try_reserve_exact(count).is_err() || ranges.try_reserve_exact(count).is_err() {
        warn!("ioapic: no memory for {count} routing candidates; interrupts stay off");
        return None;
    }

    for description in interrupts.io_apics() {
        let controller = match probe_io_apic(context, description) {
            IoApicProbe::Ready(controller) => controller,
            IoApicProbe::Refused => {
                if !release_io_apics(context, &mut controllers) {
                    warn!("ioapic: candidate cleanup was incomplete; interrupts stay off");
                }
                return None;
            }
            IoApicProbe::Retained(owner) => {
                retain_probe_owner(owner, controllers);
                return None;
            }
        };
        ranges.push(controller.range());
        controllers.push(controller);
    }

    let keyboard_line = match io::owner_of(&ranges, gsi) {
        Ok(Some(_)) => KeyboardLine::Owned,
        Ok(None) => {
            warn!("keyboard: no io apic owns gsi {gsi}; staying polled");
            KeyboardLine::Unowned
        }
        Err(reason) => {
            warn!("ioapic: {reason}; staying polled");
            KeyboardLine::Unowned
        }
    };
    if !release_io_apics(context, &mut controllers) {
        if keyboard_line == KeyboardLine::Owned {
            warn!("ioapic: candidate mappings did not retire exactly; interrupts stay off");
        }
        return None;
    }
    Some(ExternalControllersMasked { keyboard_line })
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

#[must_use = "a refused release leaves a mapping quarantined"]
fn release_io_apics(context: &mut ThreadContext, controllers: &mut Vec<IoApic>) -> bool {
    while let Some(controller) = controllers.pop() {
        if let DeviceRetirement::Retained(registers) =
            unmap_device_exact(context, controller.into_releasable())
        {
            warn!("ioapic: candidate mapping remained owned during cleanup");
            retain_registers(registers, core::mem::take(controllers));
            return false;
        }
    }
    true
}

// The index and data registers sit in the first 32 bytes; a page is the smallest mapping.
const IO_APIC_WINDOW: u64 = 4096;

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
