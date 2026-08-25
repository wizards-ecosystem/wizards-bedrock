//! Moving bytes across the privilege boundary.
//!
//! SMAP makes ring 0 unable to touch ring-3 memory unless it says so, and
//! `stac`/`clac` are how it says so. D-81 calls this the user-copy ABI, and
//! that is the right framing: the guard is not a helper the copy functions use,
//! it is the reason there are copy functions at all. Nothing else in this
//! kernel may open the window.
//!
//! **Range-checked, never `>> 47 == 0`** (D-83). The idiom everyone writes is
//! correct today and wrong under LAM, under five-level paging, and under
//! ChkTag, all of which put non-address bits in the top of a pointer. A range
//! check is the same length and right in every case.
//!
//! Every page of the range is walked before a byte moves: present, `USER`, and
//! writable where the copy writes. A user pointer that names kernel memory, or
//! nothing, is refused rather than faulted on — a fault here is terminal
//! (D-31), and "the program passed a bad pointer" is not a reason to stop the
//! machine.

use core::arch::{asm, global_asm};
use core::fmt;

use x86_paging::{Flags, PAGE_SIZE};

use super::{cpu, interrupt, paging};

/// The first address that is not user memory.
///
/// The boundary [`paging`] maps against, re-exported rather than transcribed:
/// the range a copy may touch and the range a mapping may cover are the same
/// range, and a second literal is a way for them to stop being (D-124).
pub use super::paging::USER_END;

// The copy itself, written out so that the instructions which can fault have
// addresses this crate knows.
//
// A compiler-generated `copy_nonoverlapping` has no stable instruction range,
// so nothing could say "a fault *here* is recoverable" about one. That is the
// whole reason this is assembly and not a loop in Rust (D-120).
//
// Byte at a time, which is slower than the compiler's copy and is the price.
// The ranges crossed here are a few bytes to a page; correctness at the
// boundary is worth more than the difference, and a wider copy would need one
// fixup per width.
global_asm!(
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

/// Resume a faulting user copy at its fixup, if that is what faulted.
///
/// **The premise D-97 states, finally acted on: the program is the untrusted
/// party.** Every page is walked before a byte moves, so a bad pointer is
/// refused rather than faulted on — but the walk and the copy are two moments,
/// and interrupts are on between them. A thread preempted mid-copy whose pages
/// are then unmapped faults on memory that was there when it looked. Before
/// D-120 that was terminal: a program could stop the machine by arranging it.
///
/// Nothing else is recoverable. The range is two instructions long and is the
/// only place in this kernel that dereferences a user pointer, so a fault
/// anywhere else is still a kernel bug and still stops.
///
/// `false` if the fault was not in the copy, in which case nothing is changed.
pub fn recover(frame: &interrupt::TrapFrame) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;
    let end = core::ptr::addr_of!(wost_copy_fault_end) as u64;
    if at < start || at >= end {
        return false;
    }
    let fixup = core::ptr::addr_of!(wost_copy_fixup) as u64;
    interrupt::resume_at(frame, fixup)
}

/// The instruction range a user copy can fault inside.
///
/// Exported for one check, which spawns a ring-3 thread that jumps here. That
/// fault must retire the program rather than be *recovered*: the recovery arm
/// runs before the ring-3 test and until D-126 never looked at the saved `CS`,
/// so a program could aim `RIP` at this range and have the kernel resume it at
/// the fixup. Knowing the range is not authority — ring 3 cannot execute
/// kernel memory, which is the point — and a check that hardcoded the address
/// would be checking a number rather than the window.
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
    ///
    /// Recovered rather than fatal (D-120): the walk and the copy are two
    /// moments and interrupts are on between them, so a page can go away in
    /// the gap. Names how many bytes were left.
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
/// life is one `copy_nonoverlapping`. An open window is the CPU agreeing to
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
            // pages, and it is only reached where `cpuid` reported SMAP —
            // without it the instruction does not exist.
            //
            // Neither `nomem` nor `preserves_flags`, and both were wrong
            // before the release build caught it. `stac` changes whether a
            // later memory access faults, so `nomem` lets the optimiser move
            // the copy *out* of the window — which it did, and the machine
            // took a protection violation reading the page it had just been
            // given permission for. And the flag it sets is `EFLAGS.AC`, so
            // it preserves nothing (D-97).
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
/// See [`UserCopyError`]. Every page is walked before a byte moves, so a range
/// that does not check out is refused with nothing copied.
///
/// [`UserCopyError::Vanished`] is the exception and the reason this is not
/// stated as all-or-nothing: the walk and the copy are two moments, so a page
/// can stop being mapped in the gap. `into` then holds however much was moved
/// before the fault, and the error names how many bytes were left (D-120).
pub fn copy_from_user(from: u64, into: &mut [u8]) -> Result<(), UserCopyError> {
    admits(from, into.len(), Direction::FromUser)?;
    if into.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: `admits` walked every page of the range and found it present and
    // user-accessible in the live tables, `into` is a live slice of at least
    // this length, and the two cannot overlap — one is user memory and the
    // other is the kernel's. The window is open for exactly this call, and a
    // page that goes away in between resumes at the fixup rather than faulting
    // through (D-120).
    let left = unsafe { wost_copy_across(into.as_mut_ptr(), from as *const u8, into.len()) };
    drop(window);
    if left != 0 {
        return Err(UserCopyError::Vanished(left));
    }
    Ok(())
}

/// Copy `len` bytes from user memory at `from`, **without walking the tables**.
///
/// Range-checked, so it can never touch the kernel's half — but not walked, so
/// whether the pages are there is discovered by trying. A fault resumes at the
/// fixup and this reports how many bytes were left (D-120).
///
/// **Safe, and the fixup is why.** Dereferencing an arbitrary user address is
/// memory-safe once a fault in the copy is recoverable: the worst outcome is
/// a short copy and a number saying so. That is the property an exception
/// table buys, and it is what Linux's `copy_from_user` has always relied on
/// rather than pre-walking.
///
/// The walked forms are kept because they give a *better answer* — a program
/// that passed an unmapped pointer learns that, rather than learning that some
/// bytes did not move — and because a walk is a second line of defence that
/// costs nothing on the sizes this kernel copies.
///
/// # Errors
///
/// [`UserCopyError::NotUserMemory`] or [`UserCopyError::Wraps`] for a range
/// outside ring 3's half, and [`UserCopyError::Vanished`] naming what was left
/// where a page was not there.
pub fn copy_from_user_unwalked(from: u64, into: &mut [u8]) -> Result<(), UserCopyError> {
    let end = from
        .checked_add(into.len() as u64)
        .ok_or(UserCopyError::Wraps(from))?;
    if end > USER_END {
        return Err(UserCopyError::NotUserMemory(from));
    }
    if into.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: the range is wholly inside ring 3's half, `into` is a live slice
    // of at least this length, and the two cannot overlap. A page that is not
    // there faults and resumes at the fixup rather than through this call,
    // which is what makes the absence of a walk sound.
    let left = unsafe { wost_copy_across(into.as_mut_ptr(), from as *const u8, into.len()) };
    drop(window);
    if left != 0 {
        return Err(UserCopyError::Vanished(left));
    }
    Ok(())
}

/// Copy `from` into user memory at `onto`.
///
/// # Errors
///
/// See [`UserCopyError`]. Every page is walked before a byte moves — including
/// that each is writable from ring 3 — so a range that does not check out is
/// refused with nothing copied.
///
/// [`UserCopyError::Vanished`] is the exception, as for [`copy_from_user`]:
/// the page can go away between the walk and the copy, leaving user memory
/// partly written and naming how many bytes were left (D-120).
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
