//! Model-specific registers.
//!
//! Reading one is harmless; writing one can switch paging modes, relocate the
//! interrupt controller, or turn off features the compiler assumed were there.
//! Reads are ordinary calls; writes carry the register-specific contract that
//! each architecture caller must discharge.

use core::arch::asm;

/// Read the register numbered `msr`.
///
/// # Panics
///
/// Nothing here panics, but a register this processor does not implement raises
/// a general protection fault, which the kernel reports as one. Callers check
/// `cpuid` first.
pub(super) fn read(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: `rdmsr` in ring 0 reads the register named in `ecx` and writes
    // nothing. A register the processor does not implement faults rather than
    // corrupting anything, and the fault path reports it.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    u64::from(high) << 32 | u64::from(low)
}

/// Write `value` to the register numbered `msr`.
///
/// # Safety
///
/// The caller must know what `msr` governs and that `value` leaves the machine
/// running: these registers control paging, addressing, and where the interrupt
/// controller lives, and there is no bit pattern the processor rejects on the
/// grounds of being unwise.
pub(super) unsafe fn write(msr: u32, value: u64) {
    // SAFETY: `wrmsr` writes the register named in `ecx`, which is the operation
    // the caller has just taken responsibility for.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nomem, nostack, preserves_flags),
        );
    }
}
