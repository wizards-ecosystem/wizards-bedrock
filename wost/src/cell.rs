//! Storage the hardware reaches behind Rust's back. A descriptor table is written once by an `init`
//! and then read by the CPU forever after; a fault stack is written by the CPU and never by us.

use core::cell::UnsafeCell;

/// A `static` that is reached through a raw pointer rather than a reference.
///
/// The type carries no synchronisation of its own: whoever writes through the pointer owes the
/// argument that nothing else is looking.
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct HardwareCell<T>(UnsafeCell<T>);

// SAFETY: the cell hands out only a raw pointer, so it grants no access by itself and cannot be
// aliased into a `&mut` without a further unsafe step. Every such step carries its own argument for
// why it is exclusive.
unsafe impl<T> Sync for HardwareCell<T> {}

impl<T> HardwareCell<T> {
    /// Place `value` in a static the hardware can be pointed at.
    pub(crate) const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    /// The address of the storage, stable for the life of the kernel.
    pub(crate) const fn as_mut_ptr(&self) -> *mut T {
        self.0.get()
    }

    /// The address of the storage as an integer, for a hardware register that wants one.
    pub(crate) fn address(&self) -> u64 {
        self.as_mut_ptr() as u64
    }
}
