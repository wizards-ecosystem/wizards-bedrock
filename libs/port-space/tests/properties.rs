//! Property tests over the reservation contract.
//! Who may release a span is the wrapper's to decide, and no arithmetic here can see it.

use port_space::{PORT_COUNT, PortSpace};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

/// The window requests reach plus a margin, so a snapshot sees a span that ran past its request.
const WINDOW: usize = 600;

/// One request, bounded well below `PORT_COUNT` so generated spans collide.
fn request() -> impl Strategy<Value = (usize, usize)> {
    (0usize..512, 0usize..12)
}

fn snapshot(space: &PortSpace) -> Vec<bool> {
    (0..WINDOW).map(|port| space.is_reserved(port)).collect()
}

fn occupied(setup: &[(usize, usize)]) -> PortSpace {
    let mut space = PortSpace::new();
    for (port, len) in setup {
        let _ = space.reserve(*port, *len);
    }
    space
}

proptest! {
    /// A partial record is invisible to every caller, so only a whole-window comparison can see it.
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

    /// A 32-bit access at `n` drives `n..n + 4`; recording only `n` leaves three claimable.
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

    #[test]
    fn a_zero_length_reservation_is_always_refused(port in 0usize..PORT_COUNT) {
        let mut space = PortSpace::new();
        prop_assert!(!space.reserve(port, 0), "a zero-length span was granted");
    }

    #[test]
    fn every_port_past_the_end_reads_as_reserved(over in 0usize..4096) {
        let space = PortSpace::new();
        prop_assert!(space.is_reserved(PORT_COUNT + over), "a port past the end read as free");
    }
}

/// One property asserts on refusals and one on grants.
/// A rate near zero or near one would leave one of them asserting nothing.
#[test]
fn the_reservation_properties_reach_both_grants_and_refusals() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::deterministic();
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
