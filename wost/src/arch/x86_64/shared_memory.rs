use core::arch::asm;

// x86 TSO gives an aligned dword load acquire order.
/// # Safety
///
/// `address` must name four readable, naturally aligned bytes that remain mapped for the
/// instruction. The bytes must not back a live Rust object.
pub(crate) unsafe fn load_acquire_u32(address: usize) -> u32 {
    let value: u32;
    // SAFETY: the caller proves the address mapped and aligned; one x86 dword `mov` is indivisible,
    // and no memory-effect option keeps this a compiler barrier without forming a Rust reference.
    unsafe {
        asm!(
            "mov {value:e}, dword ptr [{address}]",
            address = in(reg) address,
            value = out(reg) value,
            options(nostack, preserves_flags),
        );
    }
    value
}

// x86 TSO gives an aligned dword store release order.
/// # Safety
///
/// `address` must name four writable, naturally aligned bytes that remain mapped for the
/// instruction. The bytes must not back a live Rust object.
pub(crate) unsafe fn store_release_u32(address: usize, value: u32) {
    // SAFETY: the caller proves the address mapped and aligned; one x86 dword `mov` is indivisible,
    // and no memory-effect option keeps this a compiler barrier without forming a Rust reference.
    unsafe {
        asm!(
            "mov dword ptr [{address}], {value:e}",
            address = in(reg) address,
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}

// x86 TSO gives an aligned qword load acquire order.
/// # Safety
///
/// `address` must name eight readable, naturally aligned bytes that remain mapped for the
/// instruction. The bytes must not back a live Rust object.
pub(crate) unsafe fn load_acquire_u64(address: usize) -> u64 {
    let value: u64;
    // SAFETY: the caller proves the address mapped and aligned; one x86 qword `mov` is indivisible,
    // and no memory-effect option keeps this a compiler barrier without forming a Rust reference.
    unsafe {
        asm!(
            "mov {value}, qword ptr [{address}]",
            address = in(reg) address,
            value = out(reg) value,
            options(nostack, preserves_flags),
        );
    }
    value
}

// `lock cmpxchg` is a full barrier; it returns the word it found, equal to `current` on success.
/// # Safety
///
/// `address` must name eight writable, naturally aligned bytes that remain mapped for the
/// instruction. The bytes must not back a live Rust object.
pub(crate) unsafe fn compare_exchange_u64(address: usize, current: u64, new: u64) -> u64 {
    let found: u64;
    // SAFETY: the caller proves the address mapped and aligned; a locked `cmpxchg` on an aligned
    // qword is one indivisible read-modify-write, and it forms no Rust reference to the bytes.
    unsafe {
        asm!(
            "lock cmpxchg qword ptr [{address}], {new}",
            address = in(reg) address,
            new = in(reg) new,
            inout("rax") current => found,
            options(nostack),
        );
    }
    found
}
