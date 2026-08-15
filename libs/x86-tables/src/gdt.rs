//! Global descriptor table entries.
//!
//! In 64-bit mode a code or data descriptor carries almost nothing: base and
//! limit are ignored, and what remains is a handful of permission bits. System
//! descriptors — the task-state segment is the only one this kernel uses — are
//! twice as wide and do carry a base address.

/// The bits this crate sets. Ones it deliberately leaves clear — the accessed
/// bit, the privilege level, granularity — are named in the tests that assert
/// they stay clear.
mod bit {
    /// Readable for a code segment, writable for a data segment.
    pub(super) const ACCESS: u64 = 1 << 41;
    /// Executable — the bit that makes a descriptor a code segment.
    pub(super) const EXECUTABLE: u64 = 1 << 43;
    /// Code or data rather than a system descriptor.
    pub(super) const CODE_OR_DATA: u64 = 1 << 44;
    /// The descriptor is usable.
    pub(super) const PRESENT: u64 = 1 << 47;
    /// 64-bit code. Must be clear for data, and pairs with a clear default-size
    /// bit.
    pub(super) const LONG_MODE: u64 = 1 << 53;
}

/// Descriptor type for an available 64-bit task-state segment.
const TASK_STATE_TYPE: u64 = 0x9 << 40;

/// A code or data segment descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor(u64);

impl Descriptor {
    /// The mandatory unusable entry zero.
    pub const NULL: Self = Self(0);

    /// Ring 0 code: present, executable, readable, 64-bit.
    pub const KERNEL_CODE: Self =
        Self(bit::PRESENT | bit::CODE_OR_DATA | bit::EXECUTABLE | bit::ACCESS | bit::LONG_MODE);

    /// Ring 0 data: present, writable.
    ///
    /// `SS` must name one of these even in 64-bit mode, where `iretq` reloads it
    /// on every return from an exception.
    pub const KERNEL_DATA: Self = Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS);

    /// Build a descriptor from raw bits.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The eight bytes the CPU reads.
    pub const fn bits(self) -> u64 {
        self.0
    }
}

/// A system descriptor: sixteen bytes, occupying two table entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemDescriptor {
    /// The entry at the descriptor's own index.
    pub low: u64,
    /// The entry immediately after it, holding the top half of the base address.
    pub high: u64,
}

impl SystemDescriptor {
    /// Describe a task-state segment of `size` bytes at `base`.
    ///
    /// The limit is one less than the size, which is what makes an offset equal
    /// to the size — the value `iomap_base` carries to mean "no I/O bitmap" —
    /// land outside the segment. A task-state segment cannot exceed a 16-bit
    /// limit even with a full I/O bitmap, so the limit's high nibble is always
    /// zero and is not encoded.
    ///
    /// # Panics
    ///
    /// Panics if `size` is zero.
    pub const fn task_state(base: u64, size: u16) -> Self {
        assert!(size > 0, "a task-state segment cannot be empty");
        let limit = (size - 1) as u64;

        let low = limit
            | (base & 0x00FF_FFFF) << 16
            | TASK_STATE_TYPE
            | bit::PRESENT
            | (base & 0xFF00_0000) << 32;

        Self {
            low,
            high: base >> 32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set by the CPU on first use; a descriptor is written with it clear.
    const ACCESSED: u64 = 1 << 40;

    /// The two-bit descriptor privilege level, zero for every ring 0 entry.
    const PRIVILEGE: u64 = 0b11 << 45;

    /// The two values every x86_64 kernel arrives at, which is the point of
    /// asserting them: if a bit moves, this is where it shows.
    #[test]
    fn ring_zero_descriptors_match_the_known_encoding() {
        assert_eq!(Descriptor::KERNEL_CODE.bits(), 0x0020_9A00_0000_0000);
        assert_eq!(Descriptor::KERNEL_DATA.bits(), 0x0000_9200_0000_0000);
        assert_eq!(Descriptor::NULL.bits(), 0);
    }

    #[test]
    fn a_code_descriptor_is_not_marked_accessed() {
        assert_eq!(Descriptor::KERNEL_CODE.bits() & ACCESSED, 0);
    }

    #[test]
    fn code_and_data_differ_only_in_execute_and_long_mode() {
        let difference = Descriptor::KERNEL_CODE.bits() ^ Descriptor::KERNEL_DATA.bits();
        assert_eq!(difference, bit::EXECUTABLE | bit::LONG_MODE);
    }

    #[test]
    fn both_ring_zero_descriptors_are_present_at_privilege_zero() {
        for descriptor in [Descriptor::KERNEL_CODE, Descriptor::KERNEL_DATA] {
            assert_ne!(descriptor.bits() & bit::PRESENT, 0);
            assert_eq!(descriptor.bits() & PRIVILEGE, 0);
        }
    }

    #[test]
    fn a_descriptor_round_trips_through_its_bits() {
        let bits = Descriptor::KERNEL_CODE.bits();
        assert_eq!(Descriptor::from_bits(bits), Descriptor::KERNEL_CODE);
    }

    #[test]
    fn a_task_state_descriptor_scatters_its_base_across_four_fields() {
        let descriptor = SystemDescriptor::task_state(0xFFFF_8877_6655_4433, 104);

        assert_eq!(descriptor.low & 0xFFFF, 103);
        assert_eq!(descriptor.low >> 16 & 0xFFFF, 0x4433);
        assert_eq!(descriptor.low >> 32 & 0xFF, 0x55);
        assert_eq!(descriptor.low >> 56 & 0xFF, 0x66);
        assert_eq!(descriptor.high, 0xFFFF_8877);
    }

    #[test]
    fn a_task_state_descriptor_is_present_available_and_not_a_segment() {
        let descriptor = SystemDescriptor::task_state(0x1000, 104);

        assert_ne!(descriptor.low & bit::PRESENT, 0);
        assert_eq!(descriptor.low >> 40 & 0xF, 0x9);
        assert_eq!(descriptor.low & bit::CODE_OR_DATA, 0);
        assert_eq!(descriptor.low & PRIVILEGE, 0);
    }

    #[test]
    #[should_panic(expected = "cannot be empty")]
    fn a_task_state_descriptor_rejects_a_zero_size() {
        let _ = SystemDescriptor::task_state(0x1000, 0);
    }
}
