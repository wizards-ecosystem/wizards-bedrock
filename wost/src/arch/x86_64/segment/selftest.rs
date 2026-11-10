use super::*;

/// The selector this CPU is executing under.
#[cfg(feature = "selftest")]
pub fn code_selector() -> Selector {
    current_code_selector()
}

/// The selector this CPU's stack accesses use.
#[cfg(feature = "selftest")]
pub fn stack_selector() -> Selector {
    selectors().1
}

/// The selector in the task register, or a null selector if none is loaded.
#[cfg(feature = "selftest")]
pub fn task_register() -> Selector {
    let bits: u16;
    // SAFETY: `str` reads the task register. It is unprivileged, has no effects,
    // and stores zero when no task-state segment has been loaded.
    unsafe {
        asm!(
            "str {selector:x}",
            selector = out(reg) bits,
            options(nomem, nostack, preserves_flags),
        );
    }
    Selector::from_bits(bits)
}

/// Where the double-fault handler's stack currently starts.
#[cfg(feature = "selftest")]
pub fn double_fault_stack() -> u64 {
    double_fault_stack_address()
}

#[cfg(feature = "selftest")]
pub(crate) fn enclave_ist_stack_is_zeroed_for_test() -> bool {
    let bytes = FAULT_STACK.as_mut_ptr().cast::<u8>();
    (0..FAULT_STACK_BYTES).all(|offset| {
        // SAFETY: paired route evidence is checked by the caller before this
        // read, and the linker-owned allocation spans FAULT_STACK_BYTES.
        let byte = unsafe { bytes.add(offset) };
        // SAFETY: `byte` is inside that same inactive allocation.
        unsafe { byte.read_volatile() == 0 }
    })
}
