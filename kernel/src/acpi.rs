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
use wost::boot::{DirectMap, MemoryMap};
use wost::mm::EarlyFirmwareMemory;
use wost::mm::{self, FirmwareMemory};

use crate::{debug, warn};

/// I/O APIC descriptions this boot may temporarily map at once.
const MAX_IO_APICS: usize = 16;

/// A corrupt firmware length must not turn table discovery into an unbounded
/// allocation. Real ACPI tables are far smaller than this ceiling.
const MAX_FIRMWARE_TABLE_BYTES: usize = 1 << 20;

/// Maximum fixed root table extent: its header plus 256 XSDT pointers.
const EARLY_ROOT_BYTES: usize = Sdt::LEN + 256 * size_of::<u64>();

/// Why an allocation-free unique ACPI authority lookup failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EarlyTableError {
    RootPointer,
    RootTable,
    RootEntry,
    Candidate,
    Missing,
    Duplicate,
}

impl core::fmt::Display for EarlyTableError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::RootPointer => {
                core::fmt::Write::write_str(formatter, "the ACPI root pointer is unreadable")
            }
            Self::RootTable => {
                core::fmt::Write::write_str(formatter, "the ACPI root table is unreadable")
            }
            Self::RootEntry => {
                core::fmt::Write::write_str(formatter, "an ACPI root-table target is unreadable")
            }
            Self::Candidate => {
                core::fmt::Write::write_str(formatter, "the requested ACPI table is malformed")
            }
            Self::Missing => {
                core::fmt::Write::write_str(formatter, "the requested ACPI table is missing")
            }
            Self::Duplicate => {
                core::fmt::Write::write_str(formatter, "the requested ACPI table is duplicated")
            }
        }
    }
}

/// Read one unique fixed-size ACPI table without allocator state.
pub(crate) fn find_unique_early<const N: usize>(
    map: &MemoryMap,
    direct: DirectMap,
    rsdp: u64,
    signature: [u8; 4],
) -> Result<[u8; N], EarlyTableError> {
    let firmware = EarlyFirmwareMemory::new(map, direct);
    let physical = rsdp
        .checked_sub(direct.offset())
        .ok_or(EarlyTableError::RootPointer)?;
    let mut root_pointer = [0u8; Rsdp::V2_LEN];
    if !firmware.read(physical, &mut root_pointer[..Rsdp::V1_LEN]) {
        return Err(EarlyTableError::RootPointer);
    }
    let prefix = if root_pointer[15] == 2 {
        if !firmware.read(physical, &mut root_pointer[..24]) {
            return Err(EarlyTableError::RootPointer);
        }
        &root_pointer[..24]
    } else {
        &root_pointer[..Rsdp::V1_LEN]
    };
    let length =
        Rsdp::parse_length(prefix, Rsdp::V2_LEN).map_err(|_| EarlyTableError::RootPointer)?;
    if length > 24 && !firmware.read(physical, &mut root_pointer[..length]) {
        return Err(EarlyTableError::RootPointer);
    }
    let root = Rsdp::parse(&root_pointer[..length]).map_err(|_| EarlyTableError::RootPointer)?;
    let (address, extended) = match (root.xsdt(), root.rsdt()) {
        (Some(xsdt), _) => (xsdt, true),
        (None, Some(rsdt)) => (rsdt, false),
        (None, None) => return Err(EarlyTableError::RootPointer),
    };

    let mut root_header = [0u8; Sdt::LEN];
    if !firmware.read(address, &mut root_header) {
        return Err(EarlyTableError::RootTable);
    }
    let root_length = Sdt::parse_length(&root_header, EARLY_ROOT_BYTES)
        .map_err(|_| EarlyTableError::RootTable)?;
    let mut root_table = [0u8; EARLY_ROOT_BYTES];
    if !firmware.read(address, &mut root_table[..root_length]) {
        return Err(EarlyTableError::RootTable);
    }
    let mut pointers = if extended {
        SdtPointers::xsdt(&root_table[..root_length])
    } else {
        SdtPointers::rsdt(&root_table[..root_length])
    }
    .map_err(|_| EarlyTableError::RootTable)?;

    let mut found = None;
    for address in &mut pointers {
        let mut header = [0u8; Sdt::LEN];
        if !firmware.read(address, &mut header) {
            return Err(EarlyTableError::RootEntry);
        }
        if header[..4] != signature {
            continue;
        }
        let length = Sdt::parse_length(&header, N).map_err(|_| EarlyTableError::Candidate)?;
        if length != N {
            return Err(EarlyTableError::Candidate);
        }
        let mut table = [0u8; N];
        if !firmware.read(address, &mut table)
            || !matches!(Sdt::parse(&table), Ok(parsed) if parsed.signature() == signature)
        {
            return Err(EarlyTableError::Candidate);
        }
        if found.replace(table).is_some() {
            return Err(EarlyTableError::Duplicate);
        }
    }
    found.ok_or(EarlyTableError::Missing)
}

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
    find_table_with_multiplicity(firmware, root, signature, false)
}

/// The only table with `signature`, refusing duplicate authority.
pub(crate) fn find_unique_table(
    firmware: FirmwareMemory,
    root: Rsdp,
    signature: [u8; 4],
) -> Option<Vec<u8>> {
    find_table_with_multiplicity(firmware, root, signature, true)
}

fn find_table_with_multiplicity(
    firmware: FirmwareMemory,
    root: Rsdp,
    signature: [u8; 4],
    require_unique: bool,
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

    let mut found = None;
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
            if found.is_some() {
                if require_unique {
                    warn!("acpi: duplicate {signature:?} tables refuse unique authority");
                    return None;
                }
                return found;
            }
            if !require_unique {
                return Some(table);
            }
            found = Some(table);
        }
    }
    found
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
