//! Which x86 I/O ports are reserved; a claim must cover every port a wide access drives, and `wost` holds this behind a lock.
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

/// Addressable x86 I/O ports, the space `in` and `out` can name.
pub const PORT_COUNT: usize = 0x1_0000;

const PORTS_PER_WORD: usize = u64::BITS as usize;

const WORDS: usize = PORT_COUNT / PORTS_PER_WORD;

/// Which ports are spoken for, one bit each; a refused claim leaves the table as it was.
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

    /// Whether `port` is spoken for; a port outside the space reads as reserved so no caller drives it.
    pub const fn is_reserved(&self, port: usize) -> bool {
        if port >= PORT_COUNT {
            return true;
        }
        self.reserved[port / PORTS_PER_WORD] & bit(port) != 0
    }

    /// Reserve `len` ports at `port`, recording nothing on refusal; a zero `len` is refused as it would report success reserving nothing.
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

    /// Reserve every span or none; fixed claims are permanent, so a caller needing several must ask here, not span by span.
    #[must_use = "ignoring a refused reservation means driving ports something else owns"]
    pub fn reserve_all(&mut self, spans: &[(usize, usize)]) -> bool {
        if spans.is_empty() {
            return false;
        }

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

    /// Release every span or none; this cannot tell owners apart, so callers must pass only spans their lease reserved.
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

    /// Whether the `len` ports at `port` could be reserved now, asked without taking them.
    pub fn is_available(&self, port: usize, len: usize) -> bool {
        self.span(port, len).is_some()
    }

    /// The ports a `len`-wide access at `port` reaches, if all are in the space and free.
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

/// Spans one all-or-none call may take, fixed for want of an allocator; a longer request is refused whole.
pub const MAX_SPANS: usize = 8;

/// Whether two resolved spans share a port; `span` sees only what is recorded, so a request checks its own spans with this.
fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

#[cfg(test)]
mod tests;
