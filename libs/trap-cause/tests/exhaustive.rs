//! All 256 vectors, and agreement with `x86-tables`, which once disagreed about vector 28.

use trap_cause::TrapCause;
use x86_tables::exception;

/// The whole domain, since a decode arm returning its neighbour's value escapes a sample.
#[test]
fn every_one_of_the_256_vectors_reports_the_vector_it_decoded_from() {
    for vector in 0..=u8::MAX {
        let cause = TrapCause::decode(vector, 0);
        assert_eq!(
            cause.vector(),
            vector,
            "vector {vector} decoded to a cause reporting {}",
            cause.vector()
        );
        assert!(
            !cause.name().is_empty(),
            "vector {vector} decoded to a cause with no name"
        );
    }
}

/// A vector is architectural in both crates or in neither, the general vector-28 disagreement.
#[test]
fn every_vector_is_architectural_in_both_crates_or_in_neither() {
    let mut architectural = 0usize;
    for vector in 0..=u8::MAX {
        let named_by_tables = exception::name(vector).is_some();
        let is_fault = matches!(TrapCause::decode(vector, 0), TrapCause::Fault(_));
        assert_eq!(
            named_by_tables, is_fault,
            "vector {vector}: x86-tables names it {named_by_tables}, \
             trap-cause calls it a fault {is_fault}"
        );
        if named_by_tables {
            architectural += 1;
        }
    }
    assert!(
        architectural > 16,
        "only {architectural} architectural vectors of 256; the tables look empty \
         and the comparison would pass on two empty answers"
    );
}

/// Only an architectural vector pushes an error code, or the one entry frame would misalign.
#[test]
fn no_vector_pushes_an_error_code_without_being_architectural() {
    let mut pushing = 0usize;
    for vector in 0..=u8::MAX {
        if exception::pushes_error_code(vector) {
            pushing += 1;
            assert!(
                exception::name(vector).is_some(),
                "vector {vector} pushes an error code but is not architectural"
            );
        }
    }
    assert!(
        pushing > 0 && pushing < 32,
        "{pushing} of 256 vectors pushing an error code is not a plausible table"
    );
}
