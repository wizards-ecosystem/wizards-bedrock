/// Whether `EFLAGS.AC` is set right now, letting ring 0 touch user pages past SMAP.
///
/// Read instead of assumed, because ring 3 can set this bit and interrupt delivery does not
/// clear it.
#[cfg(feature = "selftest")]
pub fn access_check_is_off() -> bool {
    /// `EFLAGS.AC`.
    const AC: u64 = 1 << 18;

    let flags: u64;
    // SAFETY: `pushfq` and `pop` use one matching stack slot and restore `rsp`; no Rust value
    // aliases the temporary word.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    flags & AC != 0
}
