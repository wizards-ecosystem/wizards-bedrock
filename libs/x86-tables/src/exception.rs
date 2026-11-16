pub const COUNT: u8 = 32;

pub const NON_MASKABLE: u8 = 2;

pub const INVALID_OPCODE: u8 = 6;

pub const DOUBLE_FAULT: u8 = 8;

pub const GENERAL_PROTECTION: u8 = 13;

pub const PAGE_FAULT: u8 = 14;

pub const MACHINE_CHECK: u8 = 18;

pub const ERROR_CODE_VECTORS: u32 = 0x6022_7D00;

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
