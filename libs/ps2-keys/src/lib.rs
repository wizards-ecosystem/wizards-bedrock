//! PS/2 keyboard scancode set 2 decoding.
//!
//! # Examples
//!
//! ```
//! use ps2_keys::{Decoder, Key, KeyState};
//!
//! let mut decoder = Decoder::new();
//! // `1C` is A pressed; `F0 1C` is A released.
//! assert_eq!(decoder.feed(0x1C).map(|event| event.key), Some(Key::A));
//! assert!(decoder.feed(0xF0).is_none());
//! let release = decoder.feed(0x1C).expect("the break completes");
//! assert_eq!((release.key, release.state), (Key::A, KeyState::Released));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![warn(clippy::unnecessary_wraps)]
#![warn(clippy::cast_possible_truncation)]
#![warn(clippy::exhaustive_enums)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

mod arriving;
mod decode;
mod event;
mod key;
mod set1;
mod shift;
mod table;
pub mod us;

pub use arriving::{Arriving, SetProbe};
pub use decode::Decoder;
pub use event::{KeyEvent, KeyState};
pub use key::Key;
pub use shift::Shift;
