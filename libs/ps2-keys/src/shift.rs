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
mod tests {
    use super::*;

    fn press(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            state: KeyState::Pressed,
        }
    }

    fn release(key: Key) -> KeyEvent {
        KeyEvent {
            key,
            state: KeyState::Released,
        }
    }

    #[test]
    fn either_shift_held_reports_down() {
        let mut shift = Shift::new();
        assert!(!shift.is_down());

        shift.observe(press(Key::ShiftLeft));
        assert!(shift.is_down());

        let mut shift = Shift::new();
        shift.observe(press(Key::ShiftRight));
        assert!(shift.is_down());
    }

    /// The classic bug this struct exists to avoid: releasing one shift while
    /// the other is still held must not report both up.
    #[test]
    fn releasing_one_shift_keeps_the_other() {
        let mut shift = Shift::new();
        shift.observe(press(Key::ShiftLeft));
        shift.observe(press(Key::ShiftRight));
        shift.observe(release(Key::ShiftLeft));

        assert!(shift.is_down());

        shift.observe(release(Key::ShiftRight));
        assert!(!shift.is_down());
    }

    #[test]
    fn other_keys_change_nothing() {
        let mut shift = Shift::new();
        shift.observe(press(Key::A));
        assert!(!shift.is_down());

        shift.observe(press(Key::ShiftLeft));
        shift.observe(release(Key::A));
        assert!(shift.is_down());
    }
}
