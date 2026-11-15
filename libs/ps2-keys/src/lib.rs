#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
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
