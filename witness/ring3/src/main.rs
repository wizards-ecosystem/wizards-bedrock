#![no_std]
#![no_main]
// Denied, since `forbid` would refuse the `#[allow(unsafe_code)]` glue `witness_rt::entry!` expands.
#![deny(unsafe_code)]

use native_abi::ring::{Completion, Opcode, Submission, status};
use native_abi::{Call, Rights, error};
use witness_rt::{Entry, Ring, fault_with};

witness_rt::entry!(behave);

// The arguments the kernel's self-tests pass to choose a behaviour.
const ECHO: u64 = 0;
const FAULT: u64 = 1;
const PROBE: u64 = 2;
const DUPLICATE: u64 = 3;
const INVALID_RETURN_STACK: u64 = 6;
const INVALID_RETURN_FLAGS: u64 = 7;
const READ_COUNTER: u64 = 8;
const READ_KERNEL: u64 = 9;
const RING_WAIT: u64 = 10;
const THREAD_BASE: u64 = 11;
const SYSCALL: u64 = 12;

// Far above any call the kernel serves, so adding a call cannot make this one real.
const UNSERVED: Call = Call::Unknown(0x0BAD_0BAD);

const TOKEN: u32 = 0x1234;

const ECHOED: u64 = 0x00C0_FFEE;

const AGREED: u64 = 0;

const TIMER_NS: u64 = 1_000_000;

const KERNEL_HALF: u64 = 0xffff_8000_0000_0000;

const NONCANONICAL: u64 = 0x0000_8000_0000_0000;

// Read back through `fs:0` once the thread base names it.
static THREAD_MARK: u64 = 0x5EED_BA5E_0000_0011;

// The exit status names the first answer that was wrong; 0 is agreement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Failure {
    EntryVersion = 1,
    UnknownBehaviour = 2,
    RingTaken = 3,
    RingVersion = 4,
    NotServed = 5,
    NoCompletion = 6,
    Refused = 7,
    WrongValue = 8,
    Unanswered = 9,
    IdleWait = 11,
    TimerServedEarly = 12,
    NotWoken = 13,
    BaseRefused = 14,
    BaseUnread = 15,
    BadBaseAccepted = 16,
    BaseMoved = 17,
}

fn behave(entry: &Entry) -> u64 {
    match run(entry) {
        Ok(()) => AGREED,
        Err(failure) => failure as u64,
    }
}

fn run(entry: &Entry) -> Result<(), Failure> {
    if entry.version != native_abi::entry::VERSION {
        return Err(Failure::EntryVersion);
    }
    // A negative argument is a kernel address to jump to from ring 3.
    if entry.argument.cast_signed() < 0 {
        witness_rt::jump_to_kernel(entry.argument);
    }
    match entry.argument {
        ECHO => echo(),
        FAULT => fault_with(FAULT),
        PROBE => probe(),
        DUPLICATE => duplicate(entry.bootstrap_handle),
        INVALID_RETURN_STACK => witness_rt::enter_with_noncanonical_stack(UNSERVED),
        INVALID_RETURN_FLAGS => witness_rt::enter_with_nested_task(UNSERVED),
        READ_COUNTER => witness_rt::read_counter_with(READ_COUNTER),
        READ_KERNEL => witness_rt::read_kernel(entry.operand),
        RING_WAIT => ring_wait(),
        THREAD_BASE => thread_base(),
        SYSCALL => witness_rt::execute_syscall(SYSCALL),
        _ => Err(Failure::UnknownBehaviour),
    }
}

// An unserved call is answered, not dropped, and the program keeps running.
fn probe() -> Result<(), Failure> {
    if witness_rt::call(UNSERVED, 0) != error::ENOSYS {
        return Err(Failure::Unanswered);
    }
    Ok(())
}

fn echo() -> Result<(), Failure> {
    let completion = exchange(Submission::simple(Opcode::Echo, TOKEN, ECHOED))?;
    if completion.value != ECHOED {
        return Err(Failure::WrongValue);
    }
    Ok(())
}

// The entry block, not the image, supplies the authority the duplicate names.
fn duplicate(bootstrap: u64) -> Result<(), Failure> {
    let rights = u64::from(Rights::INSPECT.bits());
    let completion = exchange(Submission::about(
        Opcode::Duplicate,
        TOKEN,
        bootstrap,
        rights,
    ))?;
    if completion.value == 0 || completion.value == bootstrap {
        return Err(Failure::WrongValue);
    }
    Ok(())
}

// Idle, the wait returns at once; with a timer in flight it sleeps until that timer completes.
fn ring_wait() -> Result<(), Failure> {
    if witness_rt::call(Call::RingWait, 0) != 0 {
        return Err(Failure::IdleWait);
    }
    let mut ring = open_ring()?;
    ring.submit(0, Submission::simple(Opcode::Timer, TOKEN, TIMER_NS));
    ring.publish_submission_tail(1);
    if witness_rt::call(Call::RingEnter, 0) != 0 {
        return Err(Failure::TimerServedEarly);
    }
    if witness_rt::call(Call::RingWait, 0) != 1 {
        return Err(Failure::NotWoken);
    }
    let completion = completed(&mut ring)?;
    if completion.value == 0 {
        return Err(Failure::WrongValue);
    }
    Ok(())
}

// A user base is taken; a kernel-half or noncanonical one is refused and leaves the base alone.
fn thread_base() -> Result<(), Failure> {
    let base = core::ptr::from_ref(&THREAD_MARK).addr() as u64;
    if witness_rt::call(Call::ThreadBase, base) != 0 {
        return Err(Failure::BaseRefused);
    }
    if witness_rt::thread_word() != THREAD_MARK {
        return Err(Failure::BaseUnread);
    }
    for refused in [KERNEL_HALF, NONCANONICAL] {
        if witness_rt::call(Call::ThreadBase, refused) != error::EINVAL {
            return Err(Failure::BadBaseAccepted);
        }
    }
    if witness_rt::thread_word() != THREAD_MARK {
        return Err(Failure::BaseMoved);
    }
    Ok(())
}

fn open_ring() -> Result<Ring, Failure> {
    let ring = Ring::take().ok_or(Failure::RingTaken)?;
    if ring.version() != native_abi::ring::VERSION {
        return Err(Failure::RingVersion);
    }
    Ok(ring)
}

fn completed(ring: &mut Ring) -> Result<Completion, Failure> {
    if ring.completion_tail() != 1 {
        return Err(Failure::NoCompletion);
    }
    let completion = ring.completion(0);
    ring.release_completion_head(1);
    if completion.status != status::OK {
        return Err(Failure::Refused);
    }
    Ok(completion)
}

fn exchange(submission: Submission) -> Result<Completion, Failure> {
    let mut ring = open_ring()?;
    ring.submit(0, submission);
    ring.publish_submission_tail(1);
    if witness_rt::call(Call::RingEnter, 0) != 1 {
        return Err(Failure::NotServed);
    }
    completed(&mut ring)
}
