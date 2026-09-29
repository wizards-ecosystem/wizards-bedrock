//! Bounded exhaustion over every vector, and the agreement that once failed.
//!
//! The domain is 256 wide, so it is walked rather than sampled (D-242). No
//! dependency is needed for that; `x86-tables` is a dev-dependency only because
//! the second half of this file compares the two crates against each other.
//!
//! `libs/elf64`'s module doc records the precedent, and it is the reason this
//! file exists: two crates privately transcribing the same architectural table
//! "is how `x86-tables` and `trap-cause` came to disagree about vector 28".
//! The disagreement was repaired; nothing since has checked that it stayed
//! repaired, and both crates still name every vector separately.
//!
//! What is *not* here: whether either table matches the manual. Neither crate
//! can answer that, and no amount of comparing them to each other will — they
//! could agree and both be wrong. This checks that they agree, which is the
//! defect that actually occurred.

use trap_cause::TrapCause;
use x86_tables::exception;

/// Every vector decodes to a cause that reports the vector it came from.
///
/// The whole domain, because a decode table's characteristic defect is one arm
/// returning its neighbour's value, and a sample that skips the neighbour
/// cannot see it.
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

/// A vector is architectural in both crates or in neither.
///
/// `x86_tables::exception::name` returns `Some` exactly for the vectors the
/// architecture defines; `trap_cause` calls those `Fault`. One crate calling a
/// vector architectural while the other calls it a device interrupt is the
/// vector-28 disagreement, in the general form.
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

/// Only an architectural vector pushes an error code.
///
/// The entry path sizes one frame for every vector (D-29), so a vector that
/// claims to push an error code without being architectural would leave the
/// frame and the stack disagreeing about how many words were pushed.
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
