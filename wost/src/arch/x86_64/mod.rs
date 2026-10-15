//! x86_64 primitives.

pub mod apic;
pub mod cpu;
pub mod interrupt;
pub(super) mod kpti;
pub mod paging;
pub mod pci;
pub mod percpu;
pub mod pic;
pub mod port;
pub mod segment;
pub mod task;
pub mod usercopy;
pub mod vtd;
pub mod xstate;

pub(crate) mod shared_memory;

mod msr;
mod table;

/// Whether the linked entry enclave contains the complete paired-root switch.
pub const fn kpti_entry_path_is_implemented() -> bool {
    kpti::entry_path_is_implemented()
}

#[cfg(feature = "selftest")]
/// Whether every observed user entry reached Rust only on its full root.
pub fn kpti_every_user_entry_reached_rust_on_full_root_for_test() -> bool {
    kpti::every_user_entry_reached_rust_on_full_root()
}

#[cfg(feature = "selftest")]
/// Whether each prepared user return was next observed through its stripped root.
pub fn kpti_every_user_return_installed_stripped_root_for_test() -> bool {
    kpti::every_user_return_installed_the_stripped_root()
}

#[cfg(feature = "selftest")]
/// Whether the inactive enclave stacks retain guards and no transition bytes.
pub fn kpti_entry_stacks_are_guarded_and_scrubbed_for_test() -> bool {
    kpti::entry_stacks_are_guarded_and_scrubbed()
}

#[cfg(feature = "selftest")]
/// Whether scheduling installed complete roots for at least two distinct pairs.
pub fn kpti_context_switch_moved_complete_root_pairs_for_test() -> bool {
    kpti::context_switch_moved_complete_root_pairs()
}
