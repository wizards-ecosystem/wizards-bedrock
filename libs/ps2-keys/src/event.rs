//! What a decoded sequence means: one key, pressed or released.

use crate::key::Key;

/// Whether the key went down or came up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// The key went down — also what a held key repeats.
    Pressed,
    /// The key came up.
    Released,
}

/// One decoded key transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// Which key.
    pub key: Key,
    /// Which way it went.
    pub state: KeyState,
}
