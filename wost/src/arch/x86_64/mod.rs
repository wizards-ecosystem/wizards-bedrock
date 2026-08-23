//! x86_64 primitives.

pub mod apic;
pub mod cpu;
pub mod interrupt;
pub mod paging;
pub mod pic;
pub mod port;
pub mod segment;
pub mod task;

mod msr;
mod table;
