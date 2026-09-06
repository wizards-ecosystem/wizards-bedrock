//! Privilege-boundary copies under SMAP.
//!
//! Only this module opens the access window. It range-checks and walks every
//! page before copying; only an exact supervisor fault in its assembly copy
//! interval may resume at the private fixup (D-81, D-83, D-120).

use core::arch::asm;
use core::fmt;

use x86_paging::{Flags, PAGE_SIZE};

use super::{cpu, interrupt, paging};

/// The first address that is not user memory.
///
/// The boundary [`paging`] maps against, re-exported rather than transcribed:
/// the range a copy may touch and the range a mapping may cover are the same
/// range, and a second literal is a way for them to stop being (D-124).
pub use super::paging::USER_END;

// Explicit assembly gives the faulting instructions a stable recovery range
// that compiler-generated copying cannot (D-120).
::core::arch::global_asm!(
    ".globl wost_copy_across",
    "wost_copy_across:",
    // rdi = destination, rsi = source, rdx = length.
    "  mov rcx, rdx",
    "  test rcx, rcx",
    "  jz 3f",
    // Everything between these two labels can fault, and a fault in it is the
    // program's bad pointer rather than the kernel's bug.
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
    // Nothing left uncopied.
    "  xor rax, rax",
    "  ret",
    // Where a recovered fault resumes. `rcx` still holds what was left, and
    // the trap restored it along with everything else.
    ".globl wost_copy_fixup",
    "wost_copy_fixup:",
    "  mov rax, rcx",
    "  ret",
);

unsafe extern "C" {
    /// Copy `len` bytes, returning how many were **not** copied.
    ///
    /// Zero on success. Non-zero only where a fault was recovered, which means
    /// the range stopped being mapped between the walk and the copy.
    fn wost_copy_across(destination: *mut u8, source: *const u8, len: usize) -> usize;

    /// The first instruction of the copy that can fault.
    static wost_copy_fault_start: u8;
    /// One past the last.
    static wost_copy_fault_end: u8;
    /// Where a recovered fault resumes.
    static wost_copy_fixup: u8;
}

/// The only instruction pointer a recovered user-copy fault may resume at.
pub(super) struct RecoveryTarget(u64);

impl RecoveryTarget {
    fn fixup() -> Self {
        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)
    }

    pub(super) const fn address(&self) -> u64 {
        self.0
    }
}

/// Resume a faulting user copy at its fixup, if that is what faulted.
///
/// A mapping may disappear between the walk and copy. Nothing outside the
/// private two-instruction interval is recoverable. `false` leaves the frame
/// unchanged.
#[must_use = "a refused user-copy recovery changed no trap frame"]
pub fn recover(frame: &interrupt::TrapFrame, fault: trap_cause::PageFault) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;
    let end = core::ptr::addr_of!(wost_copy_fault_end) as u64;
    let attempt = trap_cause::RecoveryAttempt {
        vector: frame.vector(),
        frame_error_code: frame.error_code(),
        code_selector: frame.code_selector(),
        instruction_pointer: at,
        page_fault: fault,
        faulting_address: cpu::faulting_address(),
    };
    let rule = trap_cause::RecoveryRule {
        fault: trap_cause::RecoveryFault::SupervisorData,
        fault_addresses: 0..USER_END,
        instruction_addresses: start..end,
    };
    if !attempt.is_admitted_by(&rule) {
        return false;
    }
    interrupt::resume_usercopy(frame, RecoveryTarget::fixup())
}

/// The instruction range a user copy can fault inside.
#[cfg(feature = "selftest")]
pub fn fault_window() -> (u64, u64) {
    (
        core::ptr::addr_of!(wost_copy_fault_start) as u64,
        core::ptr::addr_of!(wost_copy_fault_end) as u64,
    )
}

/// Why a copy across the boundary was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserCopyError {
    /// The range is not wholly inside user memory.
    NotUserMemory(u64),
    /// The range wraps, so it does not describe a span at all.
    Wraps(u64),
    /// The range stopped being mapped between the walk and the copy.
    /// Names how many bytes were left.
    Vanished(usize),
    /// A page of it is not mapped, or not reachable from ring 3.
    Unmapped(u64),
    /// A page the copy would write is not writable from ring 3.
    ReadOnly(u64),
}

impl fmt::Display for UserCopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NotUserMemory(at) => write!(f, "{at:#x} is not user memory"),
            Self::Wraps(at) => write!(f, "a range at {at:#x} wraps"),
            Self::Vanished(left) => {
                write!(f, "the range stopped being mapped with {left} bytes left")
            }
            Self::Unmapped(at) => write!(f, "{at:#x} is not mapped for ring 3"),
            Self::ReadOnly(at) => write!(f, "{at:#x} is not writable from ring 3"),
        }
    }
}

impl core::error::Error for UserCopyError {}

/// What a page must permit for the copy being made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// The kernel reads user memory.
    FromUser,
    /// The kernel writes user memory.
    ToUser,
}

/// The window in which ring-0 code may touch ring-3 memory.
///
/// Scoped, and private, so it cannot be held across anything: the whole of its
/// life is one `wost_copy_across` invocation. An open window is the CPU agreeing to
/// dereference whatever a user pointer happens to be, and the shorter it is,
/// the smaller the class of bugs that can be a privilege escalation rather
/// than a fault.
struct Window {
    /// False where SMAP is absent, in which case `stac` would be a `#UD` and
    /// the access needs no permission anyway.
    opened: bool,
}

impl Window {
    fn open() -> Self {
        let opened = cpu::Protections::enabled().smap;
        if opened {
            // SAFETY: `stac` sets a flag permitting ring-0 access to user
            // pages, and only runs where `cpuid` reported SMAP. It changes
            // memory-access semantics and `EFLAGS.AC`, so neither `nomem` nor
            // `preserves_flags` is valid and the copy stays inside the window
            // (D-97).
            unsafe { asm!("stac", options(nostack)) };
        }
        Self { opened }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.opened {
            // SAFETY: as `open`; `clac` clears the flag that one set, and it
            // is as much a memory barrier as `stac` for the same reason —
            // an access moved *after* it would fault.
            unsafe { asm!("clac", options(nostack)) };
        }
    }
}

/// Check that `len` bytes at `address` are user memory the copy may touch.
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

/// Copy `into.len()` bytes from user memory at `from`.
///
/// # Errors
///
/// See [`UserCopyError`]. Validation failures move nothing;
/// [`UserCopyError::Vanished`] may leave a partial copy.
pub fn copy_from_user(from: u64, into: &mut [u8]) -> Result<(), UserCopyError> {
    admits(from, into.len(), Direction::FromUser)?;
    if into.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: `admits` proved every source page live and user-readable; `into`
    // is a disjoint live kernel slice. The window covers exactly this call, and
    // the fixup returns a remainder if a source page disappears (D-120).
    let left = unsafe { wost_copy_across(into.as_mut_ptr(), from as *const u8, into.len()) };
    drop(window);
    if left != 0 {
        return Err(UserCopyError::Vanished(left));
    }
    Ok(())
}

/// Drive one fixed user-copy fault through the private recovery window.
///
/// `true` means the full probe remained and its private destination did not
/// change.
#[cfg(feature = "selftest")]
pub fn drive_fault_recovery_for_test() -> bool {
    const HOLE: u64 = 0x0000_6000_0000_0000;
    const UNCHANGED: [u8; 8] = [0xa5; 8];

    let disabled = interrupt::disable();
    if paging::translate(HOLE).is_some() {
        disabled.restore();
        return false;
    }
    let mut scratch = UNCHANGED;
    let window = Window::open();
    // SAFETY: `HOLE` is a canonical user address just observed unmapped under
    // interrupt exclusion. The assembly's exact fault interval resumes at its
    // fixup, and `scratch` is a live, disjoint destination.
    let left = unsafe { wost_copy_across(scratch.as_mut_ptr(), HOLE as *const u8, scratch.len()) };
    drop(window);
    let exact = left == scratch.len() && scratch == UNCHANGED;
    disabled.restore();
    exact
}

/// Copy `from` into user memory at `onto`.
///
/// # Errors
///
/// See [`UserCopyError`]. Validation failures move nothing;
/// [`UserCopyError::Vanished`] may leave a partial copy.
pub fn copy_to_user(onto: u64, from: &[u8]) -> Result<(), UserCopyError> {
    admits(onto, from.len(), Direction::ToUser)?;
    if from.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: as `copy_from_user`, and `admits` additionally established that
    // every page is writable from ring 3.
    let left = unsafe { wost_copy_across(onto as *mut u8, from.as_ptr(), from.len()) };
    drop(window);
    if left != 0 {
        return Err(UserCopyError::Vanished(left));
    }
    Ok(())
}
