//! PS/2 keyboard scancode set 2 decoding.
//!
//! A keyboard speaks in bytes: make codes, `F0` break prefixes, `E0`
//! extended prefixes, one eight-byte monster for Pause, and protocol replies
//! that can arrive interleaved with any of them. Turning that stream into
//! key events is arithmetic over bytes with no hardware behind it, so it
//! lives here and is unit-tested on the host; the kernel's driver feeds it
//! what the controller delivers and does nothing else.
//!
//! Wire protocol is separate from layout policy: [`Decoder`] produces
//! [`KeyEvent`]s naming physical keys, and the [`us`] module turns events
//! into characters for one layout — a second layout is a sibling module, not
//! a decoder change.
//!
//! Sequences follow the IBM PS/2 keyboard specification's scan code set 2
//! tables, as transcribed by the OSDev wiki's "Keyboard Scan Codes" page.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod decode;
mod event;
mod key;
mod shift;
mod table;
pub mod us;

pub use decode::Decoder;
pub use event::{KeyEvent, KeyState};
pub use key::Key;
pub use shift::Shift;
