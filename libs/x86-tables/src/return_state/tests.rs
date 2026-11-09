use super::*;

const USER_CODE: Selector = Selector::new(6, 3);
const USER_DATA: Selector = Selector::new(5, 3);
const USER_END: u64 = 0x0000_8000_0000_0000;
const ORDINARY_FLAGS: u64 = ALWAYS_ONE | INTERRUPT;

fn state() -> UserReturnState {
    UserReturnState::new(
        0x4000,
        USER_CODE.bits() as u64,
        ORDINARY_FLAGS,
        0x8000,
        USER_DATA.bits() as u64,
    )
}

#[test]
fn an_ordinary_user_return_is_accepted() {
    assert_eq!(state().validate(USER_CODE, USER_DATA, USER_END), Ok(()));
}

#[test]
fn both_return_addresses_stay_in_the_low_user_half() {
    let at_edge = UserReturnState::new(
        USER_END - 1,
        USER_CODE.bits() as u64,
        ORDINARY_FLAGS,
        USER_END - 1,
        USER_DATA.bits() as u64,
    );
    assert_eq!(at_edge.validate(USER_CODE, USER_DATA, USER_END), Ok(()));

    let bad_instruction = UserReturnState::new(
        USER_END,
        USER_CODE.bits() as u64,
        ORDINARY_FLAGS,
        0x8000,
        USER_DATA.bits() as u64,
    );
    assert_eq!(
        bad_instruction.validate(USER_CODE, USER_DATA, USER_END),
        Err(UserReturnError::InstructionPointer(USER_END))
    );

    let bad_stack = UserReturnState::new(
        0x4000,
        USER_CODE.bits() as u64,
        ORDINARY_FLAGS,
        USER_END,
        USER_DATA.bits() as u64,
    );
    assert_eq!(
        bad_stack.validate(USER_CODE, USER_DATA, USER_END),
        Err(UserReturnError::StackPointer(USER_END))
    );
}

#[test]
fn both_user_selectors_are_exact() {
    for (code, stack, expected) in [
        (
            Selector::new(1, 0).bits() as u64,
            USER_DATA.bits() as u64,
            UserReturnError::CodeSelector(Selector::new(1, 0).bits() as u64),
        ),
        (
            USER_CODE.bits() as u64,
            Selector::new(2, 0).bits() as u64,
            UserReturnError::StackSelector(Selector::new(2, 0).bits() as u64),
        ),
    ] {
        let candidate = UserReturnState::new(0x4000, code, ORDINARY_FLAGS, 0x8000, stack);
        assert_eq!(
            candidate.validate(USER_CODE, USER_DATA, USER_END),
            Err(expected)
        );
    }
}

#[test]
fn user_flags_are_an_allowlist_with_required_state() {
    for flags in [
        ORDINARY_FLAGS & !ALWAYS_ONE,
        ORDINARY_FLAGS & !INTERRUPT,
        ORDINARY_FLAGS | (1 << 3),
        ORDINARY_FLAGS | (1 << 5),
        ORDINARY_FLAGS | (1 << 12),
        ORDINARY_FLAGS | (1 << 13),
        ORDINARY_FLAGS | (1 << 14),
        ORDINARY_FLAGS | (1 << 15),
        ORDINARY_FLAGS | (1 << 17),
        ORDINARY_FLAGS | (1 << 19),
        ORDINARY_FLAGS | (1 << 20),
        ORDINARY_FLAGS | (1 << 22),
        ORDINARY_FLAGS | (1 << 63),
    ] {
        let candidate = UserReturnState::new(
            0x4000,
            USER_CODE.bits() as u64,
            flags,
            0x8000,
            USER_DATA.bits() as u64,
        );
        assert_eq!(
            candidate.validate(USER_CODE, USER_DATA, USER_END),
            Err(UserReturnError::Flags(flags)),
            "accepted flags {flags:#x}"
        );
    }

    for optional in [0, 1 << 0, 1 << 8, 1 << 10, 1 << 16, 1 << 18, 1 << 21] {
        let flags = ORDINARY_FLAGS | optional;
        let candidate = UserReturnState::new(
            0x4000,
            USER_CODE.bits() as u64,
            flags,
            0x8000,
            USER_DATA.bits() as u64,
        );
        assert_eq!(candidate.validate(USER_CODE, USER_DATA, USER_END), Ok(()));
    }
}
