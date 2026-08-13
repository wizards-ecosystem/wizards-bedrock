//! Direct CPU control: the interrupt flag and halting.

use core::arch::asm;

/// Clear the interrupt flag on this CPU.
pub fn disable_interrupts() {
    // SAFETY: `cli` clears IF on the current CPU and touches nothing else. It
    // has no preconditions in ring 0. `preserves_flags` is deliberately absent —
    // modifying IF is the whole point.
    unsafe { asm!("cli", options(nomem, nostack)) };
}

/// Set the interrupt flag on this CPU.
///
/// # Safety
///
/// A valid IDT must be loaded and every interrupt the platform can raise must
/// have a handler, or the next interrupt triple-faults. The caller must also be
/// prepared for its own execution to be preempted from this point on.
pub unsafe fn enable_interrupts() {
    // SAFETY: the caller guarantees an IDT is loaded and preemption is safe here.
    unsafe { asm!("sti", options(nomem, nostack)) };
}

/// Halt this CPU until the next interrupt.
///
/// Returns immediately if interrupts are disabled and one is already pending;
/// callers that must not proceed should loop, as [`halt_forever`] does.
pub fn halt() {
    // SAFETY: `hlt` in ring 0 stops the CPU until an interrupt arrives. It has
    // no memory effects and cannot fail.
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
}

/// Disable interrupts and halt permanently.
///
/// This is the kernel's terminal state: used by the panic handler and by the
/// idle path before a scheduler exists.
pub fn halt_forever() -> ! {
    disable_interrupts();
    loop {
        halt();
    }
}
