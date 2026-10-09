//! Architectural checks for a 64-bit return to ring three.
//!
//! `iretq` consumes selectors, addresses and flags together. Keeping their
//! pure admission rule here makes the final privileged adapter small and gives
//! every field a host witness (D-368).

use crate::Selector;

/// Bit 1 reads as one on every supported x86 processor.
const ALWAYS_ONE: u64 = 1 << 1;
/// Ring three remains preemptible after the return.
const INTERRUPT: u64 = 1 << 9;

/// Flags a 64-bit user context may carry through this kernel's `iretq`.
///
/// Condition codes, tracing, direction, resume, alignment checking and CPUID
/// discovery are user state. System fields, virtual-8086 state and every
/// reserved bit are deliberately absent.
const USER_FLAGS: u64 = (1 << 0)
    | ALWAYS_ONE
    | (1 << 2)
    | (1 << 4)
    | (1 << 6)
    | (1 << 7)
    | (1 << 8)
    | INTERRUPT
    | (1 << 10)
    | (1 << 11)
    | (1 << 16)
    | (1 << 18)
    | (1 << 21);

/// The state consumed by a 64-bit privilege return.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserReturnState {
    instruction_pointer: u64,
    code_selector: u64,
    flags: u64,
    stack_pointer: u64,
    stack_selector: u64,
}

impl UserReturnState {
    /// Capture the five architectural words in their `iretq` order.
    pub const fn new(
        instruction_pointer: u64,
        code_selector: u64,
        flags: u64,
        stack_pointer: u64,
        stack_selector: u64,
    ) -> Self {
        Self {
            instruction_pointer,
            code_selector,
            flags,
            stack_pointer,
            stack_selector,
        }
    }

    /// Validate one return against the installed user selectors and low-half
    /// address boundary.
    ///
    /// `user_end` is the first address outside admitted user register state.
    /// Passing it in keeps paging policy at the caller while this crate owns
    /// the architectural relationship among all five words.
    ///
    /// # Errors
    ///
    /// [`UserReturnError`] naming the first field that would make the return
    /// unsafe or violate the kernel's ring-three contract.
    pub const fn validate(
        self,
        user_code: Selector,
        user_data: Selector,
        user_end: u64,
    ) -> Result<(), UserReturnError> {
        if self.code_selector != user_code.bits() as u64 {
            return Err(UserReturnError::CodeSelector(self.code_selector));
        }
        if self.stack_selector != user_data.bits() as u64 {
            return Err(UserReturnError::StackSelector(self.stack_selector));
        }
        if self.instruction_pointer >= user_end {
            return Err(UserReturnError::InstructionPointer(
                self.instruction_pointer,
            ));
        }
        if self.stack_pointer >= user_end {
            return Err(UserReturnError::StackPointer(self.stack_pointer));
        }
        if self.flags & (ALWAYS_ONE | INTERRUPT) != ALWAYS_ONE | INTERRUPT
            || self.flags & !USER_FLAGS != 0
        {
            return Err(UserReturnError::Flags(self.flags));
        }
        Ok(())
    }
}

/// Why an outer-privilege frame cannot be consumed by `iretq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserReturnError {
    /// `CS` is not the one installed for 64-bit ring-three code.
    CodeSelector(u64),
    /// `SS` is not the one installed for ring-three data.
    StackSelector(u64),
    /// `RIP` is outside the admitted user half.
    InstructionPointer(u64),
    /// `RIP` is not executable user memory in the live address space.
    InstructionNotExecutable(u64),
    /// `RSP` is outside the admitted user half.
    StackPointer(u64),
    /// `RFLAGS` contains a system/reserved bit or omits a required bit.
    Flags(u64),
}

impl core::fmt::Display for UserReturnError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let (field, value) = match *self {
            Self::CodeSelector(value) => ("code selector", value),
            Self::StackSelector(value) => ("stack selector", value),
            Self::InstructionPointer(value) => ("instruction pointer", value),
            Self::InstructionNotExecutable(value) => ("non-executable instruction pointer", value),
            Self::StackPointer(value) => ("stack pointer", value),
            Self::Flags(value) => ("flags", value),
        };
        write!(f, "unsafe user return {field} {value:#x}")
    }
}

impl core::error::Error for UserReturnError {}

#[cfg(test)]
mod tests {
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
}
