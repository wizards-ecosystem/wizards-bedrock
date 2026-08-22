//! x86_64 primitives.

pub mod apic;
pub mod cpu;
pub mod interrupt;
pub mod paging;
pub mod pic;
pub mod port;
pub mod segment;

mod msr;
mod table;
