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
    /// Descriptor privilege level three: ring 3 may load it.
    ///
    /// Both bits, because the field is two wide and ring 3 is the only
    /// non-zero level this kernel uses — there is no ring 1 or 2 here, and
    /// leaving one bit set would name a level nothing runs at.
    pub(super) const RING_THREE: u64 = 0b11 << 45;
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

    /// Ring 3 code: present, executable, readable, 64-bit, DPL 3.
    ///
    /// `sysret` and `iretq` both require the user descriptors to sit at
    /// specific indices relative to each other, which is why the order in the
    /// table is fixed rather than incidental (D-96).
    pub const USER_CODE: Self = Self(
        bit::PRESENT
            | bit::CODE_OR_DATA
            | bit::EXECUTABLE
            | bit::ACCESS
            | bit::LONG_MODE
            | bit::RING_THREE,
    );

    /// Ring 3 data: present, writable, DPL 3.
    pub const USER_DATA: Self =
        Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS | bit::RING_THREE);

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
    /// A task-state descriptor sets the type and present bits and **nothing
    /// else** in the flag byte.
    ///
    /// Bits 52–55 need explicit zero anchors: granularity would scale the
    /// 104-byte limit to `0x67FFF`, placing `iomap_base` inside the segment and
    /// making the CPU read an unintended I/O permission bitmap (D-142).
    ///
    /// SDM Vol. 3A figure 8-4: for a 64-bit TSS descriptor, bit 52 is
    /// available-for-software, 53 is reserved-zero, 54 is the default/big bit
    /// which must be clear, and 55 is granularity, which must be clear so the
    /// limit is counted in bytes.
    #[test]
    fn a_task_state_descriptor_sets_no_size_or_granularity_bits() {
        const SIZE: u16 = 104;

        let descriptor = SystemDescriptor::task_state(0x1234_5678_9ABC_DEF0, SIZE);

        // The limit is in bytes, so it reads back exactly.
        assert_eq!(descriptor.low & 0xFFFF, u64::from(SIZE - 1));

        for (bit, what) in [
            (52, "available-for-software"),
            (53, "reserved"),
            (54, "default/big"),
            (55, "granularity"),
        ] {
            assert_eq!(
                descriptor.low & (1 << bit),
                0,
                "bit {bit} ({what}) is set; a scaled limit puts iomap_base inside the segment"
            );
        }

        // And the two that must be set, so this is not satisfied by an empty
        // descriptor.
        assert_ne!(descriptor.low & bit::PRESENT, 0, "present must be set");
        assert_eq!(
            descriptor.low & (0xF << 40),
            TASK_STATE_TYPE,
            "the type nibble is an available 64-bit TSS"
        );
    }

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

    /// The user descriptors differ from the kernel ones by the privilege field
    /// and nothing else.
    ///
    /// Transcribed against the kernel pair rather than re-derived: a user code
    /// segment that also differed in, say, the long-mode bit would be a
    /// different kind of segment wearing ring 3's number, and the difference
    /// would be invisible in a boot that worked (D-96).
    #[test]
    fn the_user_descriptors_are_the_kernel_ones_at_ring_three() {
        assert_eq!(
            Descriptor::USER_CODE.bits(),
            Descriptor::KERNEL_CODE.bits() | (0b11 << 45)
        );
        assert_eq!(
            Descriptor::USER_DATA.bits(),
            Descriptor::KERNEL_DATA.bits() | (0b11 << 45)
        );
    }

    /// Privilege reads back, and only the user pair is ring 3.
    #[test]
    fn only_the_user_descriptors_are_ring_three() {
        assert_eq!(Descriptor::KERNEL_CODE.bits() >> 45 & 0b11, 0);
        assert_eq!(Descriptor::KERNEL_DATA.bits() >> 45 & 0b11, 0);
        assert_eq!(Descriptor::USER_CODE.bits() >> 45 & 0b11, 3);
        assert_eq!(Descriptor::USER_DATA.bits() >> 45 & 0b11, 3);
        assert_eq!(Descriptor::NULL.bits() >> 45 & 0b11, 0);
    }

    /// Code is executable and data is not, at both privilege levels.
    ///
    /// A data descriptor with the executable bit set is a code segment the CPU
    /// will happily jump into, and at ring 3 that is a privilege boundary.
    #[test]
    fn only_the_code_descriptors_are_executable() {
        assert_ne!(Descriptor::KERNEL_CODE.bits() & bit::EXECUTABLE, 0);
        assert_ne!(Descriptor::USER_CODE.bits() & bit::EXECUTABLE, 0);
        assert_eq!(Descriptor::KERNEL_DATA.bits() & bit::EXECUTABLE, 0);
        assert_eq!(Descriptor::USER_DATA.bits() & bit::EXECUTABLE, 0);
    }

    /// The long-mode bit is set for code and clear for data, at ring 3 too.
    ///
    /// A data descriptor with `L` set is reserved, and the manual says the
    /// result is undefined rather than ignored.
    #[test]
    fn long_mode_is_a_code_only_bit_at_every_level() {
        const LONG_MODE: u64 = 1 << 53;
        assert_ne!(Descriptor::USER_CODE.bits() & LONG_MODE, 0);
        assert_eq!(Descriptor::USER_DATA.bits() & LONG_MODE, 0);
    }
}
