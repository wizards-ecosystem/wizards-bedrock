//! Tracking the one modifier the layout needs.
//!
//! Shift only. CapsLock is a toggle entangled with an LED command protocol,
//! and Ctrl/Alt have no consumer yet — each arrives with the first code that
//! needs it, per the no-speculative-API rule.

use crate::event::{KeyEvent, KeyState};
use crate::key::Key;

/// Which shift keys are currently held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

    /// Fold one event into the state. Non-shift keys change nothing.
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
