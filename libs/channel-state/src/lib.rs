#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod cell;
// The crate's only unverified code: Verus cannot check formatting.
#[cfg_attr(verus_only, verifier::external)]
mod fmt;
mod inbox;
mod peer;
pub mod proof;
mod receivers;

use vstd::prelude::verus_verify;

pub use cell::{CallEvent, CallState, Cell, CellRefusal, GENERATION_LIMIT};
pub use inbox::{BYTES, DESCRIPTORS, Descriptor, Entry, HANDLES, Inbox, InboxRefusal, Span};
pub use peer::{Hop, Link, Route, route};
pub use receivers::{NotArmed, Receivers, Relink};

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Unexpected;
