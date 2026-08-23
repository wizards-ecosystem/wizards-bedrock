//! Kernel threads: who exists, who is ready, and who runs next.
//!
//! The mechanism — stacks, saved state, the switch — is `wost::task`'s. What
//! is here is policy, and policy is safe Rust: a table of names and states
//! over the same slot indices, and a ready list. This slice's list is a
//! round-robin ring, deliberately policy-free; the scheduler proper arrives
//! behind the seam D-53 describes.
//!
//! Lock order (`docs/009-multitasking.md`): `THREADS` is a leaf. Nothing is
//! taken while it is held — not the heap, not a console, not even `wost`'s
//! own task table. Every switch happens after the guard has dropped.

// Until slice 4 gives the kernel its idle thread, the only callers are the
// self-checks. Silenced here rather than cfg-gated, so the shipped image
// carries the same code the checks exercise. Remove with the first caller.
#![allow(dead_code)]

use core::fmt;

use stack_layout::StackLayout;
use wost::arch::current::paging::StackArena;
use wost::arch::current::task::{self, BOOT, TaskError};
use wost::panic::PanicToken;
use wost::sync::SpinLock;

/// Threads that can exist at once, besides the boot thread.
///
/// A real cap: stacks are committed at table-build time because mapping is
/// one-way (D-45), so this is how many the arena holds. Revisable, and
/// Phase 5's address-space growth is where it stops being a constant.
pub(crate) const STACK_SLOTS: usize = 16;

/// Bytes of stack per thread.
const STACK_BYTES: u64 = 16 * 1024;

/// Bytes of unmapped gap below each stack.
const GUARD_BYTES: u64 = 4096;

/// Where the arena sits: below the device window, far above the direct map,
/// so a stack address is never mistaken for either (D-52).
const WINDOW: u64 = 0xffff_fe00_0000_0000;

/// Slots in the table, the boot thread's among them.
const CAPACITY: usize = STACK_SLOTS + 1;

/// The arena the kernel asks `wost` to map.
pub(crate) fn layout() -> Option<StackLayout> {
    StackLayout::new(WINDOW, STACK_BYTES, GUARD_BYTES, STACK_SLOTS)
}

/// Why a thread could not be spawned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum SpawnError {
    /// The arena was never mapped, so there is nothing to run a thread on.
    Unavailable,
    /// Every slot holds a live thread.
    Full,
    /// `wost` refused the slot.
    Task(TaskError),
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unavailable => f.write_str("threads are unavailable"),
            Self::Full => write!(f, "all {STACK_SLOTS} thread slots are live"),
            Self::Task(error) => write!(f, "{error}"),
        }
    }
}

impl core::error::Error for SpawnError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing here, or a thread that has exited.
    Free,
    /// Waiting its turn on the ready list.
    Ready,
    /// On the CPU.
    Running,
}

#[derive(Debug, Clone, Copy)]
struct Thread {
    state: State,
    name: &'static str,
    entry: Option<fn()>,
}

impl Thread {
    const FREE: Self = Self {
        state: State::Free,
        name: "",
        entry: None,
    };
}

/// A fixed ring of slot indices: who runs next, oldest first.
///
/// Each slot is on it at most once, so it can never fill, and it allocates
/// nothing — which is what lets the lock around it stay a leaf.
#[derive(Debug)]
struct Ring {
    slots: [usize; CAPACITY],
    head: usize,
    len: usize,
}

impl Ring {
    const EMPTY: Self = Self {
        slots: [0; CAPACITY],
        head: 0,
        len: 0,
    };

    fn push(&mut self, slot: usize) {
        if self.len < CAPACITY {
            self.slots[(self.head + self.len) % CAPACITY] = slot;
            self.len += 1;
        }
    }

    fn pop(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        let slot = self.slots[self.head];
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        Some(slot)
    }
}

#[derive(Debug)]
struct Threads {
    threads: [Thread; CAPACITY],
    ready: Ring,
    /// Slots the arena backs, or zero when threads are unavailable.
    spawnable: usize,
}

static THREADS: SpinLock<Threads> = SpinLock::new(Threads {
    threads: [Thread::FREE; CAPACITY],
    ready: Ring::EMPTY,
    spawnable: 0,
});

/// Take the mapped arena into use, making the caller the boot thread.
///
/// # Errors
///
/// Whatever `wost` found wrong with the arena.
pub(crate) fn init(arena: StackArena) -> Result<usize, TaskError> {
    let spawnable = task::init(arena)?;
    let mut threads = THREADS.lock();
    threads.spawnable = spawnable.min(STACK_SLOTS);
    threads.threads[BOOT] = Thread {
        state: State::Running,
        name: "boot",
        entry: None,
    };
    Ok(threads.spawnable)
}

/// Whether threads can be spawned at all.
pub(crate) fn is_available() -> bool {
    THREADS.lock().spawnable > 0
}

/// Slots with no live thread in them.
pub(crate) fn free_slots() -> usize {
    let threads = THREADS.lock();
    threads.threads[1..=threads.spawnable]
        .iter()
        .filter(|thread| thread.state == State::Free)
        .count()
}

/// Create a thread that will run `entry` and put it on the ready list.
///
/// Returns its slot. The thread runs when something yields to it; it ends
/// when `entry` returns.
///
/// # Errors
///
/// [`SpawnError::Unavailable`] if the arena was never mapped,
/// [`SpawnError::Full`] if no slot is free, or [`SpawnError::Task`] if
/// `wost` refused the slot — which would mean the two tables disagree.
pub(crate) fn spawn(name: &'static str, entry: fn()) -> Result<usize, SpawnError> {
    let slot = {
        let mut threads = THREADS.lock();
        if threads.spawnable == 0 {
            return Err(SpawnError::Unavailable);
        }
        let slot = (1..=threads.spawnable)
            .find(|slot| threads.threads[*slot].state == State::Free)
            .ok_or(SpawnError::Full)?;
        threads.threads[slot] = Thread {
            state: State::Ready,
            name,
            entry: Some(entry),
        };
        slot
    };

    // Outside the lock: `fabricate` takes `wost`'s own, and this one is a
    // leaf. The slot is marked taken, so no other spawn can choose it.
    if let Err(error) = task::fabricate(slot, start, slot) {
        THREADS.lock().threads[slot] = Thread::FREE;
        return Err(SpawnError::Task(error));
    }

    THREADS.lock().ready.push(slot);
    Ok(slot)
}

/// Let the thread at the head of the ready list run, and rejoin the tail.
///
/// Returns when this thread's turn comes round again — at once, if nothing
/// else is ready.
///
/// # Panics
///
/// Panics if `wost` refuses the switch: the two tables disagree, which is a
/// bug and not a condition.
pub(crate) fn yield_now() {
    let next = {
        let mut threads = THREADS.lock();
        let Some(next) = threads.ready.pop() else {
            return;
        };
        let current = task::current();
        threads.threads[current].state = State::Ready;
        threads.ready.push(current);
        threads.threads[next].state = State::Running;
        next
    };

    if let Err(error) = task::switch_to(next) {
        panic!("yield to slot {next} refused: {error}");
    }
}

/// End the running thread and let the next ready one run.
///
/// # Panics
///
/// Panics from the boot thread, which owns the stack everything else was
/// started from, or if nothing is ready — neither can happen while the boot
/// thread is on the ready list, which it is whenever it is not running.
pub(crate) fn exit() -> ! {
    let next = {
        let mut threads = THREADS.lock();
        let current = task::current();
        assert!(current != BOOT, "the boot thread cannot exit");
        threads.threads[current] = Thread::FREE;
        let Some(next) = threads.ready.pop() else {
            panic!("nothing is ready to run after slot {current} exited");
        };
        threads.threads[next].state = State::Running;
        next
    };

    let error = task::exit_to(next);
    panic!("exit to slot {next} refused: {error}");
}

/// Where every spawned thread begins: run its entry, then leave.
extern "C" fn start(slot: usize) -> ! {
    let entry = THREADS.lock().threads[slot].entry;
    match entry {
        Some(entry) => entry(),
        None => panic!("slot {slot} started with no entry"),
    }
    exit()
}

/// Reclaim the thread table's lock on the terminal panic path.
pub(crate) fn force_unlock(token: &PanicToken) {
    THREADS.force_unlock(token);
}
