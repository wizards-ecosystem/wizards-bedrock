//! Shift, the one modifier the layout needs.
//! CapsLock, Ctrl and Alt arrive with the first code that needs them.

use crate::event::{KeyEvent, KeyState};
use crate::key::Key;

/// Which shift keys are currently held.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Shift {
    left: bool,
    right: bool,
}

impl Shift {
    /// Neither shift held.
    pub const fn new() -> Self {
        Self {
            left: false,
            right: false,
        }
    }

    /// Fold one event into the state; other keys change nothing.
    pub fn observe(&mut self, event: KeyEvent) {
        let held = event.state == KeyState::Pressed;
        match event.key {
            Key::ShiftLeft => self.left = held,
            Key::ShiftRight => self.right = held,
            _ => {}
        }
    }

    /// Whether either shift is held.
    pub const fn is_down(self) -> bool {
        self.left || self.right
    }
}

#[cfg(test)]
mod tests;
