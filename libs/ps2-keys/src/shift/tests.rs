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
