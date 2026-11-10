//! Global descriptor table entries: permission bits for code and data, a base for the TSS.

/// The bits this crate sets; the tests name the ones it leaves clear.
mod bit {
    /// Accessed, set already, since the user pair is first loaded from a read-only table.
    pub(super) const ACCESSED: u64 = 1 << 40;
    /// Readable for a code segment, writable for a data segment.
    pub(super) const ACCESS: u64 = 1 << 41;
    /// Executable, the bit that makes a descriptor a code segment.
    pub(super) const EXECUTABLE: u64 = 1 << 43;
    /// Code or data, as opposed to a system descriptor.
    pub(super) const CODE_OR_DATA: u64 = 1 << 44;
    /// Descriptor privilege level three, both bits, since no ring 1 or 2 exists here.
    pub(super) const RING_THREE: u64 = 0b11 << 45;
    pub(super) const PRESENT: u64 = 1 << 47;
    /// 64-bit code; clear for data, and paired with a clear default-size bit.
    pub(super) const LONG_MODE: u64 = 1 << 53;
}

/// Descriptor type for an available 64-bit task-state segment.
const TASK_STATE_TYPE: u64 = 0x9 << 40;

/// A code or data segment descriptor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// Ring 0 data: present, writable, accessed; `SS` names it even in 64-bit mode.
    pub const KERNEL_DATA: Self =
        Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS | bit::ACCESSED);

    /// Ring 3 code: present, executable, readable, 64-bit, DPL 3, accessed.
    ///
    /// `sysret` and `iretq` fix the user descriptors' indices relative to each other.
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SystemDescriptor {
    /// The entry at the descriptor's own index.
    pub low: u64,
    /// The entry immediately after it, holding the top half of the base address.
    pub high: u64,
}

impl SystemDescriptor {
    /// A TSS of `size` bytes at `base`; a 16-bit size leaves the limit's high nibble zero.
    ///
    /// The limit is the size less one, so an `iomap_base` equal to the size falls outside it.
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
