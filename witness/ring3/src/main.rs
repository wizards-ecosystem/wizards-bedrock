#![no_std]
#![no_main]
// Denied, since `forbid` would refuse the `#[allow(unsafe_code)]` glue `witness_rt::entry!` expands.
#![deny(unsafe_code)]

use native_abi::ring::{Completion, Opcode, Submission, flags, status};
use native_abi::witness::behaviour::{
    BACKDOOR, DUPLICATE, ECHO, FAULT, INVALID_RETURN_FLAGS, INVALID_RETURN_STACK, MEMORY, PROBE,
    READ_COUNTER, READ_KERNEL, RING_WAIT, SYSCALL, THREAD_BASE,
};
use native_abi::witness::{self, AGREED};
use native_abi::{Call, Rights, error, limits};
use witness_rt::{Entry, Ring, fault_with};

witness_rt::entry!(behave);

const UNSERVED: Call = Call::Unknown(witness::UNSERVED_CALL);

const TOKEN: u32 = 0x1234;

const ECHOED: u64 = 0x00C0_FFEE;

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
    MemoryAnswer = 18,
}

fn behave(entry: &Entry) -> u64 {
    match run(entry) {
        Ok(()) => AGREED,
        Err(failure) => failure as u64,
    }
}

fn run(entry: &Entry) -> Result<(), Failure> {
    if entry.version() != native_abi::entry::VERSION {
        return Err(Failure::EntryVersion);
    }
    // A negative argument is a kernel address to jump to from ring 3.
    if entry.argument().cast_signed() < 0 {
        witness_rt::jump_to_kernel(entry.argument());
    }
    match entry.argument() {
        ECHO => echo(entry),
        FAULT => fault_with(FAULT),
        PROBE => probe(),
        DUPLICATE => duplicate(entry),
        INVALID_RETURN_STACK => witness_rt::enter_with_noncanonical_stack(UNSERVED),
        INVALID_RETURN_FLAGS => witness_rt::enter_with_nested_task(UNSERVED),
        READ_COUNTER => witness_rt::read_counter_with(READ_COUNTER),
        READ_KERNEL => witness_rt::read_kernel(entry.operand()),
        RING_WAIT => ring_wait(entry),
        THREAD_BASE => thread_base(),
        SYSCALL => witness_rt::execute_syscall(SYSCALL),
        MEMORY => memory(entry),
        BACKDOOR => witness_rt::probe_backdoor(),
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

fn echo(entry: &Entry) -> Result<(), Failure> {
    let completion = exchange(entry, Submission::simple(Opcode::Echo, TOKEN, ECHOED))?;
    if completion.value != ECHOED {
        return Err(Failure::WrongValue);
    }
    Ok(())
}

// The entry block, not the image, supplies the authority the duplicate names.
fn duplicate(entry: &Entry) -> Result<(), Failure> {
    let bootstrap = entry.bootstrap_handle();
    let rights = u64::from(Rights::INSPECT.bits());
    let completion = exchange(
        entry,
        Submission::about(Opcode::Duplicate, TOKEN, bootstrap, rights),
    )?;
    if completion.value == 0 || completion.value == bootstrap {
        return Err(Failure::WrongValue);
    }
    Ok(())
}

// Idle, the wait returns at once; with a timer in flight it sleeps until that timer completes.
fn ring_wait(entry: &Entry) -> Result<(), Failure> {
    if witness_rt::call(Call::RingWait, 0) != 0 {
        return Err(Failure::IdleWait);
    }
    let mut ring = open_ring(entry)?;
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
    if witness_rt::set_thread_base(&THREAD_MARK) != 0 {
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

// Each refusal names its rule, and only a seal turns the object's one handle into another.
fn memory(entry: &Entry) -> Result<(), Failure> {
    let mut ring = Session::open(entry)?;
    let sealable = ring.expect(create(0), status::OK)?.value;
    let shareable = ring.expect(create(flags::SHAREABLE), status::OK)?.value;
    let copy = Submission {
        second: shareable,
        argument: 8,
        length: 16,
        ..Submission::about(Opcode::MemoryCopy, TOKEN, sealable, 0)
    };
    ring.expect(copy, status::OK)?;
    ring.expect(seal(shareable), status::DENIED)?;
    let sealed = ring.expect(seal(sealable), status::OK)?.value;
    if sealed == sealable {
        return Err(Failure::MemoryAnswer);
    }
    ring.expect(seal(sealed), status::DENIED)?;
    ring.expect(
        Submission {
            handle: sealed,
            ..copy
        },
        status::DENIED,
    )?;
    let executable = Rights::READ.union(Rights::EXECUTE);
    ring.expect(duplicate_as(sealed, executable), status::DENIED)?;
    ring.expect(duplicate_as(sealed, Rights::READ), status::OK)?;
    ring.expect(
        Submission::about(Opcode::Close, TOKEN, sealable, 0),
        status::BAD_HANDLE,
    )?;
    let mapped = ring.expect(create(0), status::OK)?.value;
    ring.expect(map(mapped, flags::MAP_WRITE), status::OK)?;
    ring.expect(seal(mapped), status::BUSY)?;
    ring.expect(map(sealed, flags::MAP_EXECUTE), status::DENIED)?;
    let unmap = Submission {
        length: 1,
        ..Submission::about(Opcode::Unmap, TOKEN, 0, MAPPED_AT)
    };
    ring.expect(unmap, status::OK)?;
    // Each fresh address needs its own tables, which only an unmap that returns them keeps bounded.
    for step in 1..=witness::FRESH_MAPS {
        let at = MAPPED_AT + step * FRESH_STRIDE;
        ring.expect(
            Submission {
                argument: at,
                ..map(mapped, 0)
            },
            status::OK,
        )?;
        ring.expect(
            Submission {
                argument: at,
                ..unmap
            },
            status::OK,
        )?;
    }
    ring.expect(seal(mapped), status::OK)?;
    Ok(())
}

// Below the kernel's placements and far above the image.
const MAPPED_AT: u64 = limits::PLACEMENT_START / 4;

// Odd in table spans and below the placements, so the pairs cross directory boundaries too.
const FRESH_STRIDE: u64 = 23 * (2 << 20);
const _: () = assert!(
    MAPPED_AT + (witness::FRESH_MAPS + 1) * FRESH_STRIDE <= limits::PLACEMENT_START,
    "the fresh maps stay below the kernel's placements"
);

const fn map(object: u64, flags: u64) -> Submission {
    Submission {
        second: object,
        length: 1,
        flags,
        ..Submission::about(Opcode::Map, TOKEN, 0, MAPPED_AT)
    }
}

const fn create(flags: u64) -> Submission {
    Submission {
        flags,
        ..Submission::simple(Opcode::MemoryCreate, TOKEN, 1)
    }
}

const fn seal(handle: u64) -> Submission {
    Submission {
        flags: flags::SEAL_READ,
        ..Submission::about(Opcode::MemorySeal, TOKEN, handle, 0)
    }
}

fn duplicate_as(handle: u64, rights: Rights) -> Submission {
    Submission::about(Opcode::Duplicate, TOKEN, handle, u64::from(rights.bits()))
}

// One submission at a time, each answered before the next is published.
struct Session {
    ring: Ring,
    next: u32,
}

impl Session {
    fn open(entry: &Entry) -> Result<Self, Failure> {
        Ok(Self {
            ring: open_ring(entry)?,
            next: 0,
        })
    }

    fn expect(&mut self, submission: Submission, wanted: u32) -> Result<Completion, Failure> {
        let index = self.next;
        self.next = index.wrapping_add(1);
        self.ring.submit(index, submission);
        self.ring.publish_submission_tail(self.next);
        if witness_rt::call(Call::RingEnter, 0) != 1 {
            return Err(Failure::NotServed);
        }
        if self.ring.completion_tail() != self.next {
            return Err(Failure::NoCompletion);
        }
        let completion = self.ring.completion(index);
        self.ring.release_completion_head(self.next);
        if completion.status != wanted {
            return Err(Failure::MemoryAnswer);
        }
        Ok(completion)
    }
}

fn open_ring(entry: &Entry) -> Result<Ring, Failure> {
    let ring = Ring::take(entry).ok_or(Failure::RingTaken)?;
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

fn exchange(entry: &Entry, submission: Submission) -> Result<Completion, Failure> {
    let mut ring = open_ring(entry)?;
    ring.submit(0, submission);
    ring.publish_submission_tail(1);
    if witness_rt::call(Call::RingEnter, 0) != 1 {
        return Err(Failure::NotServed);
    }
    completed(&mut ring)
}
