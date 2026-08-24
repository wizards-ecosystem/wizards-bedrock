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

use core::arch::asm;
use core::fmt;

use x86_paging::{Flags, PAGE_SIZE};

use super::{cpu, paging};

/// The first address that is not user memory.
///
/// The canonical hole begins here, and everything above it is the kernel's.
/// This is the range check that replaces the shift.
pub const USER_END: u64 = 0x0000_8000_0000_0000;

/// Why a copy across the boundary was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserCopyError {
    /// The range is not wholly inside user memory.
    NotUserMemory(u64),
    /// The range wraps, so it does not describe a span at all.
    Wraps(u64),
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
/// See [`UserCopyError`]. Nothing is copied unless the whole range checks out.
pub fn copy_from_user(from: u64, into: &mut [u8]) -> Result<(), UserCopyError> {
    admits(from, into.len(), Direction::FromUser)?;
    if into.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: `admits` walked every page of the range and found it present and
    // user-accessible in the live tables, `into` is a live slice of at least
    // this length, and the two cannot overlap — one is user memory and the
    // other is the kernel's. The window is open for exactly this call.
    unsafe { core::ptr::copy_nonoverlapping(from as *const u8, into.as_mut_ptr(), into.len()) };
    drop(window);
    Ok(())
}

/// Copy `from` into user memory at `onto`.
///
/// # Errors
///
/// See [`UserCopyError`]. Nothing is copied unless the whole range checks out,
/// including that every page of it is writable from ring 3.
pub fn copy_to_user(onto: u64, from: &[u8]) -> Result<(), UserCopyError> {
    admits(onto, from.len(), Direction::ToUser)?;
    if from.is_empty() {
        return Ok(());
    }

    let window = Window::open();
    // SAFETY: as `copy_from_user`, and `admits` additionally established that
    // every page is writable from ring 3.
    unsafe { core::ptr::copy_nonoverlapping(from.as_ptr(), onto as *mut u8, from.len()) };
    drop(window);
    Ok(())
}
