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
mod tests {
    use super::*;

    /// Intel SDM Vol. 3A table 6-1, transcribed. The bitmask is unreadable by
    /// design — it exists so the assembler can test one expression per stub —
    /// so this is the readable copy it is checked against.
    const WITH_ERROR_CODE: [u8; 10] = [8, 10, 11, 12, 13, 14, 17, 21, 29, 30];

    #[test]
    fn the_error_code_mask_lists_exactly_the_documented_vectors() {
        for vector in 0..=u8::MAX {
            let expected = WITH_ERROR_CODE.contains(&vector);
            assert_eq!(
                pushes_error_code(vector),
                expected,
                "vector {vector} error code"
            );
        }
    }

    #[test]
    fn no_vector_above_the_architectural_range_pushes_an_error_code() {
        assert!(!pushes_error_code(COUNT));
        assert!(!pushes_error_code(u8::MAX));
    }

    #[test]
    fn the_named_vectors_are_the_defined_ones() {
        assert_eq!(name(BREAKPOINT), Some("breakpoint"));
        assert_eq!(name(DOUBLE_FAULT), Some("double fault"));
        assert_eq!(name(14), Some("page fault"));
        assert_eq!(name(9), None, "coprocessor segment overrun is reserved");
        assert_eq!(name(15), None, "vector 15 is reserved");
        assert_eq!(name(COUNT), None, "the first interrupt is not an exception");
    }

    /// A fault report that cannot name the vector it is reporting is the least
    /// useful message in the kernel, and these are the vectors that carry the
    /// detail worth reporting.
    #[test]
    fn every_vector_with_an_error_code_has_a_name() {
        for vector in WITH_ERROR_CODE {
            assert!(name(vector).is_some(), "vector {vector} is unnamed");
        }
    }
}
