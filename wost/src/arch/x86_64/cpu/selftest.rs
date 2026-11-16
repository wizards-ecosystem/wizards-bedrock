pub fn access_flag_is_set() -> bool {
    const AC: u64 = 1 << 18;

    let flags: u64;
    // SAFETY: `pushfq` and `pop` use one matching stack slot and restore `rsp`; no Rust value
    // aliases the temporary word.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    flags & AC != 0
}
