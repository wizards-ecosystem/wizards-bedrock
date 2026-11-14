use super::*;

#[test]
fn the_interrupt_stack_offset_is_four_modulo_eight() {
    assert_eq!(TaskState::interrupt_stack_offset(1) % 8, 4);
}
