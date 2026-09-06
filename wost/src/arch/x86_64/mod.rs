//! x86_64 primitives.

pub mod apic;
pub mod cpu;
pub mod interrupt;
pub mod paging;
pub mod pci;
pub mod percpu;
pub mod pic;
pub mod port;
pub mod segment;
pub mod task;
pub mod usercopy;
pub mod xstate;

pub(crate) mod shared_memory;

mod msr;
mod table;
