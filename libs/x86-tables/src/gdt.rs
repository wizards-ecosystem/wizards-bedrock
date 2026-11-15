mod bit {
    // Preset: the user pair is first loaded from a read-only table.
    pub(super) const ACCESSED: u64 = 1 << 40;
    pub(super) const ACCESS: u64 = 1 << 41;
    pub(super) const EXECUTABLE: u64 = 1 << 43;
    pub(super) const CODE_OR_DATA: u64 = 1 << 44;
    pub(super) const RING_THREE: u64 = 0b11 << 45;
    pub(super) const PRESENT: u64 = 1 << 47;
    // 64-bit code requires the default-size bit clear.
    pub(super) const LONG_MODE: u64 = 1 << 53;
}

const TASK_STATE_TYPE: u64 = 0x9 << 40;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Descriptor(u64);

impl Descriptor {
    pub const NULL: Self = Self(0);

    pub const KERNEL_CODE: Self = Self(
        bit::PRESENT
            | bit::CODE_OR_DATA
            | bit::EXECUTABLE
            | bit::ACCESS
            | bit::LONG_MODE
            | bit::ACCESSED,
    );

    pub const KERNEL_DATA: Self =
        Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS | bit::ACCESSED);

    pub const USER_CODE: Self = Self(
        bit::PRESENT
            | bit::CODE_OR_DATA
            | bit::EXECUTABLE
            | bit::ACCESS
            | bit::LONG_MODE
            | bit::RING_THREE
            | bit::ACCESSED,
    );

    pub const USER_DATA: Self =
        Self(bit::PRESENT | bit::CODE_OR_DATA | bit::ACCESS | bit::RING_THREE | bit::ACCESSED);

    pub const fn bits(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SystemDescriptor {
    pub low: u64,
    pub high: u64,
}

impl SystemDescriptor {
    // The limit is the size less one, so an `iomap_base` equal to the size falls outside it.
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
