use core::arch::asm;

// An unimplemented MSR raises #GP, so callers check `cpuid` first.
pub(super) fn read(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: `rdmsr` in ring 0 reads the register named in `ecx` and writes nothing; an
    // unimplemented register faults without corrupting anything, and the fault path reports it.
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

/// # Safety
///
/// The caller must know what `msr` governs and that `value` leaves the machine running: these
/// registers control paging, addressing and where the interrupt controller lives, and the
/// processor rejects no bit pattern for being unwise.
pub(super) unsafe fn write(msr: u32, value: u64) {
    let (low, high) = register_bits::u32_halves(value);
    // SAFETY: `wrmsr` writes the register named in `ecx`, the operation the caller has just taken
    // responsibility for.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") low,
            in("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
}
