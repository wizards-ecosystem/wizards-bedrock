pub const VERSION: u32 = 1;

pub mod mode {
    pub const NONE: u32 = 0;
    pub const COUNTER: u32 = 1;
}

pub mod constants {
    pub const VERSION: usize = 0;
    pub const PAGE_BYTES: usize = 4;
    pub const COUNTER_HZ: usize = 8;
}

// Its own cache line: the clock is read constantly and the constants once.
pub const CLOCK_AT: usize = 64;

pub mod clock {
    pub const SEQ: usize = 0;
    pub const MODE: usize = 4;
    pub const BASE_NS: usize = 8;
    pub const FRACTION: usize = 16;
    pub const LAST_TICKS: usize = 24;
    pub const MULTIPLIER: usize = 32;
    pub const SHIFT: usize = 40;
}
