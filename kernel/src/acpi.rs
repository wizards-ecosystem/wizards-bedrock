use alloc::vec::Vec;
use core::fmt;

use acpi_tables::{MadtInterrupts, Rsdp, Sdt, SdtPointers};
use wost::boot::{DirectMap, MemoryMap};
use wost::mm::{self, EarlyFirmwareMemory};

use crate::{debug, warn};

const MAX_IO_APICS: usize = 16;

// A corrupt length must not become an unbounded allocation; real tables are far smaller.
const MAX_FIRMWARE_TABLE_BYTES: usize = 1 << 20;

const ROOT_TABLE_BYTES: usize = Sdt::LEN + 256 * size_of::<u64>();

const ROOT_POINTER_PREFIX: usize = 24;

const ROOT_POINTER_REVISION: usize = 15;

const ISA_IRQS: u8 = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TableError {
    NoReader,
    RootPointer,
    RootTable,
    Candidate,
    Missing,
    Duplicate,
}

impl fmt::Display for TableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoReader => formatter.write_str("no reader for firmware memory"),
            Self::RootPointer => formatter.write_str("the ACPI root pointer is unreadable"),
            Self::RootTable => formatter.write_str("the ACPI root table is unreadable"),
            Self::Candidate => formatter.write_str("the requested ACPI table is malformed"),
            Self::Missing => formatter.write_str("the requested ACPI table is missing"),
            Self::Duplicate => formatter.write_str("the requested ACPI table is duplicated"),
        }
    }
}

impl core::error::Error for TableError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Multiplicity {
    First,
    Unique,
}

pub(crate) fn find_unique_early<const N: usize>(
    map: &MemoryMap,
    direct: DirectMap,
    rsdp: u64,
    signature: [u8; 4],
) -> Result<[u8; N], TableError> {
    let firmware = EarlyFirmwareMemory::new(map, direct);
    let rsdp = rsdp
        .checked_sub(direct.offset())
        .ok_or(TableError::RootPointer)?;
    find(
        |address, into| firmware.read(address, into),
        rsdp,
        signature,
        Multiplicity::Unique,
        |length| (length == N).then_some([0u8; N]),
    )
}

pub(crate) fn find_table(rsdp: u64, signature: [u8; 4]) -> Result<Vec<u8>, TableError> {
    find_table_with_multiplicity(rsdp, signature, Multiplicity::First)
}

pub(crate) fn find_unique_table(rsdp: u64, signature: [u8; 4]) -> Result<Vec<u8>, TableError> {
    find_table_with_multiplicity(rsdp, signature, Multiplicity::Unique)
}

fn find_table_with_multiplicity(
    rsdp: u64,
    signature: [u8; 4],
    multiplicity: Multiplicity,
) -> Result<Vec<u8>, TableError> {
    let (Some(firmware), Some(direct)) = (mm::firmware_memory(), mm::direct_map()) else {
        return Err(TableError::NoReader);
    };
    let rsdp = rsdp
        .checked_sub(direct.offset())
        .ok_or(TableError::RootPointer)?;
    find(
        |address, into| firmware.read(address, into),
        rsdp,
        signature,
        multiplicity,
        |length| {
            (length <= MAX_FIRMWARE_TABLE_BYTES)
                .then(|| zeroed(length))
                .flatten()
        },
    )
}

// Unreadable entries, null ones included, are skipped; a match must be whole and checksummed.
fn find<T: AsMut<[u8]> + AsRef<[u8]>>(
    read: impl Fn(u64, &mut [u8]) -> bool,
    rsdp: u64,
    signature: [u8; 4],
    multiplicity: Multiplicity,
    buffer: impl Fn(usize) -> Option<T>,
) -> Result<T, TableError> {
    let mut root_pointer = [0u8; Rsdp::V2_LEN];
    if !read(rsdp, &mut root_pointer[..Rsdp::V1_LEN]) {
        return Err(TableError::RootPointer);
    }
    if root_pointer[ROOT_POINTER_REVISION] >= Rsdp::EXTENDED_REVISION
        && !read(rsdp, &mut root_pointer[..ROOT_POINTER_PREFIX])
    {
        return Err(TableError::RootPointer);
    }
    let length =
        Rsdp::parse_length(&root_pointer, Rsdp::V2_LEN).map_err(|_| TableError::RootPointer)?;
    if !read(rsdp, &mut root_pointer[..length]) {
        return Err(TableError::RootPointer);
    }
    let root = Rsdp::parse(&root_pointer[..length]).map_err(|_| TableError::RootPointer)?;
    let (address, extended) = match (root.xsdt(), root.rsdt()) {
        (Some(xsdt), _) => (xsdt, true),
        (None, Some(rsdt)) => (rsdt, false),
        (None, None) => return Err(TableError::RootPointer),
    };

    let mut root_table = [0u8; ROOT_TABLE_BYTES];
    if !read(address, &mut root_table[..Sdt::LEN]) {
        return Err(TableError::RootTable);
    }
    let root_length =
        Sdt::parse_length(&root_table, ROOT_TABLE_BYTES).map_err(|_| TableError::RootTable)?;
    if !read(address, &mut root_table[..root_length]) {
        return Err(TableError::RootTable);
    }
    let pointers = if extended {
        SdtPointers::xsdt(&root_table[..root_length])
    } else {
        SdtPointers::rsdt(&root_table[..root_length])
    }
    .map_err(|_| TableError::RootTable)?;

    let mut found = None;
    for address in pointers {
        let mut header = [0u8; Sdt::LEN];
        if !read(address, &mut header) || header[..4] != signature {
            continue;
        }
        let length = Sdt::parse_length(&header, usize::MAX).map_err(|_| TableError::Candidate)?;
        let mut table = buffer(length).ok_or(TableError::Candidate)?;
        if !read(address, table.as_mut())
            || !matches!(Sdt::parse(table.as_ref()), Ok(parsed) if parsed.signature() == signature)
        {
            return Err(TableError::Candidate);
        }
        if multiplicity == Multiplicity::First {
            return Ok(table);
        }
        if found.replace(table).is_some() {
            return Err(TableError::Duplicate);
        }
    }
    found.ok_or(TableError::Missing)
}

pub(crate) type Interrupts = MadtInterrupts<MAX_IO_APICS>;

// `rsdp` is the bootloader's virtual address, which base revision 4 puts in the direct map.
pub(crate) fn interrupts(rsdp: u64) -> Option<Interrupts> {
    let madt = match find_table(rsdp, Sdt::MADT) {
        Ok(madt) => madt,
        Err(reason) => {
            warn!("acpi: no usable madt: {reason}");
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

    for io_apic in interrupts.io_apics() {
        debug!(
            "acpi: io apic {} at {:#x}, gsi base {}",
            io_apic.id(),
            io_apic.address(),
            io_apic.gsi_base()
        );
    }
    for source in 0..ISA_IRQS {
        if let Some((gsi, flags)) = interrupts.override_for(source) {
            debug!("acpi: irq {source} is routed to gsi {gsi}, flags {flags:#x}");
        }
    }
    (interrupts.io_apic_count() != 0).then_some(interrupts)
}

// Fallible, so a corrupt length never reaches the allocation-failure handler.
fn zeroed(length: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).ok()?;
    bytes.resize(length, 0);
    Some(bytes)
}
