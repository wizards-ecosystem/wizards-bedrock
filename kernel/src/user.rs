//! The first thing that runs in ring 3.
//!
//! Not a program and not a process: a page of instructions, a page of stack,
//! and a thread that stops being the kernel's. What it proves is the boundary
//! — that the descriptors, the gate, the privilege stack and the return path
//! all agree — which is the thing every later slice builds on and the thing
//! that is hardest to debug once there is a loader in the way (D-96).
//!
//! It is deliberately not an ELF. Slice 7 brings `libs/elf64`, an initrd and
//! `spawn`; this is the layer underneath, and keeping them apart means a
//! failure here is a failure of the boundary rather than of a parser.

use wost::arch::current::task;
use wost::mm;
use x86_paging::{Flags, PAGE_SIZE};

use crate::thread::ThreadContext;
use crate::{info, mem, warn};

/// Where the first user program's page of code goes.
///
/// Low half, far from anything the kernel maps: an address a user mapping can
/// occupy is by definition one the kernel does not, and the split is bit 63
/// (`010` §10's LASS note).
pub(crate) const CODE: u64 = 0x0000_4000_0000_0000;

/// Where its stack goes, well away from the code so a run-off hits nothing.
pub(crate) const STACK: u64 = 0x0000_3000_0000_0000;

/// How far apart two programs' pages sit.
///
/// They share one address space until slice 7 gives each a table of its own,
/// so for now "another program" means "another pair of addresses". That is not
/// isolation and is not claimed to be — what the second one proves is that
/// `RSP0` follows the thread (D-99).
const APART: u64 = 0x1_0000;

/// Bytes of stack, which is one page: it makes one call and never returns.
const STACK_BYTES: u64 = PAGE_SIZE;

/// The whole program.
///
/// ```text
///   cd 80     int 0x80    call the kernel
///   eb fe     jmp .       and if it ever returns, stop here rather than
///                         running off into whatever follows
/// ```
///
/// The second instruction is unreachable when the kernel retires the caller,
/// which it does — but a program whose last instruction is "fall forwards" is
/// a program that executes whatever the page happens to contain, and this page
/// is zeroed, which decodes as `add [rax], al` and faults on a null write.
/// Stopping deliberately is cheaper than reasoning about that.
pub(crate) const PROGRAM: [u8; 4] = [0xCD, 0x80, 0xEB, 0xFE];

/// Map the program and its stack, and report where to enter.
///
/// Both pages are user-accessible and neither is both writable and executable:
/// the code page is read-only and executable, the stack is writable and never
/// executable. W^X is not relaxed for user memory — it is the reason user
/// memory exists (D-12).
fn place(context: &mut ThreadContext, which: u64) -> Option<(u64, u64)> {
    let code_at = CODE + which * APART;
    let stack_at = STACK + which * APART;

    let mut code = mm::allocate_contiguous(1, 1)?;
    if !code.zero() || !code.write_at(0, &PROGRAM) {
        return None;
    }
    let mut stack = mm::allocate_contiguous((STACK_BYTES / PAGE_SIZE) as usize, 1)?;
    if !stack.zero() {
        return None;
    }

    mem::with_active_space(context, |space| {
        space
            .map_owned(code_at, code, Flags::USER)
            .and_then(|()| {
                space.map_owned(
                    stack_at,
                    stack,
                    Flags::USER.union(Flags::WRITABLE).union(Flags::NO_EXECUTE),
                )
            })
            .ok()
    })??;
    Some((code_at, stack_at))
}

/// Cross into ring 3, and never come back on this thread.
///
/// Runs as an ordinary kernel thread until its last instruction, and is retired
/// by the syscall handler from the far side of the boundary.
///
/// No stack is arranged for the trap out: the context switch that scheduled
/// this thread already pointed `RSP0` at its own kernel stack, and will point
/// it somewhere else the moment another thread runs (D-99). That is what lets
/// there be more than one of these.
fn cross(which: u64) {
    let mut context = ThreadContext::require();

    let Some((code, stack)) = place(&mut context, which) else {
        warn!("user {which}: could not map the program");
        return;
    };

    info!("user {which}: entering ring 3 at {code:#x}");
    let refused = task::enter_user(code, stack + STACK_BYTES);
    warn!("user {which}: refused entry to ring 3: {refused}");
}

/// The first user program.
pub(crate) fn run() {
    cross(0);
}

/// A second one, at its own addresses, to show `RSP0` moves with the thread.
pub(crate) fn run_second() {
    cross(1);
}
