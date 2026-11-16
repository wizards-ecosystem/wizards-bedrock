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

// Far above any call the kernel serves, so adding a call cannot make this one real.
const UNSERVED: Call = Call::Unknown(0x0BAD_0BAD);

const TOKEN: u32 = 0x1234;

const ECHOED: u64 = 0x00C0_FFEE;

const AGREED: u64 = 0;

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
    WideHandle = 10,
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
    let handle = u32::try_from(bootstrap).map_err(|_| Failure::WideHandle)?;
    let rights = u64::from(Rights::INSPECT.bits());
    let completion = exchange(Submission::about(Opcode::Duplicate, TOKEN, handle, rights))?;
    if completion.value == 0 || completion.value == u64::from(handle) {
        return Err(Failure::WrongValue);
    }
    Ok(())
}

fn exchange(submission: Submission) -> Result<Completion, Failure> {
    let mut ring = Ring::take().ok_or(Failure::RingTaken)?;
    if ring.version() != native_abi::ring::VERSION {
        return Err(Failure::RingVersion);
    }
    ring.submit(0, submission);
    ring.publish_submission_tail(1);
    if witness_rt::call(Call::RingEnter, 0) != 1 {
        return Err(Failure::NotServed);
    }
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
