//! What a decoded sequence means: one key, pressed or released.

use crate::key::Key;

/// Whether the key went down or came up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyState {
    /// The key went down, also what a held key repeats.
    Pressed,
    /// The key came up.
    Released,
}

/// One decoded key transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub state: KeyState,
}
