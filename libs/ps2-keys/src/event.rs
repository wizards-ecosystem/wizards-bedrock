use crate::key::Key;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    pub key: Key,
    pub state: KeyState,
}
