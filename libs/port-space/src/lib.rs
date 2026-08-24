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

#[cfg(test)]
mod tests {
    use super::*;

    /// Widths an x86 port access can have, and therefore every span length a
    /// real kernel ever asks for.
    const WIDTHS: [usize; 3] = [1, 2, 4];

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

    #[test]
    fn availability_agrees_with_what_reserving_would_do() {
        let mut space = PortSpace::new();
        assert!(space.reserve(0x20, 2));

        for start in 0x1C..0x28 {
            for len in WIDTHS {
                let expected = space.is_available(start, len);
                let mut copy = space;
                assert_eq!(copy.reserve(start, len), expected, "{len} at {start:#x}");
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
}
