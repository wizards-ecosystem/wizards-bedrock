#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::ops::Range;

pub const PORT_COUNT: usize = 0x1_0000;

const PORTS_PER_WORD: usize = u64::BITS as usize;

const WORDS: usize = PORT_COUNT / PORTS_PER_WORD;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PortSpace {
    reserved: [u64; WORDS],
}

impl PortSpace {
    pub const fn new() -> Self {
        Self {
            reserved: [0; WORDS],
        }
    }

    const fn is_reserved(&self, port: usize) -> bool {
        if port >= PORT_COUNT {
            return true;
        }
        self.reserved[port / PORTS_PER_WORD] & bit(port) != 0
    }

    #[must_use = "ignoring a refused reservation means driving ports something else owns"]
    pub fn reserve(&mut self, port: usize, length: usize) -> bool {
        let Some(span) = self.span(port, length) else {
            return false;
        };
        for port in span {
            self.reserved[port / PORTS_PER_WORD] |= bit(port);
        }
        true
    }

    #[must_use = "ignoring a refused reservation means driving ports something else owns"]
    pub fn reserve_all(&mut self, spans: &[(usize, usize)]) -> bool {
        if spans.is_empty() {
            return false;
        }

        let mut resolved = [const { None }; MAX_SPANS];
        let Some(room) = resolved.get_mut(..spans.len()) else {
            return false;
        };
        for (index, (port, length)) in spans.iter().enumerate() {
            let Some(span) = self.span(*port, *length) else {
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

    pub fn is_available(&self, port: usize, length: usize) -> bool {
        self.span(port, length).is_some()
    }

    fn span(&self, port: usize, length: usize) -> Option<Range<usize>> {
        let span = bounded_span(port, length)?;
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

const fn bit(port: usize) -> u64 {
    1 << (port % PORTS_PER_WORD)
}

fn bounded_span(port: usize, length: usize) -> Option<Range<usize>> {
    if length == 0 {
        return None;
    }
    let end = port.checked_add(length).filter(|&end| end <= PORT_COUNT)?;
    Some(port..end)
}

const MAX_SPANS: usize = 8;

fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

#[cfg(test)]
mod tests;
