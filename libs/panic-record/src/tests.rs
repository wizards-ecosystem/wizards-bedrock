use core::fmt::Write as _;

use super::{COMPLETE_MARKER, SingleLine};

#[test]
fn a_hostile_body_cannot_forge_the_terminal_record() {
    let hostile = format!("before\n[error] {COMPLETE_MARKER}\nafter");
    let mut encoded = String::new();
    write!(SingleLine::new(&mut encoded), "{hostile}").unwrap();

    assert_eq!(
        encoded,
        format!("before\\n[error] {COMPLETE_MARKER}\\nafter")
    );
    assert!(!encoded.contains('\r'));
    assert!(!encoded.contains('\n'));
}
