//! Property tests over the reservation contract.
//!
//! `reserve` states an atomicity claim in as many words: "False — and nothing
//! recorded — if any of them is already reserved, if the span would run past
//! the end of the space, or if `len` is zero, which would reserve nothing while
//! reporting success." A refusal that recorded a partial span would leave ports
//! reserved to nobody, which no caller could release and no later claim could
//! take.
//!
//! The module doc states why the crate exists: "A driver that owns a device's
//! registers must own *every* port an access reaches, not merely the one it
//! names." D-39 records a phase spent recording one port per claim however wide
//! the access — the defect this asks about over every width at once.
//!
//! What is *not* here: who may release a span. That is the wrapper's, behind
//! the unique span capabilities of D-199, and no arithmetic here can see it.

use port_space::{PORT_COUNT, PortSpace};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRunner};

/// The window generated requests reach, plus a margin either side, so a
/// snapshot can see a span that ran past what was asked for.
const WINDOW: usize = 600;

/// One reservation request, bounded well below `PORT_COUNT` so that generated
/// spans collide with each other rather than scattering.
fn request() -> impl Strategy<Value = (usize, usize)> {
    (0usize..512, 0usize..12)
}

/// Which ports a space considers reserved across the window.
fn snapshot(space: &PortSpace) -> Vec<bool> {
    (0..WINDOW).map(|port| space.is_reserved(port)).collect()
}

/// A space with some reservations already granted.
fn occupied(setup: &[(usize, usize)]) -> PortSpace {
    let mut space = PortSpace::new();
    for (port, len) in setup {
        let _ = space.reserve(*port, *len);
    }
    space
}

proptest! {
    /// "False — and nothing recorded."
    ///
    /// The atomicity half of the contract. A partial record is invisible to the
    /// caller that was refused and to every later one, so only a before-and-
    /// after comparison over the whole window can see it.
    #[test]
    fn a_refused_reservation_records_nothing(
        setup in prop::collection::vec(request(), 0..8),
        attempt in request(),
    ) {
        let mut space = occupied(&setup);
        let before = snapshot(&space);
        if space.reserve(attempt.0, attempt.1) {
            return Ok(());
        }
        prop_assert_eq!(
            snapshot(&space),
            before,
            "a refused reservation changed the space at {:?}",
            attempt
        );
    }

    /// A granted reservation owns every port its access reaches.
    ///
    /// D-39's defect, asked of every width rather than the one that was tried:
    /// a 32-bit access at `n` drives `n..n + 4`, and recording only `n` leaves
    /// three ports something else can claim.
    #[test]
    fn a_granted_reservation_owns_its_whole_span(
        setup in prop::collection::vec(request(), 0..8),
        attempt in request(),
    ) {
        let mut space = occupied(&setup);
        if !space.reserve(attempt.0, attempt.1) {
            return Ok(());
        }
        let (port, len) = attempt;
        prop_assert!(len > 0, "a zero-length reservation was granted");
        for offset in 0..len {
            prop_assert!(
                space.is_reserved(port + offset),
                "port {} was granted but reads as free",
                port + offset
            );
        }
    }

    /// No two granted reservations share a port.
    ///
    /// The property the whole bitmap exists to provide, over sequences of
    /// overlapping requests rather than the pairs a test can enumerate.
    #[test]
    fn no_two_granted_reservations_share_a_port(
        requests in prop::collection::vec(request(), 1..16),
    ) {
        let mut space = PortSpace::new();
        let mut owner: Vec<Option<usize>> = vec![None; WINDOW];
        for (index, (port, len)) in requests.iter().copied().enumerate() {
            if !space.reserve(port, len) {
                continue;
            }
            for offset in 0..len {
                let at = port + offset;
                if at >= WINDOW {
                    continue;
                }
                prop_assert_eq!(
                    owner[at],
                    None,
                    "port {} was granted to two requests, {:?} and {}",
                    at,
                    owner[at],
                    index
                );
                owner[at] = Some(index);
            }
        }
    }

    /// A zero-length span "would reserve nothing while reporting success".
    #[test]
    fn a_zero_length_reservation_is_always_refused(port in 0usize..PORT_COUNT) {
        let mut space = PortSpace::new();
        prop_assert!(!space.reserve(port, 0), "a zero-length span was granted");
    }

    /// Past the end reads as reserved — "the answer that stops a caller
    /// treating it as free".
    #[test]
    fn every_port_past_the_end_reads_as_reserved(over in 0usize..4096) {
        let space = PortSpace::new();
        prop_assert!(space.is_reserved(PORT_COUNT + over), "a port past the end read as free");
    }
}

/// How often the two complementary properties reach their branch — D-242's
/// obligation on a generated-input check.
///
/// One asserts about refusals and the other about grants, so a rate near zero
/// or near one would leave one of them asserting nothing.
#[test]
fn the_reservation_properties_reach_both_grants_and_refusals() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut granted = 0usize;
    let mut refused = 0usize;

    for _ in 0..SAMPLES {
        let setup = prop::collection::vec(request(), 0..8)
            .new_tree(&mut runner)
            .expect("a setup")
            .current();
        let attempt = request()
            .new_tree(&mut runner)
            .expect("an attempt")
            .current();
        let mut space = occupied(&setup);
        if space.reserve(attempt.0, attempt.1) {
            granted += 1;
        } else {
            refused += 1;
        }
    }

    println!("port-space: {granted} granted, {refused} refused of {SAMPLES}");
    assert!(
        granted > SAMPLES / 8,
        "only {granted} of {SAMPLES} attempts were granted; the grant property is near-vacuous"
    );
    assert!(
        refused > SAMPLES / 8,
        "only {refused} of {SAMPLES} attempts were refused; the atomicity property is near-vacuous"
    );
}
