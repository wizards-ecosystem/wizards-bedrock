#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bar {
    Memory {
        address: u64,
        wide: bool,
        prefetchable: bool,
    },
    Io {
        port: u16,
    },
    Absent,
}

impl Bar {
    // Bit 0: zero for memory, one for I/O.
    const IS_IO: u32 = 1;
    // Bits 2:1 select the memory BAR encoding.
    const TYPE_MASK: u32 = 0b110;
    // Bits 2:1 == 0b01: the obsolete encoding constrained below one megabyte.
    const BELOW_ONE_MEGABYTE: u32 = 0b010;
    // Bits 2:1 == 0b10: the address is 64 bits and takes the next slot too.
    const WIDE: u32 = 0b100;
    // Bits 2:1 == 0b11 have no defined memory BAR meaning.
    const RESERVED: u32 = 0b110;
    // Bit 3.
    const PREFETCHABLE: u32 = 0b1000;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MemoryWidth {
    Bits32,
    Bits64,
}

pub(crate) const fn memory_window_size(low: u32, high: u32, width: MemoryWidth) -> Option<u64> {
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

pub(crate) const fn io_window_size(mask: u32) -> Option<u16> {
    let mask = register_bits::u16_halves(mask).0 & !0x3;
    if mask == 0 {
        return None;
    }
    Some((!mask).wrapping_add(1))
}
