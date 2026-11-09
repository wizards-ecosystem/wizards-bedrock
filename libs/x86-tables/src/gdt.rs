//! Global descriptor table entries.
//!
//! In 64-bit mode a code or data descriptor carries almost nothing: base and
//! limit are ignored, and what remains is a handful of permission bits. System
//! descriptors — the task-state segment is the only one this kernel uses — are
//! twice as wide and do carry a base address.

/// The bits this crate sets. Ones it deliberately leaves clear — the privilege
/// level of the kernel pair, granularity — are named in the tests that assert
/// they stay clear.
mod bit {
    /// Accessed, written already set (D-426).
    ///
    /// The processor sets it on the first load of a selector, which is a write
    /// to the table. The first load of the user pair is the `iretq` inside the
    /// paired-root exit, where the table is mapped read-only, so a clear bit
    /// is a supervisor write fault there on hardware that enforces it.
    pub(super) const ACCESSED: u64 = 1 << 40;
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

    /// Ring 0 code: present, executable, readable, 64-bit, accessed.
    pub const KERNEL_CODE: Self = Self(
        bit::PRESENT
            | bit::CODE_OR_DATA
            | bit::EXECUTABLE
            | bit::ACCESS
            | bit::LONG_MODE
            | bit::ACCESSED,
    );

    /// Ring 0 data: present, writable, accessed.
    ///
    /// `SS` must name one of these even in 64-bit mode, where `iretq` reloads it
    /// on every return from an exception.
    pub const KERNEL_DATA: Self =
        Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS | bit::ACCESSED);

    /// Ring 3 code: present, executable, readable, 64-bit, DPL 3, accessed.
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
            | bit::RING_THREE
            | bit::ACCESSED,
    );

    /// Ring 3 data: present, writable, DPL 3, accessed.
    pub const USER_DATA: Self =
        Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS | bit::RING_THREE | bit::ACCESSED);

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
mod tests;
