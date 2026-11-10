//! What the kernel does when the CPU raises a vector.
//!
//! Every vector arrives here — exceptions the CPU raised against itself, and
//! interrupts the local APIC delivered. An interrupt is dismissed and returned
//! from; an unexpected one still ends the machine, which is what keeps a leaked
//! legacy line from arriving as something else.
//!
//! Fault disposition follows its origin. A user-copy fault resumes at the
//! fixup (D-120), a ring-3 fault retires the program (D-102), and other faults
//! halt the kernel. Breakpoints resume; supervised restart remains absent
//! (D-14, D-31).

use core::fmt;
#[cfg(feature = "selftest")]
use core::sync::atomic::Ordering;

use trap_cause::TrapCause;
use wost::arch::current::apic;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::usercopy;

#[cfg(feature = "selftest")]
use wost::arch::current::paging;
#[cfg(feature = "selftest")]
use wost::arch::current::segment;

use super::vector;

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    InvalidUserReturnKind, Record, kernel_stack_selector_on_last_syscall, last_breakpoint,
    last_invalid_user_return, last_ping, last_syscall, last_user_fault, reclaimed_abandoned_eoi,
    rejected_duplicate_eoi_claims, smap_survived_the_last_syscall, spurious_count,
    user_copy_recoveries,
};
#[cfg(feature = "selftest")]
use selftest::{
    KERNEL_STACK_SELECTOR_ON_ENTRY, LAST_BREAKPOINT, LAST_INVALID_USER_RETURN,
    LAST_INVALID_USER_RETURN_KIND, LAST_PING, LAST_SYSCALL, LAST_USER_FAULT,
    REJECTED_DUPLICATE_EOI_CLAIMS, SMAP_ON_ENTRY, SPURIOUS_OBSERVATIONS, USER_COPY_RECOVERIES,
    invalid_user_return_kind, maybe_reclaim_ping,
};

static HANDLERS: interrupt::Handlers =
    interrupt::Handlers::new(on_trap, reject_invalid_user_return);

/// Route every vector to this module.
pub(crate) fn init() {
    interrupt::init(&HANDLERS, Some(vector::SYSCALL));
}

fn reject_invalid_user_return(frame: &TrapFrame, error: interrupt::UserReturnError) -> ! {
    #[cfg(feature = "selftest")]
    {
        LAST_INVALID_USER_RETURN_KIND.store(invalid_user_return_kind(error), Ordering::Relaxed);
        LAST_INVALID_USER_RETURN.record(frame);
    }
    if crate::thread::retire_from_trap(frame) {
        unreachable!("retiring an invalid user return resumed its context");
    }
    panic!("invalid user return could not be retired: {error:?}");
}

/// The kernel's response to every vector.
///
/// Nothing here logs. A fault can be raised while a lock is held, so taking the
/// console lock in trap context can deadlock against the interrupted code.
fn on_trap(frame: &TrapFrame) {
    // Dispatched on the decoded cause, not the raw vector (D-81): the
    // architecture's meaning is read once, in `libs/trap-cause`, host-tested.
    let cause = TrapCause::decode(frame.vector(), frame.error_code());
    match cause {
        TrapCause::Fault(trap_cause::Fault::Breakpoint) => {
            #[cfg(feature = "selftest")]
            LAST_BREAKPOINT.record(frame);
        }

        TrapCause::Interrupt(raw) if raw == vector::PING.get() => {
            let eoi = claim_eoi(frame, vector::PING);
            #[cfg(feature = "selftest")]
            {
                paging::observe_activation_publication_for_test();
                if apic::claim_eoi(frame, vector::PING).is_none() {
                    REJECTED_DUPLICATE_EOI_CLAIMS.fetch_add(1, Ordering::Relaxed);
                }
                let eoi = maybe_reclaim_ping(frame, eoi);
                LAST_PING.record(frame);
                if let Some(eoi) = eoi {
                    dismiss(eoi);
                }
            }
            #[cfg(not(feature = "selftest"))]
            dismiss(eoi);
        }

        TrapCause::Interrupt(raw) if raw == vector::TIMER.get() => {
            let eoi = claim_eoi(frame, vector::TIMER);
            crate::time::on_expiry();
            // Dismissed before any switch: the APIC holds this priority class
            // until EOI, and a switched-in thread would hold every expiry hostage.
            dismiss(eoi);
            crate::thread::reschedule(frame);
        }

        TrapCause::Interrupt(raw) if raw == vector::KEYBOARD.get() => {
            let eoi = claim_eoi(frame, vector::KEYBOARD);
            let woke = crate::input::on_interrupt();
            dismiss(eoi);
            // A thread woken by a keystroke must not wait for the next
            // expiry: there may not be one.
            if woke {
                crate::thread::reschedule(frame);
            }
        }

        // The CPU's saved privilege decides: a kernel-context `int 0x80` must
        // not be served as a ring-3 call (D-96, D-124).
        TrapCause::Interrupt(raw)
            if raw == vector::SYSCALL.get() && frame.code_selector() & 0b11 == 3 =>
        {
            #[cfg(feature = "selftest")]
            KERNEL_STACK_SELECTOR_ON_ENTRY.store(
                u64::from(segment::stack_selector().bits()),
                Ordering::Release,
            );
            #[cfg(feature = "selftest")]
            LAST_SYSCALL.record(frame);
            // Whether SMAP is on inside the handler, after the entry path ran,
            // which is what D-101 is about.
            #[cfg(feature = "selftest")]
            SMAP_ON_ENTRY.store(u64::from(!cpu::access_check_is_off()), Ordering::Release);
            crate::user::call(frame);
        }

        // Not dismissed: a spurious interrupt sets no in-service bit, so an
        // EOI here would retire a real interrupt underneath it.
        TrapCause::Interrupt(raw) if raw == vector::SPURIOUS.get() => {
            #[cfg(feature = "selftest")]
            SPURIOUS_OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
        }

        TrapCause::Interrupt(raw) if raw == vector::ERROR.get() => panic!(
            "local apic error {:#x} at {:#018x}",
            apic::error_status(),
            frame.instruction_pointer()
        ),

        // The controlled ring-0 write-protection probe has an exact error code
        // and private recovery interval; ordinary kernel faults remain fatal.
        #[cfg(feature = "selftest")]
        TrapCause::Fault(trap_cause::Fault::PageFault(fault))
            if cpu::recover_write_protection_probe(frame, fault) => {}

        // Only a ring-0 fault inside the user-copy interval recovers; a ring-3
        // fault at the same address is the program's (D-120, D-126).
        TrapCause::Fault(trap_cause::Fault::PageFault(fault))
            if usercopy::recover(frame, fault) =>
        {
            #[cfg(feature = "selftest")]
            USER_COPY_RECOVERIES.fetch_add(1, Ordering::Relaxed);
        }

        // Other survivable ring-3 faults retire the program, not the machine.
        TrapCause::Fault(fault) if is_the_programs(frame, fault) => {
            #[cfg(feature = "selftest")]
            LAST_USER_FAULT.record(frame);
            if !crate::thread::retire_from_trap(frame) {
                panic!("{}", Report { frame, cause });
            }
        }

        _ => panic!("{}", Report { frame, cause }),
    }
}

fn claim_eoi(frame: &TrapFrame, vector: x86_apic::vector::InterruptVector) -> apic::EoiToken<'_> {
    let Some(token) = apic::claim_eoi(frame, vector) else {
        panic!(
            "vector {:#x} has no matching APIC in-service authority",
            vector.get()
        );
    };
    token
}

fn dismiss(token: apic::EoiToken<'_>) {
    if apic::end_of_interrupt(token).is_err() {
        panic!("APIC in-service authority changed before dismissal");
    }
}

/// Whether this fault is a program's to die of rather than the machine's.
///
/// Two conditions, and both are needed. It must have come from ring 3 — the
/// kernel faulting is still a kernel bug and still terminal. And it must be a
/// fault a program can cause and the machine can survive: a double fault means
/// the fault path itself failed, and a machine check is the hardware reporting
/// its own failure. Neither becomes survivable because a program was running.
fn is_the_programs(frame: &TrapFrame, fault: trap_cause::Fault) -> bool {
    let from_ring_three = frame.code_selector() & 0b11 == 3;
    let survivable = !matches!(
        fault,
        trap_cause::Fault::DoubleFault
            | trap_cause::Fault::MachineCheck
            | trap_cause::Fault::NonMaskable
    );
    from_ring_three && survivable
}

/// A fault formatted for the panic message.
struct Report<'a> {
    frame: &'a TrapFrame,
    cause: TrapCause,
}

impl fmt::Display for Report<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frame = self.frame;
        let name = self.cause.name();
        let vector = self.cause.vector();

        write!(
            f,
            "{} (vector {}, error {:#x}) at {:#018x}, cs {:#x}, flags {:#x}, rsp {:#018x}",
            name,
            vector,
            frame.error_code(),
            frame.instruction_pointer(),
            frame.code_selector(),
            frame.flags(),
            frame.stack_pointer(),
        )?;

        // CR2 belongs only to page faults; other vectors leave its old value.
        if let TrapCause::Fault(trap_cause::Fault::PageFault(fault)) = self.cause {
            write!(
                f,
                ", faulting address {:#018x}: {fault:?}",
                cpu::faulting_address()
            )?;
        }

        Ok(())
    }
}
