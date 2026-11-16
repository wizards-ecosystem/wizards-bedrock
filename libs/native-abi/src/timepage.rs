pub const VERSION: u32 = 2;

pub mod mode {
    pub const NONE: u32 = 0;
    pub const NORMAL: u32 = 1;
}

pub mod constants {
    pub const VERSION: usize = 0;
    pub const PAGE_BYTES: usize = 4;
    pub const MODE: usize = 8;
    pub const PRECISION_NS: usize = 16;
    pub const GENERATION: usize = 24;
    pub const BYTES: usize = 64;
}

// Written once, before the page is mapped; `encode` fixes each field's width.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Constants {
    pub page_bytes: u32,
    pub mode: u32,
    pub precision_ns: u64,
    pub generation: u64,
}

impl Constants {
    pub fn encode(self) -> [u8; constants::BYTES] {
        let mut bytes = [0u8; constants::BYTES];
        bytes[constants::VERSION..constants::VERSION + 4].copy_from_slice(&VERSION.to_le_bytes());
        bytes[constants::PAGE_BYTES..constants::PAGE_BYTES + 4]
            .copy_from_slice(&self.page_bytes.to_le_bytes());
        bytes[constants::MODE..constants::MODE + 4].copy_from_slice(&self.mode.to_le_bytes());
        bytes[constants::PRECISION_NS..constants::PRECISION_NS + 8]
            .copy_from_slice(&self.precision_ns.to_le_bytes());
        bytes[constants::GENERATION..constants::GENERATION + 8]
            .copy_from_slice(&self.generation.to_le_bytes());
        bytes
    }
}

// World time in nanoseconds, a multiple of the precision; one aligned word that only rises.
pub const NOW_NS: usize = 64;
