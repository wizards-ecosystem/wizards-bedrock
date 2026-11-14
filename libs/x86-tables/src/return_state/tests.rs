use super::*;

const USER_CODE: Selector = Selector::new(6, 3);
const USER_DATA: Selector = Selector::new(5, 3);
const USER_END: u64 = 0x0000_8000_0000_0000;
const ORDINARY_FLAGS: u64 = ALWAYS_ONE | INTERRUPT;

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
