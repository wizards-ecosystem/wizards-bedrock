//! The architectural exception vectors 0 to 31: their names and which push an error code.

pub const COUNT: u8 = 32;

/// `int3`, resumable because the saved instruction pointer is already past it.
pub const BREAKPOINT: u8 = 3;

/// `ud2` and every unimplemented opcode, the one fault a program raises deliberately.
pub const INVALID_OPCODE: u8 = 6;

/// A fault while handling a fault, nearly always a dead stack, so it needs a stack of its own.
pub const DOUBLE_FAULT: u8 = 8;

/// The page tables refused an access; `CR2` holds the address.
pub const PAGE_FAULT: u8 = 14;

/// One bit per vector with an error code, which the stub generator in `wost` assembles against.
pub const ERROR_CODE_VECTORS: u32 = 0x6022_7D00;

/// Whether the CPU pushes an error code before entering `vector`'s handler.
pub const fn pushes_error_code(vector: u8) -> bool {
    vector < COUNT && ERROR_CODE_VECTORS >> vector & 1 != 0
}

/// What to call `vector` in a fault report, `None` when the architecture defines no name.
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
