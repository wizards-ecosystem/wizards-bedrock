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

// Every wrong answer faults, so a clean exit is the witness's agreement.
fn behave(entry: &Entry) {
    if entry.version != native_abi::entry::VERSION {
        fault_with(entry.version);
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
        other => fault_with(other),
    }
}

// An unserved call is answered, not dropped, and the program keeps running.
fn probe() {
    let answer = witness_rt::call(UNSERVED, 0);
    if answer != error::ENOSYS {
        fault_with(answer);
    }
}

fn echo() {
    let completion = exchange(Submission::simple(Opcode::Echo, TOKEN, ECHOED));
    if completion.value != ECHOED {
        fault_with(completion.value);
    }
}

// The entry block, not the image, supplies the authority the duplicate names.
fn duplicate(bootstrap: u64) {
    let Ok(handle) = u32::try_from(bootstrap) else {
        fault_with(bootstrap);
    };
    let rights = u64::from(Rights::INSPECT.bits());
    let completion = exchange(Submission::about(Opcode::Duplicate, TOKEN, handle, rights));
    if completion.value == 0 || completion.value == u64::from(handle) {
        fault_with(completion.value);
    }
}

fn exchange(submission: Submission) -> Completion {
    let Some(mut ring) = Ring::take() else {
        fault_with(0);
    };
    let version = ring.version();
    if version != native_abi::ring::VERSION {
        fault_with(u64::from(version));
    }
    ring.submit(0, submission);
    ring.publish_submission_tail(1);
    let served = witness_rt::call(Call::RingEnter, 0);
    if served != 1 {
        fault_with(served);
    }
    let tail = ring.completion_tail();
    if tail != 1 {
        fault_with(u64::from(tail));
    }
    let completion = ring.completion(0);
    if completion.status != status::OK {
        fault_with(u64::from(completion.status));
    }
    ring.release_completion_head(1);
    completion
}
