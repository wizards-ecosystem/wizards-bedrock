//! x86_64 primitives.

pub mod apic;
pub mod cpu;
pub mod interrupt;
pub(super) mod kpti;
mod msr;
pub mod paging;
pub mod pci;
pub(crate) mod percpu;
pub mod pic;
pub mod port;
pub mod segment;
pub(crate) mod shared_memory;
mod table;
pub mod task;
pub mod usercopy;
pub mod vtd;
pub mod xstate;

/// Whether the linked entry enclave contains the complete paired-root switch.
pub const fn kpti_entry_path_is_implemented() -> bool {
    kpti::entry_path_is_implemented()
}
