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

use wost::arch::current::paging::MappedStack;
use wost::arch::current::{segment, task};
use wost::mm;
use x86_paging::{Flags, PAGE_SIZE};

use crate::thread::ThreadContext;
use crate::{info, mem, warn};

/// Where the user program's single page of code goes.
///
/// Low half, far from anything the kernel maps: an address a user mapping can
/// occupy is by definition one the kernel does not, and the split is bit 63
/// (`010` §10's LASS note).
const CODE: u64 = 0x0000_4000_0000_0000;

/// Where its stack goes, a page below the code so a run-off hits nothing.
const STACK: u64 = 0x0000_3000_0000_0000;

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
const PROGRAM: [u8; 4] = [0xCD, 0x80, 0xEB, 0xFE];

/// Map the program and its stack, and report where to enter.
///
/// Both pages are user-accessible and neither is both writable and executable:
/// the code page is read-only and executable, the stack is writable and never
/// executable. W^X is not relaxed for user memory — it is the reason user
/// memory exists (D-12).
fn place(context: &mut ThreadContext) -> Option<()> {
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
            .map_owned(CODE, code, Flags::USER)
            .and_then(|()| {
                space.map_owned(
                    STACK,
                    stack,
                    Flags::USER.union(Flags::WRITABLE).union(Flags::NO_EXECUTE),
                )
            })
            .ok()
    })?
}

/// Where the trap out of ring 3 lands.
///
/// A stack of its own rather than the calling thread's, and that is a
/// limitation rather than a design: `RSP0` is set once here, so **one thread
/// may be in ring 3 at a time**. Per-thread `RSP0`, updated by the scheduler
/// on every switch, arrives with the processes that need it in slice 7.
fn kernel_landing(context: &mut ThreadContext) -> Option<MappedStack> {
    mem::map_guarded_stack(context, crate::thread::user_trap_stack_layout()?)
}

/// Cross into ring 3, and never come back on this thread.
///
/// Runs as an ordinary kernel thread until its last instruction. The thread is
/// retired by the syscall handler, from the far side of the boundary.
pub(crate) fn run() {
    let mut context = ThreadContext::require();

    if place(&mut context).is_none() {
        warn!("user: could not map the program");
        return;
    }
    let Some(landing) = kernel_landing(&mut context) else {
        warn!("user: could not map a stack for its traps");
        return;
    };
    if !segment::set_kernel_stack(&landing) {
        warn!("user: the trap stack is not in the installed space");
        return;
    }

    info!("user: entering ring 3 at {CODE:#x}");
    let refused = task::enter_user(CODE, STACK + STACK_BYTES);
    warn!("user: refused entry to ring 3: {refused}");
}
