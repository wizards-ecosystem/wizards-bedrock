use core::fmt;
#[cfg(feature = "selftest")]
use core::sync::atomic::Ordering;

use boot_record::Fault;
use trap_cause::TrapCause;
use wost::arch::current::apic;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame, UserEntry, UserRegisters};
use wost::arch::current::paging;
use wost::arch::current::usercopy;

use super::vector;
use crate::thread::{self, ThreadContext};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::{
    LAST_INVALID_USER_RETURN, LAST_USER_FAULT, USER_COPY_RECOVERIES, observe_syscall_entry,
};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    Record, last_invalid_user_return, last_syscall, last_user_fault,
    smap_survived_the_last_syscall, user_copy_recoveries,
};

static HANDLERS: interrupt::Handlers =
    interrupt::Handlers::new(on_trap, on_user_entry, refuse_invalid_user_return);

pub(crate) fn init() {
    interrupt::init(&HANDLERS, Some(vector::SYSCALL));
}

fn on_user_entry(context: &mut ThreadContext, registers: &mut UserRegisters<'_>, entry: UserEntry) {
    match entry {
        UserEntry::Call => {
            #[cfg(feature = "selftest")]
            observe_syscall_entry(registers);
            crate::user::call(context, registers);
        }
        UserEntry::Fault(_, address) => {
            #[cfg(feature = "selftest")]
            LAST_USER_FAULT.record(registers);
            crate::user::record_ending(boot_record::Record::Fault(Fault {
                vector: u64::from(registers.vector()),
                error: registers.error_code(),
                address,
                rip: registers.instruction_pointer(),
            }));
            thread::retire()
        }
        UserEntry::Interrupted => thread::preempt(context),
    }
    if crate::world::current_is_dying() {
        thread::retire()
    }
    thread::refresh_clock();
}

fn refuse_invalid_user_return(
    _: &mut ThreadContext,
    registers: &UserRegisters<'_>,
    _: interrupt::UserReturnError,
) -> ! {
    #[cfg(feature = "selftest")]
    LAST_INVALID_USER_RETURN.record(registers);
    let _ = registers;
    thread::retire()
}

// Nothing here logs: a fault may arrive under a held lock, and the console lock could deadlock.
fn on_trap(frame: &TrapFrame) {
    let cause = TrapCause::decode(frame.vector(), frame.error_code());
    match cause {
        TrapCause::Fault(trap_cause::Fault::Breakpoint) => {}

        TrapCause::Interrupt(raw) if raw == vector::TIMER.get() => {
            // Dismissed before any switch, or the APIC would hold the timer's class until EOI.
            let eoi = claim_eoi(frame, vector::TIMER);
            crate::time::on_expiry();
            dismiss(eoi);
            thread::reschedule(frame);
        }

        // A spurious interrupt sets no in-service bit, so an EOI would retire a real one beneath.
        TrapCause::Interrupt(raw) if raw == vector::SPURIOUS.get() => {}

        TrapCause::Interrupt(raw) if raw == vector::ERROR.get() => panic!(
            "local apic error {:#x} at {:#018x}",
            apic::error_status(),
            frame.instruction_pointer()
        ),

        TrapCause::Fault(trap_cause::Fault::PageFault(_)) if paging::stale_kernel_write(frame) => {}

        TrapCause::Fault(trap_cause::Fault::PageFault(fault))
            if usercopy::recover(frame, fault) =>
        {
            #[cfg(feature = "selftest")]
            USER_COPY_RECOVERIES.fetch_add(1, Ordering::Relaxed);
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
    assert!(
        apic::end_of_interrupt(token).is_ok(),
        "APIC in-service authority changed before dismissal"
    );
}

#[derive(Debug)]
struct Report<'frame> {
    frame: &'frame TrapFrame,
    cause: TrapCause,
}

impl fmt::Display for Report<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let frame = self.frame;
        let name = self.cause.name();
        let vector = self.cause.vector();

        write!(
            formatter,
            "{} (vector {}, error {:#x}) at {:#018x}, cs {:#x}, flags {:#x}, rsp {:#018x}",
            name,
            vector,
            frame.error_code(),
            frame.instruction_pointer(),
            frame.code_selector(),
            frame.flags(),
            frame.stack_pointer(),
        )?;

        if let TrapCause::Fault(trap_cause::Fault::PageFault(fault)) = self.cause {
            write!(
                formatter,
                ", faulting address {:#018x}: {fault:?}",
                cpu::faulting_address()
            )?;
        }

        Ok(())
    }
}
