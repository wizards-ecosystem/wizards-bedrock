use crate::event::{KeyEvent, KeyState};
use crate::key::Key;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Shift {
    left: bool,
    right: bool,
}

impl Shift {
    pub const fn new() -> Self {
        Self {
            left: false,
            right: false,
        }
    }

    pub fn observe(&mut self, event: KeyEvent) {
        let held = event.state == KeyState::Pressed;
        match event.key {
            Key::ShiftLeft => self.left = held,
            Key::ShiftRight => self.right = held,
            _ => {}
        }
    }

    pub(crate) const fn is_down(self) -> bool {
        self.left || self.right
    }
}
