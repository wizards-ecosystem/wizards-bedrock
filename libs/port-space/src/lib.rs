//! Reservation bookkeeping for the x86 I/O port space.
//!
//! A driver that owns a device's registers must own *every* port an access
//! reaches, not merely the one it names: a 32-bit `out` to port `n` drives
//! `n..n + 4`. Tracking that is arithmetic over a bitmap with no hardware
//! behind it, so it lives here and is unit-tested on the host; `wost` wraps one
//! of these in a lock and hands out the port handles.
//!
//! Fixed-driver reservations are permanent. A transient PCI lease may return
//! several dynamic spans together before a driver commits; the wrapper keeps
//! release behind the unique span capabilities (D-199).
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Fixed-driver claims in this registry are **permanent**, because a driver
    /// owns its registers for the life of the kernel. A transient higher-level
    /// lease can return its unique spans through
    /// [`release_all`](Self::release_all), but that does not help a caller
    /// which takes one fixed span and fails on the second: the first is
    /// stranded for the boot. Only the registry lock can answer the complete
    /// all-or-none question (D-138).
    ///
    /// False, and nothing recorded, if *any* span is unavailable — checked
    /// against the others in the same call, so two overlapping spans in one
    /// request are refused rather than silently merged.
    #[must_use = "ignoring a refused reservation means driving ports something else owns"]
    pub fn reserve_all(&mut self, spans: &[(usize, usize)]) -> bool {
        if spans.is_empty() {
            return false;
        }

        // Every span is resolved before any is written and checked against
        // its predecessors: `span` sees only what is recorded.
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

    /// Release several reserved spans, all or none.
    ///
    /// This is bookkeeping for a higher-level transient lease, not an
    /// address-based ownership API: the `wost` wrapper accepts only the unique
    /// span values its matching reservation returned (D-199).
    ///
    /// False, and nothing changed, if the request is empty, malformed,
    /// overlaps itself, or contains any port which is not currently reserved.
    #[must_use = "a refused release leaves every requested port reserved"]
    pub fn release_all(&mut self, spans: &[(usize, usize)]) -> bool {
        if spans.is_empty() {
            return false;
        }

        let mut resolved = [const { None }; MAX_SPANS];
        let Some(room) = resolved.get_mut(..spans.len()) else {
            return false;
        };
        for (index, (port, len)) in spans.iter().enumerate() {
            let Some(span) = bounded_span(*port, *len) else {
                return false;
            };
            if !span.clone().all(|port| self.is_reserved(port)) {
                return false;
            }
            for earlier in room.iter().take(index).flatten() {
                if overlaps(&span, earlier) {
                    return false;
                }
            }
            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] &= !bit(port);
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
        let span = bounded_span(port, len)?;
        span.clone()
            .all(|port| !self.is_reserved(port))
            .then_some(span)
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

/// A non-empty span inside the architectural port space.
fn bounded_span(port: usize, len: usize) -> Option<Range<usize>> {
    if len == 0 {
        return None;
    }
    let end = port.checked_add(len).filter(|&end| end <= PORT_COUNT)?;
    Some(port..end)
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
mod tests;
