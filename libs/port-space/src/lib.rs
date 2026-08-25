//! Reservation bookkeeping for the x86 I/O port space.
//!
//! A driver that owns a device's registers must own *every* port an access
//! reaches, not merely the one it names: a 32-bit `out` to port `n` drives
//! `n..n + 4`. Tracking that is arithmetic over a bitmap with no hardware
//! behind it, so it lives here and is unit-tested on the host; `wost` wraps one
//! of these in a lock and hands out the port handles.
//!
//! Reservations are permanent by design. A driver owns its registers for the
//! life of the kernel, and a registry with no release path cannot suffer the
//! stale-handle problem a releasable one would.
//!
//! # Examples
//!
//! ```
//! use port_space::PortSpace;
//!
//! let mut space = PortSpace::new();
//! // A serial port: eight registers from 0x3F8.
//! assert!(space.reserve(0x3F8, 8));
//!
//! // A 32-bit access at 0x3F6 reaches 0x3F6..0x3FA, so it overlaps.
//! assert!(!space.reserve(0x3F6, 4));
//! // The two ports below it are still free.
//! assert!(space.reserve(0x3F6, 2));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::ops::Range;

/// Addressable I/O ports on x86 — the space `in` and `out` can name.
pub const PORT_COUNT: usize = 0x1_0000;

/// Ports one word of the bitmap accounts for.
const PORTS_PER_WORD: usize = u64::BITS as usize;

/// Words needed to give every port a bit.
const WORDS: usize = PORT_COUNT / PORTS_PER_WORD;

/// Which ports are spoken for.
///
/// One bit per port. [`reserve`](Self::reserve) is all-or-nothing: a span that
/// overlaps an existing reservation leaves the table exactly as it found it, so
/// a refused claim can never half-own a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortSpace {
    reserved: [u64; WORDS],
}

impl PortSpace {
    /// A space with nothing reserved.
    pub const fn new() -> Self {
        Self {
            reserved: [0; WORDS],
        }
    }

    /// Whether `port` is spoken for.
    ///
    /// A port outside the space reads as reserved: nothing may drive it, which
    /// is the answer that stops a caller treating it as free.
    pub const fn is_reserved(&self, port: usize) -> bool {
        if port >= PORT_COUNT {
            return true;
        }
        self.reserved[port / PORTS_PER_WORD] & bit(port) != 0
    }

    /// Reserve the `len` ports starting at `port`.
    ///
    /// False — and nothing recorded — if any of them is already reserved, if
    /// the span would run past the end of the space, or if `len` is zero, which
    /// would reserve nothing while reporting success.
    #[must_use = "ignoring a refused reservation means driving ports something else owns"]
    pub fn reserve(&mut self, port: usize, len: usize) -> bool {
        let Some(span) = self.span(port, len) else {
            return false;
        };
        for port in span {
            self.reserved[port / PORTS_PER_WORD] |= bit(port);
        }
        true
    }

    /// Reserve several spans, all or none.
    ///
    /// A claim in this registry is **permanent** — nothing releases, because a
    /// driver owns its registers for the life of the kernel. That makes a
    /// caller wanting two spans a trap: take the first, fail on the second,
    /// and the first is stranded where nothing can ever claim it again, so the
    /// thing that needed both can never be built on this boot.
    ///
    /// Two callers had exactly that shape. `i8042::Controller::new` chains
    /// `DATA` and `CONTROL`, so a refused `CONTROL` foreclosed the keyboard;
    /// `pic::silence` chains its two blocks. Neither could guard against it
    /// alone, because the check and the claim are two moments and the registry
    /// has no undo. Doing it under one lock, against one bitmap, is the only
    /// place the question can be answered (D-138).
    ///
    /// False, and nothing recorded, if *any* span is unavailable — checked
    /// against the others in the same call, so two overlapping spans in one
    /// request are refused rather than silently merged.
    #[must_use = "ignoring a refused reservation means driving ports something else owns"]
    pub fn reserve_all(&mut self, spans: &[(usize, usize)]) -> bool {
        if spans.is_empty() {
            return false;
        }

        // Every span resolved before any is written, and each checked against
        // the ones before it — `span` only sees what is already *recorded*, so
        // two requests for the same port in one call would both pass it.
        let mut resolved = [const { None }; MAX_SPANS];
        let Some(room) = resolved.get_mut(..spans.len()) else {
            return false;
        };
        for (index, (port, len)) in spans.iter().enumerate() {
            let Some(span) = self.span(*port, *len) else {
                return false;
            };
            for earlier in room.iter().take(index).flatten() {
                if overlaps(&span, earlier) {
                    return false;
                }
            }
            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);
            }
        }
        true
    }

    /// Whether the `len` ports starting at `port` could be reserved right now.
    ///
    /// The question [`reserve`](Self::reserve) answers as a side effect,
    /// separated out so a caller can ask without taking anything.
    pub fn is_available(&self, port: usize, len: usize) -> bool {
        self.span(port, len).is_some()
    }

    /// The ports a `len`-wide access at `port` reaches, or `None` if that span
    /// is not this caller's to take.
    fn span(&self, port: usize, len: usize) -> Option<Range<usize>> {
        if len == 0 {
            return None;
        }
        let end = port.checked_add(len).filter(|&end| end <= PORT_COUNT)?;
        (port..end)
            .all(|port| !self.is_reserved(port))
            .then_some(port..end)
    }
}

impl Default for PortSpace {
    fn default() -> Self {
        Self::new()
    }
}

/// The bit within its word that `port` occupies.
const fn bit(port: usize) -> u64 {
    1 << (port % PORTS_PER_WORD)
}

/// Spans one [`reserve_all`](PortSpace::reserve_all) may take at once.
///
/// Two is what the two callers need. The array is fixed because this crate is
/// `no_std` with no allocator, and a ceiling that refuses is better than a
/// silent truncation that reserves some of what was asked for.
pub const MAX_SPANS: usize = 8;

/// Whether two resolved spans share a port.
fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Widths an x86 port access can have, and therefore every span length a
    /// real kernel ever asks for.
    const WIDTHS: [usize; 3] = [1, 2, 4];

    /// A refused multi-span reservation strands nothing.
    ///
    /// The whole point: a claim here is permanent, so a caller that takes the
    /// first span and then fails has foreclosed it for the life of the boot.
    /// Both real callers wanted two spans and neither could ask for them
    /// together (D-138).
    #[test]
    fn a_refused_multi_span_reservation_takes_nothing() {
        const FIRST: usize = 0x60;
        const SECOND: usize = 0x64;

        let mut space = PortSpace::new();
        // Something else already owns the second span.
        assert!(space.reserve(SECOND, 1));

        assert!(
            !space.reserve_all(&[(FIRST, 1), (SECOND, 1)]),
            "the pair must be refused when either half is taken"
        );
        // And the first is still free, which is what chaining would have lost.
        assert!(
            space.is_available(FIRST, 1),
            "a refused pair stranded its first span"
        );
        // So a later attempt can still get it once the obstacle is theirs.
        assert!(space.reserve(FIRST, 1));
    }

    /// A granted multi-span reservation takes all of them.
    #[test]
    fn a_granted_multi_span_reservation_takes_every_span() {
        let spans = [(0x60, 1), (0x64, 1), (0x3F8, 8)];

        let mut space = PortSpace::new();
        assert!(space.reserve_all(&spans));

        for (port, len) in spans {
            assert!(!space.is_available(port, len), "{port:#x} was not recorded");
        }
        // And nothing beyond them: the span at 0x70 is untouched.
        assert!(space.is_available(0x70, 1));
        // Asking again is refused, because the registry never releases.
        assert!(!space.reserve_all(&spans));
    }

    /// Two spans in one request that overlap each other are refused.
    ///
    /// `span` only sees what is already *recorded*, so without the pairwise
    /// check both halves of `[(0x60, 4), (0x62, 2)]` pass it and the caller is
    /// told it owns one range twice.
    #[test]
    fn a_request_that_overlaps_itself_is_refused() {
        let mut space = PortSpace::new();
        assert!(!space.reserve_all(&[(0x60, 4), (0x62, 2)]));
        assert!(space.is_available(0x60, 4), "nothing was recorded");

        // Adjacent but disjoint is fine — the boundary is exclusive.
        assert!(space.reserve_all(&[(0x60, 2), (0x62, 2)]));
    }

    /// An empty request and one past the ceiling are both refused.
    #[test]
    fn an_empty_or_oversized_multi_span_request_is_refused() {
        let mut space = PortSpace::new();
        // Empty would reserve nothing while reporting success.
        assert!(!space.reserve_all(&[]));

        let too_many: [(usize, usize); MAX_SPANS + 1] =
            core::array::from_fn(|index| (index * 2, 1));
        assert!(!space.reserve_all(&too_many), "past the ceiling is refused");
        assert!(space.is_available(0, 1), "and nothing was recorded");
    }

    #[test]
    fn a_fresh_space_reserves_nothing() {
        let space = PortSpace::new();

        for port in [0, 1, 0x3F8, PORT_COUNT - 1] {
            assert!(!space.is_reserved(port), "port {port:#x}");
        }
        assert_eq!(space, PortSpace::default());
    }

    #[test]
    fn reserving_marks_exactly_its_span() {
        let mut space = PortSpace::new();
        assert!(space.reserve(0x3F8, 8));

        assert!(!space.is_reserved(0x3F7), "the port below is untouched");
        for port in 0x3F8..0x400 {
            assert!(space.is_reserved(port), "port {port:#x}");
        }
        assert!(!space.is_reserved(0x400), "the port above is untouched");
    }

    /// The defect this crate exists to rule out: a claim marking one port while
    /// the access it authorises drives four.
    #[test]
    fn a_wide_reservation_covers_every_port_it_drives() {
        for width in WIDTHS {
            let mut space = PortSpace::new();
            assert!(space.reserve(0xF4, width), "width {width}");

            for offset in 0..width {
                assert!(space.is_reserved(0xF4 + offset), "width {width} +{offset}");
            }
            assert!(!space.is_reserved(0xF4 + width), "width {width} ran over");
        }
    }

    #[test]
    fn a_second_reservation_of_the_same_port_is_refused() {
        let mut space = PortSpace::new();
        assert!(space.reserve(0x60, 1));
        assert!(!space.reserve(0x60, 1));
    }

    /// Every way one span can meet another — touching below, overlapping from
    /// below, contained, containing, overlapping from above, touching above —
    /// swept rather than enumerated, so no case is the one nobody thought of.
    #[test]
    fn overlap_is_refused_from_either_side_and_adjacency_is_not() {
        let held = 0x40..0x44;
        for start in 0x3C..0x48 {
            for len in WIDTHS {
                let mut space = PortSpace::new();
                assert!(space.reserve(held.start, held.len()));

                let overlaps = (start..start + len).any(|port| held.contains(&port));
                assert_eq!(
                    space.reserve(start, len),
                    !overlaps,
                    "{len} ports at {start:#x} against {held:#x?}"
                );
            }
        }
    }

    /// A refused reservation must leave the table untouched, or a driver told
    /// "no" still ends up owning half of a neighbour's registers.
    #[test]
    fn a_refused_reservation_records_nothing() {
        let mut space = PortSpace::new();
        assert!(space.reserve(0x64, 1));

        let before = space;
        assert!(!space.reserve(0x62, 4), "the span straddles 0x64");
        assert_eq!(space, before);
        assert!(!space.is_reserved(0x62));
        assert!(!space.is_reserved(0x63));
        assert!(!space.is_reserved(0x65));
    }

    #[test]
    fn a_span_running_past_the_end_of_the_space_is_refused() {
        let mut space = PortSpace::new();

        assert!(!space.reserve(PORT_COUNT - 3, 4));
        assert!(!space.reserve(PORT_COUNT, 1));
        assert!(!space.reserve(usize::MAX, 1), "the addition must not wrap");
        assert_eq!(space, PortSpace::new());
    }

    /// The last port in the space is ordinary; only what would follow it is not.
    #[test]
    fn a_span_ending_exactly_at_the_end_of_the_space_is_allowed() {
        let mut space = PortSpace::new();

        assert!(space.reserve(PORT_COUNT - 4, 4));
        assert!(space.is_reserved(PORT_COUNT - 1));
    }

    #[test]
    fn an_empty_span_reserves_nothing_and_says_so() {
        let mut space = PortSpace::new();

        assert!(!space.reserve(0x80, 0));
        assert!(!space.is_available(0x80, 0));
        assert!(!space.is_reserved(0x80));
    }

    /// Outside the space there is no free port, so a caller cannot mistake the
    /// answer for an invitation.
    #[test]
    fn every_port_outside_the_space_reads_as_reserved() {
        let space = PortSpace::new();

        assert!(space.is_reserved(PORT_COUNT));
        assert!(space.is_reserved(usize::MAX));
    }

    /// Availability agrees with reserving, *and* with an independent oracle.
    ///
    /// `is_available` and `reserve` both return `span(p, l).is_some()`, so
    /// comparing them is `span(p,l).is_some() == span(p,l).is_some()` — true
    /// for every implementation of `span`, correct or not. Mutating `span`'s
    /// overlap scan killed three other tests here and never this one (D-105).
    #[test]
    fn availability_agrees_with_what_reserving_would_do() {
        let mut space = PortSpace::new();
        assert!(space.reserve(0x20, 2));
        // Reserved: ports 0x20 and 0x21.
        let taken = 0x20usize..0x22;

        for start in 0x1C..0x28 {
            for len in WIDTHS {
                let expected = space.is_available(start, len);
                let mut copy = space;
                assert_eq!(copy.reserve(start, len), expected, "{len} at {start:#x}");

                // The oracle: a span is available exactly when it touches no
                // reserved port, worked out with `Range` rather than with the
                // bitmap this is testing.
                let end = start + len;
                let overlaps = (start..end).any(|port| taken.contains(&port));
                assert_eq!(expected, !overlaps, "{len} at {start:#x}");
            }
        }
    }

    /// Word boundaries are where a bitmap goes wrong: a span crossing one must
    /// mark both sides, and a port beyond it must stay free.
    #[test]
    fn a_span_crossing_a_word_boundary_marks_both_words() {
        let mut space = PortSpace::new();
        let boundary = PORTS_PER_WORD;

        assert!(space.reserve(boundary - 2, 4));
        assert!(space.is_reserved(boundary - 1), "last bit of the low word");
        assert!(space.is_reserved(boundary), "first bit of the high word");
        assert!(!space.is_reserved(boundary + 2));
    }

    /// Sweeping the whole space one port at a time: every port reserves once
    /// and refuses thereafter, and no bit lands on a neighbour along the way.
    #[test]
    fn every_port_in_the_space_can_be_reserved_exactly_once() {
        let mut space = PortSpace::new();

        for port in 0..PORT_COUNT {
            assert!(space.reserve(port, 1), "port {port:#x}");
        }
        for port in 0..PORT_COUNT {
            assert!(space.is_reserved(port), "port {port:#x}");
            assert!(!space.reserve(port, 1), "port {port:#x} reserved twice");
        }
    }

    /// The I/O space is sixteen bits, from the architecture rather than from
    /// itself.
    ///
    /// Every ceiling is spelled `PORT_COUNT` and every test port is a literal
    /// below it, so halving the constant survived the suite — x86's I/O space
    /// being 65,536 ports appeared nowhere (D-124).
    #[test]
    fn the_io_space_is_sixteen_bits() {
        assert_eq!(PORT_COUNT, 65_536);

        // The highest port a one-byte access can reach, and the one past it.
        let mut space = PortSpace::new();
        assert!(space.reserve(0xFFFF, 1), "the last port is reservable");
        assert!(!space.is_available(0xFFFF, 2), "and nothing runs past it");
    }
}
