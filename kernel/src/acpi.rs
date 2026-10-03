//! Finding the interrupt controllers ACPI describes.
//!
//! Enough of the table tree to find every bounded I/O APIC and learn which
//! global system interrupt a legacy IRQ arrives on. Parsing and inventory are
//! `acpi_tables`, host-tested; the bytes come through `wost`'s ACPI-only
//! physical reader.
//!
//! Read before the page tables are built because the bootloader's direct map
//! already covers them without adding a mapping dependency (D-45, D-49,
//! D-86).

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
    // Each refusal says so: a boot that lost its routing to the ACPI-only
    // reader is otherwise a keyboard that stopped for no stated reason.
    let (firmware, root) = parse_root(rsdp)?;
    let madt = match find_table(firmware, root, Sdt::MADT) {
        Some(madt) => madt,
        None => {
            warn!("acpi: no madt among the tables listed");
            return None;
        }
    };
    let interrupts = match Interrupts::parse(&madt) {
        Ok(interrupts) => interrupts,
        Err(reason) => {
            warn!("acpi: madt unusable: {reason}");
            return None;
        }
    };

    let mut io_apics = interrupts.io_apics();
    let mut next_io_apic = core::iter::Iterator::next(&mut io_apics);
    while let Some(io_apic) = next_io_apic {
        next_io_apic = core::iter::Iterator::next(&mut io_apics);
        let id = io_apic.id();
        let address = io_apic.address();
        let gsi_base = io_apic.gsi_base();
        debug!(
            "acpi: io apic {} at {:#x}, gsi base {}",
            id, address, gsi_base
        );
    }
    for source in 0..16 {
        if let Some((gsi, flags)) = interrupts.override_for(source) {
            debug!("acpi: irq {source} is routed to gsi {gsi}, flags {flags:#x}");
        }
    }
    (interrupts.io_apic_count() != 0).then_some(interrupts)
}

/// The ACPI-only firmware reader and the parsed root pointer, or `None` with a
/// stated reason. Shared by every table walk (D-45).
pub(crate) fn parse_root(rsdp: u64) -> Option<(FirmwareMemory, Rsdp)> {
    let Some(firmware) = mm::firmware_memory() else {
        warn!("acpi: no reader for firmware memory");
        return None;
    };
    let Some(root) = read_root_pointer(firmware, rsdp) else {
        warn!("acpi: the root pointer at {rsdp:#x} is not readable acpi storage");
        return None;
    };
    match Rsdp::parse(&root) {
        Ok(root) => Some((firmware, root)),
        Err(reason) => {
            warn!("acpi: root pointer unusable: {reason}");
            None
        }
    }
}

/// The first table with `signature`, found by walking whichever root table the
/// RSDP offers. The caller names the table it wanted when this returns `None`.
pub(crate) fn find_table(
    firmware: FirmwareMemory,
    root: Rsdp,
    signature: [u8; 4],
) -> Option<Vec<u8>> {
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

    let mut pointers = pointers;
    let mut next_pointer = core::iter::Iterator::next(&mut pointers);
    while let Some(entry) = next_pointer {
        next_pointer = core::iter::Iterator::next(&mut pointers);
        let Some(table) = read_table(firmware, entry) else {
            continue;
        };
        if let Ok(header) = Sdt::parse(&table)
            && header.signature() == signature
        {
            return Some(table);
        }
    }
    None
}

/// A whole table at a physical address, read in two steps.
///
/// The header must be read before the length is known, and the length is what
/// says how much to read — so a header is fetched first and then the whole
/// table, which is also what lets `Sdt::parse` checksum every byte.
fn read_table(firmware: FirmwareMemory, address: u64) -> Option<Vec<u8>> {
    let mut header = [0; Sdt::LEN];
    if !FirmwareMemory::read(&firmware, address, &mut header) {
        return None;
    }
    let length = Sdt::parse_length(&header, MAX_FIRMWARE_TABLE_BYTES).ok()?;
    let mut table = zeroed(length)?;
    FirmwareMemory::read(&firmware, address, &mut table).then_some(table)
}

/// Read the variable-length root pointer from its direct-map address.
fn read_root_pointer(firmware: FirmwareMemory, address: u64) -> Option<Vec<u8>> {
    let offset = mm::direct_map()?.offset();
    let physical = address.checked_sub(offset)?;
    let mut original = [0; Rsdp::V1_LEN];
    if !FirmwareMemory::read(&firmware, physical, &mut original) {
        return None;
    }
    let length = if original[15] == 2 {
        let mut prefix = [0; 24];
        if !FirmwareMemory::read(&firmware, physical, &mut prefix) {
            return None;
        }
        Rsdp::parse_length(&prefix, MAX_FIRMWARE_TABLE_BYTES).ok()?
    } else {
        Rsdp::parse_length(&original, MAX_FIRMWARE_TABLE_BYTES).ok()?
    };
    let mut bytes = zeroed(length)?;
    FirmwareMemory::read(&firmware, physical, &mut bytes).then_some(bytes)
}

/// Allocate a zeroed firmware buffer without invoking the allocation handler.
fn zeroed(length: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).ok()?;
    bytes.resize(length, 0);
    Some(bytes)
}
