#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod cell;
mod inbox;
mod peer;
mod receivers;
#[cfg(test)]
mod tests;

use core::fmt;

pub use cell::{CallEvent, CallState, Cell, CellRefusal, GENERATION_LIMIT};
pub use inbox::{
    BYTES, DESCRIPTORS, Descriptor, Entry, HANDLES, Inbox, InboxRefusal, MESSAGE_BYTES,
    MESSAGE_HANDLES, Span,
};
pub use peer::{Hop, Link, Route, route};
pub use receivers::{NotArmed, Receivers, Relink};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Unexpected;

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the event has no transition from this state")
    }
}

impl core::error::Error for Unexpected {}
