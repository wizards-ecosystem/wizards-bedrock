#![no_std]
#![no_main]
// Denied, since `forbid` would refuse the `#[allow(unsafe_code)]` glue `witness_rt::entry!` expands.
#![deny(unsafe_code)]

use native_abi::ring::{Completion, Opcode, Submission, flags, status};
use native_abi::witness::behaviour::{
    BACKDOOR, CHANNELS, DUPLICATE, ECHO, FAULT, INVALID_RETURN_FLAGS, INVALID_RETURN_STACK, MEMORY,
    PROBE, READ_COUNTER, READ_KERNEL, RING_WAIT, SYSCALL, THREAD_BASE,
};
use native_abi::witness::{self, AGREED};
use native_abi::{Call, Rights, error, limits, message};
use witness_rt::{Data, Entry, Ring, fault_with};

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
    ChannelAnswer = 19,
    WrittenTooFew = 20,
    RefusedHandleLost = 21,
    MessageOutOfOrder = 22,
    MovedHandleLost = 23,
    ReadNotReady = 24,
    DataTaken = 25,
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
        CHANNELS => channels(entry),
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
    let rights = u64::from(Rights::READ.bits());
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

const WRITE_AT: usize = 0;
const READ_AT: usize = 2400;
const READ_ROOM: u32 = 64;
const MOVED: Rights = Rights::TRANSFER.union(Rights::READ);
const ENDPOINT: Rights = Rights::DUPLICATE
    .union(Rights::TRANSFER)
    .union(Rights::READ)
    .union(Rights::WRITE);

// Writes that each move a handle fill the inbox; the refused one keeps its handle, and the reads
// return every committed message once, in order, each with the handle it moved.
fn channels(entry: &Entry) -> Result<(), Failure> {
    let mut ring = Session::open(entry)?;
    let mut data = Data::take(entry).ok_or(Failure::DataTaken)?;
    let created = ring.expect(
        Submission::simple(Opcode::ChannelCreate, TOKEN, 0),
        status::OK,
    )?;
    let (writer, reader) = (created.value, created.extra);
    let mut written = 0;
    let refused = loop {
        let moved = ring
            .expect(duplicate_as(entry.bootstrap_handle(), ENDPOINT), status::OK)?
            .value;
        compose(&mut data, written, moved);
        let completion = ring.submit(channel(Opcode::ChannelWrite, writer, WRITE_AT))?;
        match completion.status {
            status::OK => written += 1,
            status::FULL => break moved,
            _ => return Err(Failure::ChannelAnswer),
        }
    };
    if written != 16 {
        return Err(Failure::WrittenTooFew);
    }
    let again = ring.submit(duplicate_as(refused, ENDPOINT))?;
    if again.status != status::OK {
        return Err(Failure::RefusedHandleLost);
    }
    ring.expect(close(again.value), status::OK)?;
    ask_room(&mut data);
    for sequence in 0..written {
        let completion = ring.expect(channel(Opcode::ChannelRead, reader, READ_AT), status::OK)?;
        if completion.value != 8 || completion.extra != 1 || payload(&data) != sequence {
            return Err(Failure::MessageOutOfOrder);
        }
        let (handle, rights) = received(&data);
        if rights != MOVED.bits() {
            return Err(Failure::MovedHandleLost);
        }
        ring.expect(close(handle), status::OK)?;
    }
    // An empty inbox arms the read; the next write readies it and the ring wait serves it.
    ring.publish(channel(Opcode::ChannelRead, reader, READ_AT));
    if witness_rt::call(Call::RingEnter, 0) != 0 {
        return Err(Failure::ReadNotReady);
    }
    compose(&mut data, written, refused);
    let committed = ring.submit(channel(Opcode::ChannelWrite, writer, WRITE_AT))?;
    if committed.status != status::OK {
        return Err(Failure::ChannelAnswer);
    }
    if witness_rt::call(Call::RingWait, 0) != 1 {
        return Err(Failure::ReadNotReady);
    }
    let late = ring.collect()?;
    if late.status != status::OK || payload(&data) != written {
        return Err(Failure::MessageOutOfOrder);
    }
    ring.expect(close(received(&data).0), status::OK)?;
    ring.expect(close(writer), status::OK)?;
    ring.expect(
        channel(Opcode::ChannelRead, reader, READ_AT),
        status::PEER_CLOSED,
    )?;
    Ok(())
}

fn compose(data: &mut Data, sequence: u64, moved: u64) {
    let mut header = [0u8; message::ENTRIES_AT];
    header[message::VERSION_AT..][..4].copy_from_slice(&message::VERSION.to_le_bytes());
    header[message::BYTES..][..4].copy_from_slice(&8u32.to_le_bytes());
    header[message::HANDLES..][..4].copy_from_slice(&1u32.to_le_bytes());
    data.write(WRITE_AT, &header);
    let mut entry = [0u8; message::ENTRY_BYTES];
    entry[..8].copy_from_slice(&moved.to_le_bytes());
    entry[8..12].copy_from_slice(&MOVED.bits().to_le_bytes());
    data.write(WRITE_AT + message::ENTRIES_AT, &entry);
    data.write(WRITE_AT + message::BYTES_AT, &sequence.to_le_bytes());
}

fn ask_room(data: &mut Data) {
    let mut header = [0u8; message::ENTRIES_AT];
    header[message::VERSION_AT..][..4].copy_from_slice(&message::VERSION.to_le_bytes());
    header[message::BYTE_CAPACITY..][..4].copy_from_slice(&READ_ROOM.to_le_bytes());
    header[message::HANDLE_CAPACITY..][..4].copy_from_slice(&limits::MESSAGE_HANDLES.to_le_bytes());
    data.write(READ_AT, &header);
}

fn payload(data: &Data) -> u64 {
    let mut bytes = [0u8; 8];
    data.read(READ_AT + message::BYTES_AT, &mut bytes);
    u64::from_le_bytes(bytes)
}

fn received(data: &Data) -> (u64, u32) {
    let mut entry = [0u8; message::ENTRY_BYTES];
    data.read(READ_AT + message::ENTRIES_AT, &mut entry);
    let mut handle = [0u8; 8];
    handle.copy_from_slice(&entry[..8]);
    let rights = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
    (u64::from_le_bytes(handle), rights)
}

const fn channel(opcode: Opcode, endpoint: u64, at: usize) -> Submission {
    Submission {
        at: at as u64,
        ..Submission::about(opcode, TOKEN, endpoint, 0)
    }
}

const fn close(handle: u64) -> Submission {
    Submission::about(Opcode::Close, TOKEN, handle, 0)
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
    head: u32,
}

impl Session {
    fn open(entry: &Entry) -> Result<Self, Failure> {
        Ok(Self {
            ring: open_ring(entry)?,
            next: 0,
            head: 0,
        })
    }

    fn expect(&mut self, submission: Submission, wanted: u32) -> Result<Completion, Failure> {
        let completion = self.submit(submission)?;
        if completion.status != wanted {
            return Err(Failure::MemoryAnswer);
        }
        Ok(completion)
    }

    fn submit(&mut self, submission: Submission) -> Result<Completion, Failure> {
        self.publish(submission);
        if witness_rt::call(Call::RingEnter, 0) != 1 {
            return Err(Failure::NotServed);
        }
        self.collect()
    }

    fn publish(&mut self, submission: Submission) {
        self.ring.submit(self.next, submission);
        self.next = self.next.wrapping_add(1);
        self.ring.publish_submission_tail(self.next);
    }

    fn collect(&mut self) -> Result<Completion, Failure> {
        if self.ring.completion_tail() == self.head {
            return Err(Failure::NoCompletion);
        }
        let completion = self.ring.completion(self.head);
        self.head = self.head.wrapping_add(1);
        self.ring.release_completion_head(self.head);
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
