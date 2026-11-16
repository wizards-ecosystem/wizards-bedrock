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

// The interrupt vector of the call gate, ring 3's one way into the kernel.
pub const GATE_VECTOR: u8 = 0x80;

pub mod limits {
    // Registered buffers per process; index 0 is the data page.
    pub const BUFFERS: u32 = 16;
    // Mapping records per address space, the kernel's own placements included.
    pub const SPACE_RECORDS: u32 = 64;
    // Bytes of an `OpenAt` path, far below a page, because the kernel copies it onto its stack.
    pub const PATH_BYTES: usize = 256;
    // Components of a path, fixed so the length of an argument cannot set the resolver's work.
    pub const PATH_DEPTH: usize = 16;
    // Bytes of one component; longer is refused rather than truncated, which would find another file.
    pub const NAME_BYTES: usize = 255;
    // Bytes one `Read` returns at most; a larger request gets a short read.
    pub const READ_BYTES: usize = 512;
    // The kernel's placements stay above the first 4 GiB, where position-dependent images link.
    pub const PLACEMENT_START: u64 = 0x0000_0001_0000_0000;
    // The end of what `Map` accepts: the last canonical user page stays unmapped, since a trap
    // saving a RIP at the user end would fault on return.
    pub const USER_MAPPING_END: u64 = 0x0000_7FFF_FFFF_F000;
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

// The witness program's contract with the kernel that launches it and the gate that reads its exit.
pub mod witness {
    pub const AGREED: u64 = 0;

    // Far above any call the kernel serves, so adding a call cannot make this one real.
    pub const UNSERVED_CALL: u64 = 0x0BAD_0BAD;

    // The memory-refusals behaviour maps and unmaps one page at this many fresh table spans.
    pub const FRESH_MAPS: u64 = 64;

    // VMware's backdoor magic, which a refused probe leaves in `rax` for the fault record.
    pub const BACKDOOR_MAGIC: u64 = 0x564D_5868;

    // Entry-block arguments that pick a behaviour.
    pub mod behaviour {
        pub const ECHO: u64 = 0;
        pub const FAULT: u64 = 1;
        pub const PROBE: u64 = 2;
        pub const DUPLICATE: u64 = 3;
        pub const INVALID_RETURN_STACK: u64 = 6;
        pub const INVALID_RETURN_FLAGS: u64 = 7;
        pub const READ_COUNTER: u64 = 8;
        pub const READ_KERNEL: u64 = 9;
        pub const RING_WAIT: u64 = 10;
        pub const THREAD_BASE: u64 = 11;
        pub const SYSCALL: u64 = 12;
        pub const MEMORY: u64 = 13;
        pub const BACKDOOR: u64 = 14;
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
