//! Architectural checks for a 64-bit `iretq` to ring three, kept where every field has a host test.

use core::fmt;

use crate::Selector;

/// Bit 1 reads as one on every supported x86 processor.
const ALWAYS_ONE: u64 = 1 << 1;
/// Ring three remains preemptible after the return.
const INTERRUPT: u64 = 1 << 9;

/// Flags a user context may carry through `iretq`; system, VM86 and reserved bits are absent.
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

    /// Validate one return against the user selectors and `user_end`, the first non-user address.
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

impl fmt::Display for UserReturnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
mod tests;
