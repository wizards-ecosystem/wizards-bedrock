use super::*;

/// Intel SDM Vol. 3A table 6-1, transcribed. The bitmask is unreadable by
/// design — it exists so the assembler can test one expression per stub —
/// so this is the readable copy it is checked against.
const WITH_ERROR_CODE: [u8; 10] = [8, 10, 11, 12, 13, 14, 17, 21, 29, 30];

/// The named constants are the vectors `name` answers for.
///
/// Index with each named constant so a literal-only lookup cannot leave the
/// constant itself unchecked; pin `COUNT` at its exact boundary (D-148).
#[test]
fn the_named_vectors_are_the_ones_the_table_names() {
    // SDM Vol. 3A table 6-1, transcribed. Each constant is used to *index*
    // the table, so a wrong constant gives the wrong name.
    for (constant, expected) in [
        (BREAKPOINT, "breakpoint"),
        (INVALID_OPCODE, "invalid opcode"),
        (DOUBLE_FAULT, "double fault"),
        (PAGE_FAULT, "page fault"),
    ] {
        assert_eq!(
            name(constant),
            Some(expected),
            "vector {constant} is not `{expected}`"
        );
    }

    // Thirty-two architectural vectors, exactly: 31 is the last reserved
    // one and 32 is the first the APIC may deliver.
    assert_eq!(COUNT, 32);
    assert!(name(COUNT - 1).is_none(), "31 is reserved and unnamed");
    assert!(name(COUNT).is_none(), "32 is not an exception");
}

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

#[test]
fn every_exception_name_matches_the_architecture() {
    let expected = [
        Some("divide error"),
        Some("debug"),
        Some("non-maskable interrupt"),
        Some("breakpoint"),
        Some("overflow"),
        Some("bound range exceeded"),
        Some("invalid opcode"),
        Some("device not available"),
        Some("double fault"),
        None,
        Some("invalid task state segment"),
        Some("segment not present"),
        Some("stack segment fault"),
        Some("general protection fault"),
        Some("page fault"),
        None,
        Some("x87 floating point error"),
        Some("alignment check"),
        Some("machine check"),
        Some("simd floating point error"),
        Some("virtualisation exception"),
        Some("control protection exception"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some("hypervisor injection exception"),
        Some("vmm communication exception"),
        Some("security exception"),
        None,
    ];

    for (vector, expected_name) in expected.into_iter().enumerate() {
        assert_eq!(name(vector as u8), expected_name, "vector {vector}");
    }
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
