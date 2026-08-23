//! Kernel threads: who exists, who is ready, and who runs next.
//!
//! The mechanism — stacks, saved state, the switch — is `wost::task`'s. What
//! is here is policy, and policy is safe Rust: a table of names, states and
//! scheduling entities over the same slot indices. The policy itself is
//! `eevdf`, host-tested; this module feeds it the runnable entities and acts
//! on the answer, with a round-robin fallback if the answer is ever missing
//! (D-53). The timer preempts at trap exit (D-54).
//!
//! Lock order (`docs/009-multitasking.md`): `THREADS` is a leaf. Nothing is
//! taken while it is held — not the heap, not a console, not even `wost`'s
//! own task table. Every switch happens after the guard has dropped, with
//! the interrupt flag clear from before the lock was taken until after the
//! switch, so no tick can preempt a half-made decision.

// Until slice 4 gives the kernel its idle thread, the only callers are the
// self-checks. Silenced here rather than cfg-gated, so the shipped image
// carries the same code the checks exercise. Remove with the first caller.
#![allow(dead_code)]

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

use eevdf::{BASE_WEIGHT, Entity};
use stack_layout::StackLayout;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::paging::StackArena;
use wost::arch::current::task::{self, BOOT, TaskError};
use wost::panic::PanicToken;
use wost::sync::SpinLock;

use crate::time;

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

/// Real time a thread may run before its deadline moves, in milliseconds.
///
/// Three ticks of the 100 Hz clock. The one tunable EEVDF has, and the
/// thing slice 3's finer clock will let shrink.
const SLICE_MS: u64 = 30;

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
    /// Taken by a spawn that has not finished; not yet runnable.
    Reserved,
    /// Runnable and waiting to be picked.
    Ready,
    /// On the CPU.
    Running,
}

#[derive(Debug, Clone, Copy)]
struct Thread {
    state: State,
    name: &'static str,
    entry: Option<fn()>,
    entity: Entity,
    /// Uptime, in milliseconds, when it last went on the CPU.
    started: u64,
}

impl Thread {
    const FREE: Self = Self {
        state: State::Free,
        name: "",
        entry: None,
        entity: Entity::new(BASE_WEIGHT),
        started: 0,
    };

    const fn is_runnable(&self) -> bool {
        matches!(self.state, State::Ready | State::Running)
    }
}

#[derive(Debug)]
struct Threads {
    threads: [Thread; CAPACITY],
    /// Slots the arena backs, or zero when threads are unavailable.
    spawnable: usize,
}

impl Threads {
    /// The fair-share point of everything runnable.
    fn average(&self) -> Option<u64> {
        eevdf::average_vruntime(
            self.threads
                .iter()
                .filter(|thread| thread.is_runnable())
                .map(|thread| &thread.entity),
        )
    }

    /// Charge the running thread for the time since it went on.
    fn charge(&mut self, slot: usize, now: u64) {
        let thread = &mut self.threads[slot];
        thread.entity.charge(now.saturating_sub(thread.started));
        thread.started = now;
        if thread.entity.slice_is_spent() {
            thread.entity.renew_deadline(SLICE_MS);
        }
    }

    /// Which runnable slot should run, considered in order starting just
    /// after `current` so that equals take turns rather than the lowest index
    /// winning every tie. `current` itself is a candidate only if
    /// `include_current`.
    fn choose(&self, current: usize, include_current: bool) -> Option<usize> {
        let candidates = || {
            (1..=CAPACITY)
                .map(|offset| (current + offset) % CAPACITY)
                .filter(|slot| *slot != current || include_current)
                .filter(|slot| self.threads[*slot].is_runnable())
        };
        let average = self.average()?;
        let picked = eevdf::pick(
            candidates().map(|slot| (slot, &self.threads[slot].entity)),
            average,
        );
        match picked {
            Some(slot) => Some(slot),
            // With the running thread excluded, "nobody else is eligible" is
            // an honest answer: it is the one behind the average, and it
            // keeps the CPU.
            None if !include_current => None,
            // With everyone considered, the policy says nobody is eligible —
            // which `eevdf`'s tests say cannot happen. Counted so the
            // self-checks can see it, and answered with round-robin so the
            // machine keeps running (D-53).
            None => {
                let fallback = candidates().next();
                if fallback.is_some() {
                    POLICY_FALLBACKS.fetch_add(1, Ordering::Relaxed);
                }
                fallback
            }
        }
    }

    /// Move the CPU's bookkeeping from `current` to `next`.
    fn hand_over(&mut self, current: usize, next: usize, leaving: State, now: u64) {
        self.threads[current].state = leaving;
        self.threads[next].state = State::Running;
        self.threads[next].started = now;
    }
}

static THREADS: SpinLock<Threads> = SpinLock::new(Threads {
    threads: [Thread::FREE; CAPACITY],
    spawnable: 0,
});

/// Times the policy had no answer and round-robin stood in for it.
///
/// Should stay at zero; a count that climbs is the policy disagreeing with
/// its own host tests, which is worth knowing about immediately.
static POLICY_FALLBACKS: AtomicUsize = AtomicUsize::new(0);

/// Take the mapped arena into use, making the caller the boot thread.
///
/// # Errors
///
/// Whatever `wost` found wrong with the arena.
pub(crate) fn init(arena: StackArena) -> Result<usize, TaskError> {
    let spawnable = task::init(arena)?;
    let mut threads = THREADS.lock();
    threads.spawnable = spawnable.min(STACK_SLOTS);
    let mut entity = Entity::new(BASE_WEIGHT);
    entity.place(0, 0, SLICE_MS);
    threads.threads[BOOT] = Thread {
        state: State::Running,
        name: "boot",
        entry: None,
        entity,
        started: time::uptime_ms(),
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

/// How often the policy has been stood in for. See [`POLICY_FALLBACKS`].
pub(crate) fn policy_fallbacks() -> usize {
    POLICY_FALLBACKS.load(Ordering::Relaxed)
}

/// Create a thread of the base weight that will run `entry`.
///
/// See [`spawn_weighted`].
///
/// # Errors
///
/// As [`spawn_weighted`].
pub(crate) fn spawn(name: &'static str, entry: fn()) -> Result<usize, SpawnError> {
    spawn_weighted(name, entry, BASE_WEIGHT)
}

/// Create a thread of `weight` that will run `entry`, and make it runnable.
///
/// Returns its slot. It is placed at the queue's fair-share point with a
/// fresh slice, so it neither jumps the queue nor waits behind everyone. It
/// ends when `entry` returns.
///
/// # Errors
///
/// [`SpawnError::Unavailable`] if the arena was never mapped,
/// [`SpawnError::Full`] if no slot is free, or [`SpawnError::Task`] if
/// `wost` refused the slot — which would mean the two tables disagree.
pub(crate) fn spawn_weighted(
    name: &'static str,
    entry: fn(),
    weight: u32,
) -> Result<usize, SpawnError> {
    let slot = {
        let mut threads = THREADS.lock();
        if threads.spawnable == 0 {
            return Err(SpawnError::Unavailable);
        }
        let slot = (1..=threads.spawnable)
            .find(|slot| threads.threads[*slot].state == State::Free)
            .ok_or(SpawnError::Full)?;
        threads.threads[slot] = Thread {
            state: State::Reserved,
            name,
            entry: Some(entry),
            entity: Entity::new(weight),
            started: 0,
        };
        slot
    };

    // Outside the lock: `fabricate` takes `wost`'s own, and this one is a
    // leaf. The slot is reserved, so no other spawn can choose it and no
    // tick can pick it.
    if let Err(error) = task::fabricate(slot, start, slot) {
        THREADS.lock().threads[slot] = Thread::FREE;
        return Err(SpawnError::Task(error));
    }

    let mut threads = THREADS.lock();
    let average = threads.average().unwrap_or(0);
    threads.threads[slot].entity.place(average, 0, SLICE_MS);
    threads.threads[slot].state = State::Ready;
    Ok(slot)
}

/// Let another runnable thread have the CPU, if any is owed it.
///
/// Returns when this thread is picked again — at once, if nothing else is
/// eligible.
///
/// # Panics
///
/// Panics from trap context, where a switch would carry the handler's stack
/// frame away with it (D-55), or if `wost` refuses the switch: the two
/// tables disagree, which is a bug and not a condition.
pub(crate) fn yield_now() {
    assert!(!interrupt::in_trap(), "yield from trap context");

    // Flag clear from decision to switch, so a tick cannot preempt between
    // them and act on a table that says `next` is running when it is not.
    let saved = interrupt::disable();
    let now = time::uptime_ms();
    let next = {
        let mut threads = THREADS.lock();
        let current = task::current();
        threads.charge(current, now);
        let next = threads.choose(current, false);
        if let Some(next) = next {
            threads.hand_over(current, next, State::Ready, now);
        }
        next
    };

    if let Some(next) = next
        && let Err(error) = task::switch_to(next)
    {
        panic!("yield to slot {next} refused: {error}");
    }
    saved.restore();
}

/// The timer's turn: charge the running thread, and switch if something
/// else is owed the CPU.
///
/// Called from the tick handler, after the end-of-interrupt. Does nothing
/// if the interrupted code had interrupts off — it holds a lock, or is
/// between a decision and a switch — so a critical section is never
/// preempted; D-34 makes that test free. Nothing here logs (D-31).
pub(crate) fn preempt(frame: &TrapFrame) {
    if frame.flags & INTERRUPT_FLAG == 0 {
        return;
    }

    let now = time::uptime_ms();
    let next = {
        let mut threads = THREADS.lock();
        if threads.spawnable == 0 {
            return;
        }
        let current = task::current();
        threads.charge(current, now);
        match threads.choose(current, true) {
            Some(next) if next != current => {
                threads.hand_over(current, next, State::Ready, now);
                next
            }
            _ => return,
        }
    };

    if let Err(error) = task::switch_from_trap(next, frame) {
        panic!("preemption to slot {next} refused: {error}");
    }
}

/// `RFLAGS` bit 9, restated here rather than imported: the test is against
/// what the CPU pushed, not what a helper believes.
const INTERRUPT_FLAG: u64 = 1 << 9;

/// End the running thread and let the next runnable one run.
///
/// # Panics
///
/// Panics from the boot thread, which owns the stack everything else was
/// started from, or if nothing is runnable — neither can happen while the
/// boot thread is ready, which it is whenever it is not running.
pub(crate) fn exit() -> ! {
    // Never restored: this context is ending, and a tick between the
    // decision and the switch would find a thread that is neither running
    // nor free.
    let _off = interrupt::disable();
    let now = time::uptime_ms();
    let next = {
        let mut threads = THREADS.lock();
        let current = task::current();
        assert!(current != BOOT, "the boot thread cannot exit");
        threads.threads[current] = Thread::FREE;
        let Some(next) = threads.choose(current, false) else {
            panic!("nothing is runnable after slot {current} exited");
        };
        threads.hand_over(current, next, State::Free, now);
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
