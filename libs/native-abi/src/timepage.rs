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
}

// World time in nanoseconds, a multiple of the precision; one aligned word that only rises.
pub const NOW_NS: usize = 64;
