use core::cell::UnsafeCell;

// Whoever writes through the pointer must prove nothing else reads the storage.
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct HardwareCell<T>(UnsafeCell<T>);

// SAFETY: the cell hands out only a raw pointer, so it grants no access by itself; every unsafe
// step that turns the pointer into a `&mut` carries its own argument for why it is exclusive.
unsafe impl<T> Sync for HardwareCell<T> {}

impl<T> HardwareCell<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    pub(crate) const fn as_mut_ptr(&self) -> *mut T {
        self.0.get()
    }

    pub(crate) fn address(&self) -> u64 {
        self.as_mut_ptr() as u64
    }
}
