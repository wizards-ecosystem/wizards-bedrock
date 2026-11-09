//! The architectural exception vectors.
//!
//! Vectors 0–31 are reserved by the architecture; everything above them is a
//! hardware or software interrupt. Two facts about each vector matter to the
//! entry path: what to call it in a fault report, and whether the CPU pushes an
//! error code before the handler runs.

/// Vectors the architecture reserves.
pub const COUNT: u8 = 32;

/// `int3` — the trap a debugger plants. Resumable: the saved instruction
/// pointer is already past it.
pub const BREAKPOINT: u8 = 3;

/// `ud2` and every opcode the CPU does not implement in this mode. The one
/// fault a program can raise deliberately and unambiguously.
pub const INVALID_OPCODE: u8 = 6;

/// Raised when handling one exception raised another. Overwhelmingly it means
/// the stack is unusable, so it is the one vector that needs a stack of its own.
pub const DOUBLE_FAULT: u8 = 8;

/// Raised when a memory access cannot be satisfied by the page tables. `CR2`
/// holds the address, which is the whole content of the report.
pub const PAGE_FAULT: u8 = 14;

/// One bit per vector that the CPU pushes an error code for.
///
/// The entry stubs push a zero for every other vector so that the saved frame
/// has one shape. This constant is what the stub generator in
/// `wost::arch::current::interrupt` assembles against, so the two cannot drift.
pub const ERROR_CODE_VECTORS: u32 = 0x6022_7D00;

/// Whether the CPU pushes an error code before entering `vector`'s handler.
pub const fn pushes_error_code(vector: u8) -> bool {
    vector < COUNT && ERROR_CODE_VECTORS >> vector & 1 != 0
}

/// What to call `vector` in a fault report.
///
/// Returns `None` for the vectors the architecture reserves without defining,
/// and for everything above the architectural range.
pub const fn name(vector: u8) -> Option<&'static str> {
    Some(match vector {
        0 => "divide error",
        1 => "debug",
        2 => "non-maskable interrupt",
        3 => "breakpoint",
        4 => "overflow",
        5 => "bound range exceeded",
        6 => "invalid opcode",
        7 => "device not available",
        8 => "double fault",
        10 => "invalid task state segment",
        11 => "segment not present",
        12 => "stack segment fault",
        13 => "general protection fault",
        14 => "page fault",
        16 => "x87 floating point error",
        17 => "alignment check",
        18 => "machine check",
        19 => "simd floating point error",
        20 => "virtualisation exception",
        21 => "control protection exception",
        28 => "hypervisor injection exception",
        29 => "vmm communication exception",
        30 => "security exception",
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
