//! Finding the interrupt controllers ACPI describes.
//!
//! Just enough of the table tree to answer two questions: where the I/O APIC's
//! registers are, and which global system interrupt a legacy IRQ actually
//! arrives on. The parsing is `acpi_tables`', host-tested against synthetic
//! and malformed tables; the bytes come from `wost::mm::read_physical`, which
//! refuses any range the frame allocator might hand out.
//!
//! Read **before** the kernel builds its own page tables, and that ordering is
//! forced. D-45 made mapping one-way — `activate` consumes the address space so
//! nothing can map into a table the CPU is already walking — so every device
//! mapping has to be known while the tables are still being built. The ACPI
//! regions are reachable through the bootloader's direct map at that point,
//! which is what makes the ordering possible at all (D-49).

use acpi_tables::{MadtEntries, MadtEntry, Rsdp, Sdt, SdtPointers};
use wost::mm;

use crate::{debug, warn};

/// Legacy IRQ numbers, which is as many overrides as can exist.
const LEGACY_IRQS: usize = 16;

/// An I/O APIC, as the MADT describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IoApic {
    /// Physical address of its register window.
    pub(crate) address: u32,
    /// The first global system interrupt it routes.
    pub(crate) gsi_base: u32,
}

/// What ACPI says about interrupt routing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Interrupts {
    io_apic: IoApic,
    /// For each legacy IRQ, the GSI and flags an override gave it.
    overrides: [Option<(u32, u16)>; LEGACY_IRQS],
}

impl Interrupts {
    /// The first I/O APIC. One is all this kernel routes through.
    pub(crate) const fn io_apic(&self) -> IoApic {
        self.io_apic
    }

    /// The global system interrupt `irq` arrives on.
    ///
    /// Identity unless firmware said otherwise. Assuming identity is the
    /// classic way to program an I/O APIC that then delivers nothing: the
    /// override table exists because firmware commonly remaps the timer, and
    /// may remap anything.
    pub(crate) fn gsi_for(&self, irq: u8) -> u32 {
        match self.overrides.get(irq as usize).copied().flatten() {
            Some((gsi, _)) => gsi,
            None => u32::from(irq),
        }
    }

    /// The MPS INTI flags for `irq`, or zero — "conforms to the bus" — if
    /// firmware gave none.
    pub(crate) fn flags_for(&self, irq: u8) -> u16 {
        match self.overrides.get(irq as usize).copied().flatten() {
            Some((_, flags)) => flags,
            None => 0,
        }
    }
}

/// Walk the ACPI tables from the root pointer and report interrupt routing.
///
/// `rsdp` is the *virtual* address the bootloader reported, which base
/// revision 4 guarantees is inside the direct map.
pub(crate) fn interrupts(rsdp: u64) -> Option<Interrupts> {
    let root = read_at_virtual(rsdp, 36)?;
    let root = match Rsdp::parse(root) {
        Ok(root) => root,
        Err(reason) => {
            warn!("acpi: root pointer unusable: {reason}");
            return None;
        }
    };

    let madt = find_madt(root)?;
    let header = Sdt::parse(madt).ok()?;

    let mut io_apic = None;
    let mut overrides = [None; LEGACY_IRQS];
    for entry in MadtEntries::new(madt, header) {
        match entry {
            MadtEntry::IoApic {
                id,
                address,
                gsi_base,
            } => {
                debug!("acpi: io apic {id} at {address:#x}, gsi base {gsi_base}");
                io_apic.get_or_insert(IoApic { address, gsi_base });
            }
            MadtEntry::InterruptSourceOverride {
                source, gsi, flags, ..
            } => {
                debug!("acpi: irq {source} is routed to gsi {gsi}, flags {flags:#x}");
                if let Some(slot) = overrides.get_mut(source as usize) {
                    *slot = Some((gsi, flags));
                }
            }
            other => debug!("acpi: {other:?}"),
        }
    }

    Some(Interrupts {
        io_apic: io_apic?,
        overrides,
    })
}

/// The MADT, found by walking whichever root table the RSDP offers.
fn find_madt(root: Rsdp) -> Option<&'static [u8]> {
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

    let root_table = read_table(address)?;
    let root_header = Sdt::parse(root_table).ok()?;
    let pointers = if extended {
        SdtPointers::xsdt(root_table, root_header)
    } else {
        SdtPointers::rsdt(root_table, root_header)
    };

    for entry in pointers {
        let Some(table) = read_table(entry) else {
            continue;
        };
        if let Ok(header) = Sdt::parse(table)
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
fn read_table(address: u64) -> Option<&'static [u8]> {
    let header = mm::read_physical(address, Sdt::LEN)?;
    let length = Sdt::parse_length(header)?;
    mm::read_physical(address, length)
}

/// Bytes at a direct-map virtual address.
fn read_at_virtual(address: u64, len: usize) -> Option<&'static [u8]> {
    let offset = mm::direct_map()?.offset();
    mm::read_physical(address.checked_sub(offset)?, len)
}
