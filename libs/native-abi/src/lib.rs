#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

mod rights;
pub mod ring;
pub mod timepage;

pub use rights::Rights;

const PAGE_BYTES: usize = 4096;

pub mod limits {
    // Registered buffers per process; index 0 is the data page.
    pub const BUFFERS: u32 = 16;
}

pub mod entry {
    pub const VERSION: u64 = 3;

    pub const BYTES: usize = 64;

    pub mod offset {
        pub const VERSION: usize = 0;
        pub const ARGUMENT: usize = 8;
        pub const BOOTSTRAP_HANDLE: usize = 16;
        pub const OPERAND: usize = 24;
        // Where the kernel placed this process's ring, buffer 0 and the time page.
        pub const RING: usize = 32;
        pub const BUFFER: usize = 40;
        pub const TIME: usize = 48;
    }
}

pub mod error {
    pub const ENOSYS: u64 = (-38i64) as u64;

    pub const EINVAL: u64 = (-22i64) as u64;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Call {
    Exit,
    RingEnter,
    RingWait,
    ThreadBase,
    Unknown(u64),
}

impl Call {
    pub const fn decode(value: u64) -> Self {
        match value {
            0 => Self::Exit,
            1 => Self::RingEnter,
            2 => Self::ThreadBase,
            3 => Self::RingWait,
            other => Self::Unknown(other),
        }
    }

    pub const fn number(self) -> u64 {
        match self {
            Self::Exit => 0,
            Self::RingEnter => 1,
            Self::ThreadBase => 2,
            Self::RingWait => 3,
            Self::Unknown(other) => other,
        }
    }
}

#[cfg(test)]
mod tests;
