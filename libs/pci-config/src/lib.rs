//! Configuration-space addressing, BAR decoding and the class codes this
//! kernel looks for.
//!
//! The arithmetic of talking to PCI, where `cargo test` and a mutation can
//! reach it. `wost` keeps the two ports and the `in`/`out` instructions, and
//! nothing else.
//!
//! **CF8/CFC rather than ECAM**, and the reason is not preference. ECAM needs
//! an MCFG table, and QEMU's i440fx publishes none while VMware emulates a
//! 440BX — so the mechanism guaranteed present on *both* boot targets is the
//! one from 1992. It reaches everything a read-only driver needs, and an ECAM
//! path later is purely additive (`012` §2).
//!
//! Three things here are worth more than their size:
//!
//! - **The offset's low two bits must be zero.** Configuration space is read a
//!   dword at a time; a misaligned offset does not fault, it silently reads the
//!   dword containing it and the caller shifts the wrong bytes out.
//! - **A BAR's size is learned by writing all ones and reading back**, and the
//!   answer is the two's complement of the mask — *in the register's own
//!   width*. Getting it wrong maps too little, which fails at the far end of
//!   the register file long after the mapping, or too much, which overlaps the
//!   next device.
//! - **A 64-bit BAR occupies two slots**, so a walk that steps one at a time
//!   reads the high half as a second base address and finds a device at a
//!   nonsense address.
//!
//! # Examples
//!
//! ```
//! use pci_config::{Address, header};
//!
//! // Bus 0, device 2, function 0, the vendor and device word.
//! let at = Address::new(0, 2, 0, header::VENDOR).expect("a real address");
//! assert_eq!(at.bits(), 0x8000_1000);
//!
//! // The enable bit is what makes CF8 a configuration cycle rather than a
//! // write to whatever else lives at that port.
//! assert!(at.bits() & (1 << 31) != 0);
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Buses a configuration address can name.
pub const BUSES: u8 = u8::MAX;

/// Devices on one bus.
pub const DEVICES: u8 = 32;

/// Functions in one device.
pub const FUNCTIONS: u8 = 8;

/// Bytes of the configuration header this addressing reaches.
///
/// The original mechanism sees 256; the extended space ECAM adds is not
/// reachable through CF8/CFC at all, and a driver that needs it needs ECAM.
pub const HEADER_BYTES: u16 = 256;

/// Byte offsets in the configuration header.
///
/// Type 0 (a device) and type 1 (a bridge) share everything up to `0x0F`, and
/// the header type at `0x0E` is what says which the rest is.
pub mod header {
    /// Vendor id, and device id in the high half. `0xFFFF` means nothing is
    /// there — the bus returns all ones for an absent function.
    pub const VENDOR: u16 = 0x00;
    /// Command register. Bit 1 is memory space, bit 2 is I/O space, and **bit
    /// 2 is bus master**.
    pub const COMMAND: u16 = 0x04;
    /// Revision in the low byte, then prog-if, subclass and class.
    pub const CLASS: u16 = 0x08;
    /// Cache line size, latency, header type, BIST.
    pub const HEADER_TYPE: u16 = 0x0C;
    /// The first base address register.
    pub const BAR0: u16 = 0x10;
    /// A bridge's primary, secondary and subordinate bus numbers.
    pub const BUS_NUMBERS: u16 = 0x18;
}

/// Command-register bits this kernel writes.
pub mod command {
    /// Respond to memory-space accesses.
    pub const MEMORY_SPACE: u16 = 1 << 1;
    /// **Bus master.** Without it the controller cannot DMA, queues never
    /// advance, and *there is no error anywhere to read* (`012` §2). It is the
    /// single most expensive bit to forget in this whole subsystem.
    pub const BUS_MASTER: u16 = 1 << 2;
}

/// A configuration-space address, ready for the address port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address(u32);

impl Address {
    /// Bit 31: this is a configuration cycle and not something else.
    const ENABLE: u32 = 1 << 31;

    /// Build an address, or refuse one that cannot be encoded.
    ///
    /// # Errors
    ///
    /// `None` for a device above 31, a function above 7, an offset at or past
    /// [`HEADER_BYTES`], or an offset that is not a multiple of four — the
    /// last because configuration space is read a dword at a time and a
    /// misaligned offset does not fault, it quietly reads the dword containing
    /// it.
    pub const fn new(bus: u8, device: u8, function: u8, offset: u16) -> Option<Self> {
        if device >= DEVICES || function >= FUNCTIONS {
            return None;
        }
        if offset >= HEADER_BYTES || !offset.is_multiple_of(4) {
            return None;
        }
        Some(Self(
            Self::ENABLE
                | ((bus as u32) << 16)
                | ((device as u32) << 11)
                | ((function as u32) << 8)
                | offset as u32,
        ))
    }

    /// The word to write to the address port.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The same function at a different offset.
    ///
    /// The common move — walk a header — expressed so the bus, device and
    /// function are not re-encoded and cannot be re-encoded differently.
    pub const fn at(self, offset: u16) -> Option<Self> {
        if offset >= HEADER_BYTES || !offset.is_multiple_of(4) {
            return None;
        }
        Some(Self((self.0 & !0xFF) | offset as u32))
    }

    /// Which bus this names.
    pub const fn bus(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// Which device on that bus.
    pub const fn device(self) -> u8 {
        ((self.0 >> 11) & 0x1F) as u8
    }

    /// Which function of that device.
    pub const fn function(self) -> u8 {
        ((self.0 >> 8) & 0x07) as u8
    }

    /// Which offset within its header.
    pub const fn offset(self) -> u16 {
        (self.0 & 0xFF) as u16
    }
}

/// What a function says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Class {
    /// The broad kind: `0x01` is mass storage, `0x06` a bridge.
    pub class: u8,
    /// Within that kind: `0x08` under mass storage is a non-volatile memory
    /// controller.
    pub subclass: u8,
    /// The programming interface: `0x02` under `01:08` is NVM Express.
    pub interface: u8,
    /// Silicon revision, which decides nothing here and is read because it
    /// shares the word.
    pub revision: u8,
}

impl Class {
    /// Decode the class word at [`header::CLASS`].
    pub const fn from_bits(bits: u32) -> Self {
        Self {
            revision: bits as u8,
            interface: (bits >> 8) as u8,
            subclass: (bits >> 16) as u8,
            class: (bits >> 24) as u8,
        }
    }

    /// Whether this is an NVM Express controller.
    ///
    /// All three bytes, not two: `01:08` is "non-volatile memory controller"
    /// and `01:08:01` is NVMHCI, a different and older interface this driver
    /// would misprogram.
    pub const fn is_nvme(self) -> bool {
        self.class == 0x01 && self.subclass == 0x08 && self.interface == 0x02
    }

    /// Whether this is a PCI-to-PCI bridge, and so a bus to descend into.
    ///
    /// **Enumeration must recurse.** VMware places functions behind
    /// `pciBridge0`–`pciBridge7`, so a scan of bus zero alone may never find
    /// the controller (`012` §2).
    pub const fn is_bridge(self) -> bool {
        self.class == 0x06 && self.subclass == 0x04
    }
}

/// What the header-type byte says about a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderType(u8);

impl HeaderType {
    /// Decode the byte at [`header::HEADER_TYPE`] + 2.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Whether the device has functions past zero.
    ///
    /// Bit 7. A walk that ignores it and probes all eight anyway is not wrong,
    /// only slower; a walk that *reads* it and gets the bit wrong misses every
    /// function of a multi-function device.
    pub const fn is_multifunction(self) -> bool {
        self.0 & 0x80 != 0
    }

    /// The layout of the rest of the header: 0 a device, 1 a bridge.
    pub const fn layout(self) -> u8 {
        self.0 & 0x7F
    }

    /// Whether the rest of the header is a bridge's.
    pub const fn is_bridge(self) -> bool {
        self.layout() == 1
    }
}

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
    /// Bits 2:1 == 0b10: the address is 64 bits and takes the next slot too.
    const WIDE: u32 = 0b100;
    /// Bit 3.
    const PREFETCHABLE: u32 = 0b1000;

    /// Decode one slot, given the slot after it in case this one is wide.
    ///
    /// `high` is read unconditionally by the caller because it must be read
    /// *before* this can say whether it was needed, and reading it costs one
    /// configuration cycle.
    pub const fn decode(low: u32, high: u32) -> Self {
        if low == 0 {
            return Self::Absent;
        }
        if low & Self::IS_IO != 0 {
            return Self::Io {
                port: (low & !0x3) as u16,
            };
        }
        let base = (low & !0xF) as u64;
        if low & 0b110 == Self::WIDE {
            Self::Memory {
                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: low & Self::PREFETCHABLE != 0,
            }
        } else {
            Self::Memory {
                address: base,
                wide: false,
                prefetchable: low & Self::PREFETCHABLE != 0,
            }
        }
    }

    /// How many register slots this occupies.
    ///
    /// One, except a 64-bit memory window, which occupies two. A walk that
    /// steps one at a time over a wide register reads its high half as a
    /// second base address and believes there is a device at a nonsense
    /// address.
    pub const fn slots(self) -> usize {
        match self {
            Self::Memory { wide: true, .. } => 2,
            _ => 1,
        }
    }
}

/// The size of a **memory** window, from the all-ones probe.
///
/// The protocol is: write `0xFFFF_FFFF` to the register, read it back, restore
/// what was there. What comes back has the address bits the device does not
/// decode set to zero, so the size is the two's complement of the mask.
///
/// `wide` says whether `high` is part of the same register or the next one.
/// It is a parameter rather than a convention because the first version of
/// this function took "pass all ones for a narrow register", and the I/O case
/// then complemented a thirty-two-bit mask in sixty-four bits and answered
/// eighteen quintillion (D-168). A width you must remember to pass correctly
/// is a width that will be passed wrongly.
///
/// # Errors
///
/// `None` when the mask decodes nothing — an absent register, or a device that
/// answered all zeros, both of which mean "there is no window here" rather
/// than "the window is four gigabytes".
///
/// # Examples
///
/// ```
/// use pci_config::memory_window_size;
///
/// // A 16 KiB window: the low fourteen address bits are not decoded.
/// assert_eq!(memory_window_size(0xFFFF_C000, 0, false), Some(0x4000));
///
/// // A 64-bit window of four gigabytes, where the high half carries it.
/// assert_eq!(memory_window_size(0, 0xFFFF_FFFF, true), Some(0x1_0000_0000));
/// ```
pub const fn memory_window_size(low: u32, high: u32, wide: bool) -> Option<u64> {
    if wide {
        let mask = ((low & !0xF) as u64) | ((high as u64) << 32);
        if mask == 0 {
            return None;
        }
        let size = (!mask).wrapping_add(1);
        if size == 0 { None } else { Some(size) }
    } else {
        let mask = low & !0xF;
        if mask == 0 {
            return None;
        }
        Some((!mask).wrapping_add(1) as u64)
    }
}

/// The size of an **I/O** window, from the same probe.
///
/// Thirty-two bits throughout, and separate from the memory form for exactly
/// that reason: an I/O window is at most sixty-five thousand ports, and
/// complementing its mask in sixty-four bits gives a number no machine has.
///
/// # Errors
///
/// `None` when the mask decodes nothing, or when the size will not fit the
/// port space it claims to describe.
///
/// # Examples
///
/// ```
/// use pci_config::io_window_size;
///
/// // Thirty-two ports. The low two bits are the register's own.
/// assert_eq!(io_window_size(0xFFFF_FFE1), Some(32));
/// ```
pub const fn io_window_size(mask: u32) -> Option<u16> {
    let mask = mask & !0x3;
    if mask == 0 {
        return None;
    }
    let size = (!mask).wrapping_add(1);
    // A run that does not fit the sixteen-bit port space is a device
    // misreporting, and claiming it would be claiming most of I/O space.
    if size == 0 || size > u16::MAX as u32 {
        return None;
    }
    Some(size as u16)
}

/// Whether a vendor word means "nothing is here".
///
/// An absent function does not fail a configuration read; the bus returns all
/// ones, so `0xFFFF` is the answer and not an error. A driver that treats it
/// as a vendor id enumerates 256 buses of imaginary devices.
pub const fn is_absent(vendor: u16) -> bool {
    vendor == 0xFFFF
}

/// A bridge's secondary bus number, out of the word at [`header::BUS_NUMBERS`].
///
/// The bus behind it, which is the number a recursive enumeration descends to.
pub const fn secondary_bus(bits: u32) -> u8 {
    (bits >> 8) as u8
}

/// A bridge's subordinate bus number: the highest bus behind it.
pub const fn subordinate_bus(bits: u32) -> u8 {
    (bits >> 16) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address is the bus, device, function and offset the manual gives.
    ///
    /// Transcribed from the specification's field positions rather than
    /// derived, and the values are **pairwise distinct** so no two fields can
    /// be swapped with this still passing (D-136).
    #[test]
    fn an_address_puts_every_field_where_the_manual_says() {
        let at = Address::new(0x12, 0x1A, 0x05, 0x3C).expect("a valid address");

        assert_eq!(at.bits() >> 31, 1, "the enable bit");
        assert_eq!(at.bus(), 0x12);
        assert_eq!(at.device(), 0x1A);
        assert_eq!(at.function(), 0x05);
        assert_eq!(at.offset(), 0x3C);
        assert_eq!(at.bits(), 0x8012_D53C);
    }

    /// Every encodable address round-trips through its own fields.
    ///
    /// Exhaustive over devices, functions and offsets, and a spread of buses:
    /// a field that overlapped its neighbour would survive any single example
    /// and cannot survive all of them.
    #[test]
    fn every_address_round_trips_through_its_fields() {
        for bus in [0u8, 1, 0x7F, 0x80, 0xFE, 0xFF] {
            for device in 0..DEVICES {
                for function in 0..FUNCTIONS {
                    for offset in (0..HEADER_BYTES).step_by(4) {
                        let at =
                            Address::new(bus, device, function, offset).expect("all encodable");
                        assert_eq!(
                            (at.bus(), at.device(), at.function(), at.offset()),
                            (bus, device, function, offset)
                        );
                    }
                }
            }
        }
    }

    /// An address that cannot be encoded is refused, not truncated.
    ///
    /// The misaligned offset is the one that matters: it does not fault, it
    /// reads the dword containing it, and the caller shifts the wrong bytes
    /// out of a plausible-looking value.
    #[test]
    fn an_address_that_cannot_be_encoded_is_refused() {
        assert!(Address::new(0, DEVICES, 0, 0).is_none(), "device 32");
        assert!(Address::new(0, 0, FUNCTIONS, 0).is_none(), "function 8");
        assert!(
            Address::new(0, 0, 0, HEADER_BYTES).is_none(),
            "past the header"
        );
        for misaligned in [1u16, 2, 3, 0x0D, 0xFF] {
            assert!(
                Address::new(0, 0, 0, misaligned).is_none(),
                "offset {misaligned:#x} is not a dword boundary"
            );
        }

        // And `at` refuses the same things, because it is the same rule from
        // the other direction — a walk that re-offsets is where a misaligned
        // value would actually come from.
        let at = Address::new(0, 1, 0, 0).expect("valid");
        assert!(at.at(0x0D).is_none());
        assert!(at.at(HEADER_BYTES).is_none());
        assert_eq!(at.at(0x10).expect("valid").offset(), 0x10);
        assert_eq!(at.at(0x10).expect("valid").device(), 1, "the rest is kept");
    }

    /// The class word is read as four separate bytes.
    ///
    /// Pairwise distinct on purpose: `01:08:02:03` would let class and
    /// subclass be swapped, so every byte here differs from every other.
    #[test]
    fn the_class_word_is_read_as_four_separate_bytes() {
        let class = Class::from_bits(0x0C_03_30_A1);
        assert_eq!(class.class, 0x0C);
        assert_eq!(class.subclass, 0x03);
        assert_eq!(class.interface, 0x30);
        assert_eq!(class.revision, 0xA1);
    }

    /// NVM Express is all three bytes, and its neighbours are not it.
    ///
    /// `01:08:01` is NVMHCI — an older interface a driver written for `02`
    /// would misprogram — so a two-byte test would accept the wrong device
    /// and the failure would be a controller that never answers.
    #[test]
    fn nvme_is_all_three_bytes_and_its_neighbours_are_not() {
        assert!(Class::from_bits(0x01_08_02_00).is_nvme());

        for near in [
            0x01_08_01_00u32, // NVMHCI
            0x01_08_03_00,    // a prog-if this driver does not know
            0x01_06_02_00,    // SATA
            0x02_08_02_00,    // a network controller with the same tail
        ] {
            assert!(
                !Class::from_bits(near).is_nvme(),
                "{near:#010x} is not NVM Express"
            );
        }
    }

    /// A bridge is recognised, because enumeration has to descend through one.
    #[test]
    fn a_bridge_is_recognised_and_nothing_else_is() {
        assert!(Class::from_bits(0x06_04_00_00).is_bridge());
        assert!(
            !Class::from_bits(0x06_00_00_00).is_bridge(),
            "a host bridge"
        );
        assert!(
            !Class::from_bits(0x06_01_00_00).is_bridge(),
            "an ISA bridge"
        );
        assert!(!Class::from_bits(0x01_08_02_00).is_bridge());
    }

    /// The header-type byte's two halves are read separately.
    #[test]
    fn the_header_type_byte_is_a_layout_and_a_flag() {
        let plain = HeaderType::from_bits(0x00);
        assert!(!plain.is_multifunction());
        assert_eq!(plain.layout(), 0);
        assert!(!plain.is_bridge());

        let multi_bridge = HeaderType::from_bits(0x81);
        assert!(multi_bridge.is_multifunction());
        assert_eq!(multi_bridge.layout(), 1);
        assert!(multi_bridge.is_bridge());

        // The flag must not leak into the layout, which is what a missing mask
        // would do: `0x81 & 0x7F` is 1, `0x81` is 129.
        let multi_plain = HeaderType::from_bits(0x80);
        assert!(multi_plain.is_multifunction());
        assert_eq!(multi_plain.layout(), 0);
        assert!(!multi_plain.is_bridge());
    }

    /// A base address register decodes into what it actually describes.
    #[test]
    fn a_base_address_register_decodes_to_its_window() {
        assert_eq!(Bar::decode(0, 0), Bar::Absent);

        assert_eq!(Bar::decode(0xC001, 0), Bar::Io { port: 0xC000 });

        assert_eq!(
            Bar::decode(0xFEBF_0000, 0),
            Bar::Memory {
                address: 0xFEBF_0000,
                wide: false,
                prefetchable: false,
            }
        );

        assert_eq!(
            Bar::decode(0xFEBF_000C, 0x0000_0001),
            Bar::Memory {
                address: 0x1_FEBF_0000,
                wide: true,
                prefetchable: true,
            }
        );
    }

    /// A 64-bit register occupies two slots and a 32-bit one does not.
    ///
    /// The whole reason this function exists: a walk that steps one at a time
    /// over a wide register reads its high half as a second base address, and
    /// believes there is a window at whatever the top thirty-two bits were.
    #[test]
    fn a_wide_register_occupies_the_slot_after_it() {
        assert_eq!(Bar::decode(0xFEBF_000C, 0).slots(), 2);
        assert_eq!(Bar::decode(0xFEBF_0000, 0).slots(), 1);
        assert_eq!(Bar::decode(0xC001, 0).slots(), 1);
        assert_eq!(Bar::decode(0, 0).slots(), 1);
    }

    /// A memory window's size is the two's complement of what came back.
    ///
    /// Every power of two from a page to two gigabytes, because the failure is
    /// off by a factor of two and a single example cannot see it.
    #[test]
    fn a_memory_window_size_is_the_complement_of_the_probe() {
        for bits in 4..31u32 {
            let size = 1u32 << bits;
            let mask = !(size - 1);
            assert_eq!(
                memory_window_size(mask, 0, false),
                Some(u64::from(size)),
                "a {size}-byte window"
            );
        }

        // A 64-bit window past four gigabytes, where the high half carries it.
        assert_eq!(
            memory_window_size(0, 0xFFFF_FFFF, true),
            Some(0x1_0000_0000)
        );
        assert_eq!(
            memory_window_size(0xFFF0_0000, 0xFFFF_FFFF, true),
            Some(0x10_0000)
        );
    }

    /// A narrow register's mask is complemented in thirty-two bits.
    ///
    /// The bug this function was split to prevent: complementing a
    /// thirty-two-bit mask in sixty-four bits sets the whole high half and
    /// answers eighteen quintillion, which a caller then tries to map
    /// (D-168).
    #[test]
    fn a_narrow_window_is_not_complemented_in_a_wide_word() {
        // The same mask read both ways: as a narrow register it is 16 KiB, as
        // the low half of a wide one it is very much not.
        assert_eq!(memory_window_size(0xFFFF_C000, 0, false), Some(0x4000));
        assert!(memory_window_size(0xFFFF_C000, 0, true).unwrap() > u64::from(u32::MAX));
    }

    /// An I/O window's size is a port count, not an address range.
    #[test]
    fn an_io_window_size_is_a_port_count() {
        for bits in 2..16u32 {
            let size = 1u32 << bits;
            let mask = !(size - 1);
            assert_eq!(
                io_window_size(mask),
                u16::try_from(size).ok(),
                "a {size}-port window"
            );
        }
        // The register's own low bits are not address bits.
        assert_eq!(io_window_size(0xFFFF_FFE1), Some(32));
    }

    /// A probe that decoded nothing is refused rather than read as huge.
    ///
    /// All zeros means "there is no window here". Read as a mask it is a
    /// four-gigabyte window at address zero, which a kernel would then map.
    #[test]
    fn a_probe_that_decoded_nothing_is_refused() {
        assert_eq!(memory_window_size(0, 0, false), None);
        assert_eq!(memory_window_size(0, 0, true), None);
        assert_eq!(io_window_size(0), None);
        // Only the low bits set, which are the register's own and not address
        // bits at all.
        assert_eq!(memory_window_size(0x0000_000F, 0, false), None);
        assert_eq!(io_window_size(0x0000_0003), None);
        // And a window larger than the port space is a device misreporting.
        assert_eq!(io_window_size(0xFFFF_0000), None);
    }

    /// An absent function is all ones, and that is not a vendor.
    #[test]
    fn an_absent_function_reads_as_all_ones() {
        assert!(is_absent(0xFFFF));
        assert!(!is_absent(0x8086), "Intel");
        assert!(!is_absent(0x1B36), "QEMU");
        assert!(!is_absent(0x15AD), "VMware");
        assert!(!is_absent(0x0000), "a vendor of zero is odd, not absent");
    }

    /// A bridge's bus numbers are three separate bytes.
    #[test]
    fn a_bridges_bus_numbers_are_read_separately() {
        // Primary 0x11, secondary 0x22, subordinate 0x33: pairwise distinct,
        // so a shift off by eight cannot pass.
        let bits = 0x0044_3322u32 << 8 | 0x11;
        assert_eq!(secondary_bus(bits), 0x22);
        assert_eq!(subordinate_bus(bits), 0x33);
    }
}
