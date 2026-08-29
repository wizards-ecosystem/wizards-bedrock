//! Finding the interrupt controllers ACPI describes.
//!
//! Enough of the table tree to find every bounded I/O APIC and learn which
//! global system interrupt a legacy IRQ arrives on. Parsing and inventory are
//! `acpi_tables`, host-tested; the bytes come through `wost`'s ACPI-only
//! physical reader.
//!
//! Read before the page tables are built. That was forced while mapping was
//! one-way (D-45, D-49); since D-86 it is merely the simplest order, because
//! the bootloader's direct map still covers the tables at that point and
//! nothing has had to be mapped to reach them.

use acpi_tables::{MadtInterrupts, Rsdp, Sdt, SdtPointers};
use alloc::vec::Vec;
use wost::mm::{self, FirmwareMemory};

use crate::{debug, warn};

/// I/O APIC descriptions this boot may temporarily map at once.
const MAX_IO_APICS: usize = 16;

/// A corrupt firmware length must not turn table discovery into an unbounded
/// allocation. Real ACPI tables are far smaller than this ceiling.
const MAX_FIRMWARE_TABLE_BYTES: usize = 1 << 20;

/// What ACPI says about interrupt routing.
pub(crate) type Interrupts = MadtInterrupts<MAX_IO_APICS>;

/// Walk the ACPI tables from the root pointer and report interrupt routing.
///
/// `rsdp` is the *virtual* address the bootloader reported, which base
/// revision 4 guarantees is inside the direct map.
pub(crate) fn interrupts(rsdp: u64) -> Option<Interrupts> {
    // Each refusal says so: the reader only reaches ACPI storage, and a boot
    // that lost its routing to that rule is otherwise a keyboard that stopped
    // working for no stated reason.
    let Some(firmware) = mm::firmware_memory() else {
        warn!("acpi: no reader for firmware memory");
        return None;
    };
    let Some(root) = read_root_pointer(firmware, rsdp) else {
        warn!("acpi: the root pointer at {rsdp:#x} is not readable acpi storage");
        return None;
    };
    let root = match Rsdp::parse(&root) {
        Ok(root) => root,
        Err(reason) => {
            warn!("acpi: root pointer unusable: {reason}");
            return None;
        }
    };

    let madt = find_madt(firmware, root)?;
    let interrupts = match Interrupts::parse(&madt) {
        Ok(interrupts) => interrupts,
        Err(reason) => {
            warn!("acpi: madt unusable: {reason}");
            return None;
        }
    };

    for io_apic in interrupts.io_apics() {
        debug!(
            "acpi: io apic {} at {:#x}, gsi base {}",
            io_apic.id(),
            io_apic.address(),
            io_apic.gsi_base()
        );
    }
    for source in 0..16 {
        if let Some((gsi, flags)) = interrupts.override_for(source) {
            debug!("acpi: irq {source} is routed to gsi {gsi}, flags {flags:#x}");
        }
    }
    (interrupts.io_apic_count() != 0).then_some(interrupts)
}

/// The MADT, found by walking whichever root table the RSDP offers.
fn find_madt(firmware: FirmwareMemory, root: Rsdp) -> Option<Vec<u8>> {
    let (address, extended) = match (root.xsdt(), root.rsdt()) {
        // The XSDT wherever it exists: the RSDT's entries are 32-bit and
        // cannot name a table above 4 GiB.
        (Some(xsdt), _) => (xsdt, true),
        (None, Some(rsdt)) => (rsdt, false),
        (None, None) => {
            warn!("acpi: root pointer names neither an xsdt nor an rsdt");
            return None;
        }
    };

    let Some(root_table) = read_table(firmware, address) else {
        warn!("acpi: the root table at {address:#x} is not readable acpi storage");
        return None;
    };
    let pointers = if extended {
        SdtPointers::xsdt(&root_table)
    } else {
        SdtPointers::rsdt(&root_table)
    };
    let pointers = match pointers {
        Ok(pointers) => pointers,
        Err(reason) => {
            warn!("acpi: root table unusable: {reason}");
            return None;
        }
    };

    for entry in pointers {
        let Some(table) = read_table(firmware, entry) else {
            continue;
        };
        if let Ok(header) = Sdt::parse(&table)
            && header.signature() == Sdt::MADT
        {
            return Some(table);
        }
    }
    warn!("acpi: no madt among the tables listed");
    None
}

/// A whole table at a physical address, read in two steps.
///
/// The header must be read before the length is known, and the length is what
/// says how much to read — so a header is fetched first and then the whole
/// table, which is also what lets `Sdt::parse` checksum every byte.
fn read_table(firmware: FirmwareMemory, address: u64) -> Option<Vec<u8>> {
    let mut header = [0; Sdt::LEN];
    if !firmware.read(address, &mut header) {
        return None;
    }
    let length = Sdt::parse_length(&header, MAX_FIRMWARE_TABLE_BYTES).ok()?;
    let mut table = zeroed(length)?;
    firmware.read(address, &mut table).then_some(table)
}

/// Read the variable-length root pointer from its direct-map address.
fn read_root_pointer(firmware: FirmwareMemory, address: u64) -> Option<Vec<u8>> {
    let offset = mm::direct_map()?.offset();
    let physical = address.checked_sub(offset)?;
    let mut original = [0; Rsdp::V1_LEN];
    if !firmware.read(physical, &mut original) {
        return None;
    }
    let length = if original[15] == 2 {
        let mut prefix = [0; 24];
        if !firmware.read(physical, &mut prefix) {
            return None;
        }
        Rsdp::parse_length(&prefix, MAX_FIRMWARE_TABLE_BYTES).ok()?
    } else {
        Rsdp::parse_length(&original, MAX_FIRMWARE_TABLE_BYTES).ok()?
    };
    let mut bytes = zeroed(length)?;
    firmware.read(physical, &mut bytes).then_some(bytes)
}

/// Allocate a zeroed firmware buffer without invoking the allocation handler.
fn zeroed(length: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).ok()?;
    bytes.resize(length, 0);
    Some(bytes)
}
