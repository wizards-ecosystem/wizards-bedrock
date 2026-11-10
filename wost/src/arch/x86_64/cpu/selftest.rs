use super::*;

/// Drive the write-protect bit off and through the kernel's enabling path.
///
/// Firmware commonly leaves `CR0.WP` set, so the check needs a controlled
/// off/on transition. Interrupts stay excluded and a failed enable is repaired.
#[cfg(feature = "selftest")]
pub fn drive_write_protection_for_test() -> bool {
    let disabled = interrupt::disable();
    write_cr0(read_cr0() & !CR0_WRITE_PROTECT);
    let was_cleared = !write_protection_is_enabled();
    enable_write_protection();
    let was_enabled = write_protection_is_enabled();
    if !was_enabled {
        write_cr0(read_cr0() | CR0_WRITE_PROTECT);
    }
    let result = was_cleared && was_enabled;
    disabled.restore();
    result
}

/// Attempt a supervisor write to a byte in the kernel's read-only image.
///
/// The assembly reads the byte and writes the same value back, so a broken
/// protection leaves the image unchanged. `true` means the write faulted and
/// the trap handler resumed at this probe's private fixup.
#[cfg(feature = "selftest")]
pub fn write_protection_probe() -> bool {
    // SAFETY: the assembly names one byte in this crate's own read-only image
    // and writes back exactly the value it read. The trap path recognises only
    // this instruction and address; without protection, no value changes.
    unsafe { wost_write_protection_probe() != 0 }
}

/// Drive a fast-syscall door open and through the kernel's closing path.
///
/// Firmware already boots the test machine shut, so only a controlled
/// closed/open/closed transition makes the production close causal (D-139).
/// Interrupts stay excluded while the door is open.
#[cfg(feature = "selftest")]
pub fn drive_fast_syscall_shutdown_for_test() -> bool {
    let disabled = interrupt::disable();
    let shut_at_start = fast_syscall_doors_are_shut();
    // SAFETY: sets the bit that enables an instruction this kernel never
    // executes — no `syscall` or `sysret` appears in the tree — and interrupts
    // are excluded until the private closing path runs below, so no existing
    // ring-3 context can resume through the open door.
    unsafe { msr::write(EFER, msr::read(EFER) | EFER_SYSCALL) };
    let reader_sees_open = !fast_syscall_doors_are_shut();
    shut_the_fast_syscall_doors();
    let shut_again = fast_syscall_doors_are_shut();
    disabled.restore();
    shut_at_start && reader_sees_open && shut_again
}

/// Whether `EFLAGS.AC` is set right now — that is, whether SMAP is currently
/// letting ring 0 touch user pages.
///
/// Read rather than assumed, because the whole point of D-101 is that ring 3
/// can set this bit and interrupt delivery does not clear it.
#[cfg(feature = "selftest")]
pub fn access_check_is_off() -> bool {
    /// `EFLAGS.AC`.
    const AC: u64 = 1 << 18;

    let flags: u64;
    // SAFETY: `pushfq` and `pop` use one matching stack slot and restore `rsp`;
    // no Rust value aliases the temporary word.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    flags & AC != 0
}

/// Whether page-table reloads flush every non-global translation.
#[cfg(feature = "selftest")]
pub fn page_table_context_ids_are_disabled_for_test() -> bool {
    page_table_context_ids_are_disabled()
}

#[cfg(feature = "selftest")]
pub(super) static PAGE_INVALIDATIONS: AtomicUsize = AtomicUsize::new(0);

/// Number of `invlpg` instructions issued since boot.
#[cfg(feature = "selftest")]
pub fn page_invalidations_for_test() -> usize {
    PAGE_INVALIDATIONS.load(Ordering::Relaxed)
}
