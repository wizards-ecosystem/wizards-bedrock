use core::arch::asm;
use core::fmt;

use x86_paging::{Flags, PAGE_SIZE};

use super::{cpu, interrupt, paging};

pub use super::paging::USER_END;

// Explicit assembly gives the faulting instructions a stable recovery range.
::core::arch::global_asm!(
    ".globl wost_copy_across",
    "wost_copy_across:",
    "  mov rcx, rdx",
    "  test rcx, rcx",
    "  jz 3f",
    // Only these two instructions may fault, and such a fault is the program's bad pointer.
    ".globl wost_copy_fault_start",
    "wost_copy_fault_start:",
    "  mov al, [rsi]",
    "  mov [rdi], al",
    ".globl wost_copy_fault_end",
    "wost_copy_fault_end:",
    "  inc rsi",
    "  inc rdi",
    "  dec rcx",
    "  jnz wost_copy_fault_start",
    "3:",
    "  xor rax, rax",
    "  ret",
    // A recovered fault resumes here with `rcx` holding what was left: the trap restored it.
    ".globl wost_copy_fixup",
    "wost_copy_fixup:",
    "  mov rax, rcx",
    "  ret",
);

unsafe extern "C" {
    // Returns the bytes left uncopied, non-zero only after a recovered fault.
    fn wost_copy_across(destination: *mut u8, source: *const u8, len: usize) -> usize;

    static wost_copy_fault_start: u8;
    static wost_copy_fault_end: u8;
    static wost_copy_fixup: u8;
}

#[derive(Debug)]
pub(super) struct RecoveryTarget(u64);

impl RecoveryTarget {
    fn fixup() -> Self {
        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)
    }

    pub(super) const fn address(&self) -> u64 {
        self.0
    }
}

#[must_use = "a refused user-copy recovery changed no trap frame"]
pub fn recover(frame: &interrupt::TrapFrame, fault: trap_cause::PageFault) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;
    let end = core::ptr::addr_of!(wost_copy_fault_end) as u64;
    let attempt = trap_cause::RecoveryAttempt {
        vector: u64::from(frame.vector()),
        frame_error_code: frame.error_code(),
        code_selector: frame.code_selector(),
        instruction_pointer: at,
        page_fault: fault,
        faulting_address: cpu::faulting_address(),
    };
    let rule = trap_cause::RecoveryRule {
        fault_addresses: 0..USER_END,
        instruction_addresses: start..end,
    };
    if !attempt.is_admitted_by(&rule) {
        return false;
    }
    interrupt::resume_usercopy(frame, RecoveryTarget::fixup())
}

#[cfg(feature = "selftest")]
pub fn fault_window() -> (u64, u64) {
    (
        core::ptr::addr_of!(wost_copy_fault_start) as u64,
        core::ptr::addr_of!(wost_copy_fault_end) as u64,
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum UserCopyError {
    NotUserMemory(u64),
    Wraps(u64),
    Vanished(usize),
    Unmapped(u64),
    ReadOnly(u64),
}

impl fmt::Display for UserCopyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotUserMemory(at) => write!(formatter, "{at:#x} is not user memory"),
            Self::Wraps(at) => write!(formatter, "a range at {at:#x} wraps"),
            Self::Vanished(left) => {
                write!(
                    formatter,
                    "the range stopped being mapped with {left} bytes left"
                )
            }
            Self::Unmapped(at) => write!(formatter, "{at:#x} is not mapped for ring 3"),
            Self::ReadOnly(at) => write!(formatter, "{at:#x} is not writable from ring 3"),
        }
    }
}

impl core::error::Error for UserCopyError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Direction {
    #[cfg(feature = "selftest")]
    FromUser,
    ToUser,
}

#[derive(Debug)]
struct Window {
    // False without SMAP, where `stac` would raise #UD.
    opened: bool,
}

impl Window {
    fn open() -> Self {
        assert!(
            !crate::sync::holds_leaf(),
            "a leaf guard is held, so no user copy may run"
        );
        let opened = cpu::Protections::enabled().smap;
        if opened {
            // SAFETY: `stac` runs only where `cpuid` reported SMAP. It changes `AC` and access
            // semantics, so no `nomem` or `preserves_flags`, and the copy stays inside the window.
            unsafe { asm!("stac", options(nostack)) };
        }
        Self { opened }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.opened {
            // SAFETY: as `open`; `clac` clears the `AC` flag `stac` set and, like `stac`, is a
            // memory barrier, since an access moved after it would fault.
            unsafe { asm!("clac", options(nostack)) };
        }
    }
}

fn admits(address: u64, len: usize, direction: Direction) -> Result<(), UserCopyError> {
    if len == 0 {
        return Ok(());
    }
    let end = address
        .checked_add(len as u64)
        .ok_or(UserCopyError::Wraps(address))?;
    if end > USER_END {
        return Err(UserCopyError::NotUserMemory(address));
    }

    let mut page = address & !(PAGE_SIZE - 1);
    while page < end {
        match paging::translate(page) {
            Some((_, flags)) if flags.contains(Flags::USER) => {
                if direction == Direction::ToUser && !flags.contains(Flags::WRITABLE) {
                    return Err(UserCopyError::ReadOnly(page));
                }
            }
            _ => return Err(UserCopyError::Unmapped(page)),
        }
        page = page
            .checked_add(PAGE_SIZE)
            .ok_or(UserCopyError::Wraps(page))?;
    }
    Ok(())
}

#[cfg(feature = "selftest")]
pub fn copy_from_user(from: u64, into: &mut [u8]) -> Result<(), UserCopyError> {
    admits(from, into.len(), Direction::FromUser)?;
    if into.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: `admits` proved each source page live and user-readable, `into` is a disjoint live
    // kernel slice, the window covers exactly this call, and the fixup returns any remainder.
    let left = unsafe { wost_copy_across(into.as_mut_ptr(), from as *const u8, into.len()) };
    drop(window);
    if left != 0 {
        return Err(UserCopyError::Vanished(left));
    }
    Ok(())
}

pub fn copy_to_user(onto: u64, from: &[u8]) -> Result<(), UserCopyError> {
    admits(onto, from.len(), Direction::ToUser)?;
    if from.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: as `copy_from_user`, and `admits` also established every page writable from ring 3.
    let left = unsafe { wost_copy_across(onto as *mut u8, from.as_ptr(), from.len()) };
    drop(window);
    if left != 0 {
        return Err(UserCopyError::Vanished(left));
    }
    Ok(())
}
