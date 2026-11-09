/// A base address register, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    /// A memory window.
    Memory {
        /// Where it begins.
        address: u64,
        /// Whether it took two register slots.
        wide: bool,
        /// Whether the device tolerates speculative reads. **Not** whether the
        /// kernel should map it that way: a register file is mapped
        /// uncacheable whatever this says, and mapping it write-back is a
        /// silent, hard-to-debug bug (`012` §2).
        prefetchable: bool,
    },
    /// An I/O port window.
    Io {
        /// The first port.
        port: u16,
    },
    /// The slot is unused, or is the high half of the register before it.
    Absent,
}

impl Bar {
    /// Bit 0: zero for memory, one for I/O.
    const IS_IO: u32 = 1;
    /// Bits 2:1 select the memory BAR encoding.
    const TYPE_MASK: u32 = 0b110;
    /// The obsolete memory encoding constrained below one megabyte.
    const BELOW_ONE_MEGABYTE: u32 = 0b010;
    /// Bits 2:1 == 0b10: the address is 64 bits and takes the next slot too.
    const WIDE: u32 = 0b100;
    /// Bits 2:1 == 0b11 have no defined memory BAR meaning.
    const RESERVED: u32 = 0b110;
    /// Bit 3.
    const PREFETCHABLE: u32 = 0b1000;

    /// Decode one supported slot, given the slot after it when it is wide.
    ///
    /// `high` is read unconditionally by the caller because it must be read
    /// *before* this can say whether it was needed, and reading it costs one
    /// configuration cycle.
    ///
    /// # Errors
    ///
    /// `None` for the unsupported below-one-megabyte memory encoding, the
    /// reserved memory encoding, or an I/O address outside the 16-bit port space.
    pub const fn decode(low: u32, high: u32) -> Option<Self> {
        if low == 0 {
            return Some(Self::Absent);
        }
        if low & Self::IS_IO != 0 {
            let address = low & !0x3;
            if address >> 16 != 0 {
                return None;
            }
            return Some(Self::Io {
                port: address as u16,
            });
        }
        let memory_type = low & Self::TYPE_MASK;
        if memory_type == Self::BELOW_ONE_MEGABYTE {
            return None;
        }
        if memory_type == Self::RESERVED {
            return None;
        }
        let base = (low & !0xF) as u64;
        if memory_type == Self::WIDE {
            Some(Self::Memory {
                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: low & Self::PREFETCHABLE != 0,
            })
        } else {
            Some(Self::Memory {
                address: base,
                wide: false,
                prefetchable: low & Self::PREFETCHABLE != 0,
            })
        }
    }
}

/// Register width used to interpret a probed memory-window mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryWidth {
    /// One 32-bit base-address register.
    Bits32,
    /// A low/high pair forming one 64-bit base-address register.
    Bits64,
}

/// The size of a **memory** window, from the all-ones probe.
///
/// The size is the two's complement of the returned address mask. `width`
/// keeps complementation in the register's actual width (D-168).
///
/// # Errors
///
/// `None` when the mask decodes no window.
///
/// # Examples
///
/// ```
/// use pci_config::{MemoryWidth, memory_window_size};
///
/// // A 16 KiB window: the low fourteen address bits are not decoded.
/// assert_eq!(
///     memory_window_size(0xFFFF_C000, 0, MemoryWidth::Bits32),
///     Some(0x4000),
/// );
///
/// // A 64-bit window of four gigabytes, where the high half carries it.
/// assert_eq!(
///     memory_window_size(0, 0xFFFF_FFFF, MemoryWidth::Bits64),
///     Some(0x1_0000_0000),
/// );
/// ```
pub const fn memory_window_size(low: u32, high: u32, width: MemoryWidth) -> Option<u64> {
    match width {
        MemoryWidth::Bits64 => {
            let mask = ((low & !0xF) as u64) | ((high as u64) << 32);
            if mask == 0 {
                return None;
            }
            let size = (!mask).wrapping_add(1);
            if size == 0 { None } else { Some(size) }
        }
        MemoryWidth::Bits32 => {
            let mask = low & !0xF;
            if mask == 0 {
                return None;
            }
            Some((!mask).wrapping_add(1) as u64)
        }
    }
}

/// The size of an **I/O** window, from the same probe.
///
/// The port mask is the low half of the probe: a 16-bit decoder reads back
/// zero above it.
///
/// # Errors
///
/// `None` when the mask decodes nothing.
///
pub(crate) const fn io_window_size(mask: u32) -> Option<u16> {
    let mask = register_bits::u16_halves(mask).0 & !0x3;
    if mask == 0 {
        return None;
    }
    Some((!mask).wrapping_add(1))
}
