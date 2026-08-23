//! Kernel threads: who exists, who is ready, who is asleep, and who runs next.
//!
//! The mechanism — stacks, saved state, the switch — is `wost::task`'s. What
//! is here is policy, and policy is safe Rust: a table of names, states and
//! scheduling entities over the same slot indices. The policy itself is
//! `eevdf`, host-tested; this module feeds it the runnable entities and acts
//! on the answer, with a round-robin fallback if the answer is ever missing
//! (D-53). The timer preempts at trap exit (D-54).
//!
//! Nothing runs on a tick. Every path that changes who is runnable ends by
//! arming the timer for the next moment a decision is due, or stopping it
//! when there is none (D-57).
//!
//! Lock order (`docs/009-multitasking.md`): `THREADS` is a leaf — nothing is
//! taken while it is held. Every switch happens after the guard has dropped,
//! with the interrupt flag clear throughout, so no expiry can preempt a
//! half-made decision.

// `yield_now` and `exit` still have no caller but the self-checks. Silenced
// rather than cfg-gated, so the shipped image carries the code the checks
// exercise.
#![allow(dead_code)]

use core::fmt;
use core::sync::atomic::{AtomicUsize, Ordering};

use eevdf::{BASE_WEIGHT, Entity};
use stack_layout::StackLayout;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::paging::StackArena;
use wost::arch::current::task::{self, BOOT, TaskError};
use wost::panic::PanicToken;
use wost::sync::{Once, SpinLock};

use crate::time::{self, NANOSECONDS_PER_MS};

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
pub(crate) const CAPACITY: usize = STACK_SLOTS + 1;

/// Evidence that the caller is in thread context, not inside a trap handler.
///
/// D-62. Every call that can block takes one, so the rule they used to assert
/// at run time is one the compiler keeps. Neither `Send` nor `Sync`, so it
/// cannot be stashed in a `static` and produced later.
#[derive(Debug)]
pub(crate) struct ThreadContext(core::marker::PhantomData<*const ()>);

impl ThreadContext {
    /// Mint one, or `None` inside a trap handler.
    pub(crate) fn claim() -> Option<Self> {
        (!interrupt::in_trap()).then_some(Self(core::marker::PhantomData))
    }

    /// Mint one, panicking inside a trap handler.
    ///
    /// # Panics
    ///
    /// From trap context, where a block would carry the handler's stack frame
    /// away with it (D-55).
    pub(crate) fn require() -> Self {
        match Self::claim() {
            Some(context) => context,
            None => panic!("thread context required, but a handler is on the stack"),
        }
    }
}

/// Real time a thread may run before its deadline moves, in nanoseconds.
///
/// Three milliseconds, in the range mainline uses. The one tunable EEVDF has,
/// and a tenth of what a tick-bounded slice could be (D-57).
const SLICE_NS: u64 = 3 * NANOSECONDS_PER_MS;

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
    /// Waiting for a deadline, and not a candidate until it passes.
    Sleeping,
    /// Waiting on a queue, and not a candidate until something wakes it.
    Blocked,
}

/// What kind of thread a slot holds, which is the whole of this kernel's
/// scheduling classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A thread the policy schedules.
    Normal,
    /// The thread that runs when no normal one can, and never otherwise.
    ///
    /// Outside the policy, not at the bottom of it: a weight small enough to
    /// ignore is still a weight, and eligibility would hand it the CPU now
    /// and then — a wakeup spent on nothing.
    Idle,
}

#[derive(Debug, Clone, Copy)]
struct Thread {
    state: State,
    class: Class,
    name: &'static str,
    entry: Option<fn()>,
    entity: Entity,
    /// Uptime, in nanoseconds, when it last went on the CPU.
    started: u64,
    /// Uptime at which a sleeper becomes runnable again. Meaningless in every
    /// other state.
    wake_at: u64,
    /// The lag it stopped being runnable carrying, which is what it is placed
    /// back on the queue with — clamped to a slice either way by the policy,
    /// so a brief wait sheds no debt and a long one banks no credit.
    parked_with_lag: i64,
}

impl Thread {
    const FREE: Self = Self {
        state: State::Free,
        class: Class::Normal,
        name: "",
        entry: None,
        entity: Entity::new(BASE_WEIGHT),
        started: 0,
        wake_at: 0,
        parked_with_lag: 0,
    };

    const fn is_runnable(&self) -> bool {
        matches!(self.state, State::Ready | State::Running)
    }

    /// Whether the policy considers this thread at all.
    const fn is_scheduled(&self) -> bool {
        self.is_runnable() && matches!(self.class, Class::Normal)
    }
}

#[derive(Debug)]
struct Threads {
    threads: [Thread; CAPACITY],
    /// Slots the arena backs, or zero when threads are unavailable.
    spawnable: usize,
}

impl Threads {
    /// The fair-share point of everything the policy schedules.
    fn average(&self) -> Option<u64> {
        eevdf::average_vruntime(
            self.threads
                .iter()
                .filter(|thread| thread.is_scheduled())
                .map(|thread| &thread.entity),
        )
    }

    /// How many threads the policy could pick right now. The idle thread is
    /// not one of them, which is what makes an otherwise-empty machine
    /// tickless rather than merely quiet.
    fn runnable(&self) -> usize {
        self.threads
            .iter()
            .filter(|thread| thread.is_scheduled())
            .count()
    }

    /// The slot holding the idle thread, if it has been started.
    fn idle(&self) -> Option<usize> {
        self.threads
            .iter()
            .position(|thread| thread.class == Class::Idle && thread.is_runnable())
    }

    /// Charge the running thread for the time since it went on.
    fn charge(&mut self, slot: usize, now: u64) {
        let thread = &mut self.threads[slot];
        thread.entity.charge(now.saturating_sub(thread.started));
        thread.started = now;
        if thread.entity.slice_is_spent() {
            thread.entity.renew_deadline(SLICE_NS);
        }
    }

    /// Make every sleeper whose deadline has passed runnable again.
    ///
    /// Each comes back at the fair-share point offset by the lag it left
    /// with, which is where D-53's clamp acts.
    fn wake_due(&mut self, now: u64) {
        for slot in 0..CAPACITY {
            let thread = &self.threads[slot];
            if thread.state != State::Sleeping || thread.wake_at > now {
                continue;
            }
            self.make_ready(slot);
        }
    }

    /// Put a parked thread back on the queue, at the fair-share point offset
    /// by the lag it left with.
    fn make_ready(&mut self, slot: usize) {
        let average = self
            .average()
            .unwrap_or(self.threads[slot].entity.vruntime());
        let lag = self.threads[slot].parked_with_lag;
        let thread = &mut self.threads[slot];
        thread.entity.place(average, lag, SLICE_NS);
        thread.state = State::Ready;
        WAKES.fetch_add(1, Ordering::Relaxed);
    }

    /// Record what a thread about to stop being runnable is owed, so that
    /// [`make_ready`](Self::make_ready) can give it back.
    fn park(&mut self, slot: usize) {
        let average = self.average().unwrap_or(0);
        let thread = &mut self.threads[slot];
        thread.parked_with_lag = thread.entity.lag(average);
    }

    /// Which runnable slot should run, considered in order starting just
    /// after `current` so that equals take turns rather than the lowest index
    /// winning every tie. `current` itself is a candidate only if
    /// `include_current`.
    ///
    /// A thread *leaving* the CPU records that before asking, so it is not a
    /// candidate and the idle thread becomes reachable. A thread merely
    /// offering the CPU is told to keep it when nobody else is owed one.
    fn choose(&self, current: usize, include_current: bool) -> Option<usize> {
        let candidates = || {
            (1..=CAPACITY)
                .map(|offset| (current + offset) % CAPACITY)
                .filter(|slot| *slot != current || include_current)
                .filter(|slot| self.threads[*slot].is_scheduled())
        };
        // The idle thread is the answer only when there is no other, and it
        // is never the answer to "should the running thread be replaced" —
        // a normal thread with the CPU keeps it.
        let Some(average) = self.average() else {
            return self.idle().filter(|slot| *slot != current);
        };
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

    /// When the kernel next has a reason to be interrupted, given that
    /// `running` is on the CPU — or `None` if it has none at all.
    ///
    /// Two reasons only. A slice ending matters just when somebody else could
    /// take the CPU; a sleeper's deadline always does, because nothing else
    /// will notice it.
    fn next_event(&self, running: usize, now: u64) -> Option<u64> {
        let slice_end = (self.runnable() > 1)
            .then(|| now.saturating_add(self.threads[running].entity.remaining_slice()));
        let earliest_sleeper = self
            .threads
            .iter()
            .filter(|thread| thread.state == State::Sleeping)
            .map(|thread| thread.wake_at)
            .min();

        match (slice_end, earliest_sleeper) {
            (Some(slice), Some(sleeper)) => Some(slice.min(sleeper)),
            (only, None) | (None, only) => only,
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

/// Parked threads put back on the queue.
static WAKES: AtomicUsize = AtomicUsize::new(0);

/// Threads that have stopped being runnable to wait on a queue.
static BLOCKS: AtomicUsize = AtomicUsize::new(0);

/// Times the idle thread has reached its halt: times the machine had nothing
/// to do.
static IDLE_HALTS: AtomicUsize = AtomicUsize::new(0);

/// Started once, by whoever gets there first.
static IDLE_STARTED: Once = Once::new();

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
    entity.place(0, 0, SLICE_NS);
    threads.threads[BOOT] = Thread {
        state: State::Running,
        name: "boot",
        entity,
        started: time::now_ns(),
        ..Thread::FREE
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

/// How many parked threads have been put back on the queue.
pub(crate) fn wakes() -> usize {
    WAKES.load(Ordering::Relaxed)
}

/// How many threads have blocked on a wait queue.
pub(crate) fn blocks() -> usize {
    BLOCKS.load(Ordering::Relaxed)
}

/// How many times the idle thread has had the CPU with nothing to do on it.
pub(crate) fn idle_halts() -> usize {
    IDLE_HALTS.load(Ordering::Relaxed)
}

/// Whether the idle thread is running.
pub(crate) fn idle_is_started() -> bool {
    IDLE_STARTED.is_claimed() && THREADS.lock().idle().is_some()
}

/// Start the thread that runs when nothing else can.
///
/// Outside the policy entirely (see [`Class::Idle`]), so a machine with
/// nothing to do halts on this thread and still takes no interrupts — the
/// difference being that everything else may now assume something is
/// runnable. Does nothing after the first call, or with no threads.
pub(crate) fn start_idle() {
    if !is_available() || !IDLE_STARTED.claim() {
        return;
    }
    let Ok(slot) = spawn("idle", idle_loop) else {
        return;
    };
    THREADS.lock().threads[slot].class = Class::Idle;
}

/// What the idle thread does: nothing, as cheaply as the hardware allows.
fn idle_loop() {
    loop {
        IDLE_HALTS.fetch_add(1, Ordering::Relaxed);
        // Nothing is armed for this thread; the halt ends on whatever the
        // rest of the machine was already waiting for.
        cpu::halt();
    }
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
            ..Thread::FREE
        };
        slot
    };

    // Outside the lock: `fabricate` takes `wost`'s own, and this one is a
    // leaf. The slot is reserved, so no other spawn can choose it and no
    // expiry can pick it.
    if let Err(error) = task::fabricate(slot, start, slot) {
        THREADS.lock().threads[slot] = Thread::FREE;
        return Err(SpawnError::Task(error));
    }

    // The new thread is the second runnable one, which is the moment the
    // running thread's slice starts mattering — so this is an arming point
    // even though nothing switches here.
    let saved = interrupt::disable();
    let now = time::now_ns();
    let event = {
        let mut threads = THREADS.lock();
        let average = threads.average().unwrap_or(0);
        threads.threads[slot].entity.place(average, 0, SLICE_NS);
        threads.threads[slot].state = State::Ready;
        threads.next_event(task::current(), now)
    };
    reschedule_timer(event);
    saved.restore();
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
pub(crate) fn yield_now(_context: &ThreadContext) {
    // Flag clear from decision to switch, so an expiry cannot preempt between
    // them and act on a table that says `next` is running when it is not.
    let saved = interrupt::disable();
    let now = time::now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock();
        let current = task::current();
        threads.charge(current, now);
        threads.wake_due(now);
        let next = threads.choose(current, false);
        if let Some(next) = next {
            threads.hand_over(current, next, State::Ready, now);
        }
        (next, threads.next_event(next.unwrap_or(current), now))
    };

    reschedule_timer(event);
    if let Some(next) = next
        && let Err(error) = task::switch_to(next)
    {
        panic!("yield to slot {next} refused: {error}");
    }
    saved.restore();
}

/// Give the CPU up until `milliseconds` have passed.
///
/// Another runnable thread takes the CPU meanwhile; with none, the CPU halts
/// here until the timer this armed goes off. Nothing is interrupted before
/// the deadline unless somebody else asked for it.
///
/// Returns no earlier than the deadline and possibly later — the thread
/// becomes runnable then, and runs when the policy picks it — or at once on a
/// machine whose clock never started, where a wait would be a hang.
///
/// # Panics
///
/// Panics from trap context, where a switch would carry the handler's stack
/// frame away with it (D-55), or if `wost` refuses a switch.
pub(crate) fn sleep_ms(context: &ThreadContext, milliseconds: u64) {
    sleep_ns(context, milliseconds.saturating_mul(NANOSECONDS_PER_MS));
}

/// Give the CPU up until `nanoseconds` have passed. See [`sleep_ms`].
///
/// # Panics
///
/// As [`sleep_ms`].
pub(crate) fn sleep_ns(context: &ThreadContext, nanoseconds: u64) {
    if !time::is_calibrated() {
        return;
    }

    let saved = interrupt::disable();
    let wake_at = time::now_ns().saturating_add(nanoseconds);
    // A loop rather than one switch: a thread can be woken by something other
    // than its own deadline — the halt below returns on any interrupt — and
    // the deadline, not the wakeup, is what this promised.
    while time::now_ns() < wake_at {
        THREADS.lock().threads[task::current()].wake_at = wake_at;
        park(context, State::Sleeping, Some(wake_at));
    }
    saved.restore();
}

/// Stop being runnable until something makes this thread ready again.
///
/// Mark the running thread `leaving`, record what it is owed, and hand the
/// CPU on. With nobody to hand it to it keeps the CPU and halts, and the
/// caller re-checks — a halt returns on any interrupt.
///
/// `until` bounds the arming, so a sleeper halting alone is woken by its own
/// timer. Called with the flag already clear, and it stays clear until after
/// the switch.
fn park(_context: &ThreadContext, leaving: State, until: Option<u64>) {
    let now = time::now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock();
        let current = task::current();
        threads.charge(current, now);
        threads.wake_due(now);
        threads.park(current);
        threads.threads[current].state = leaving;

        match threads.choose(current, false) {
            Some(next) => {
                threads.threads[next].state = State::Running;
                threads.threads[next].started = now;
                (Some(next), threads.next_event(next, now))
            }
            // Nobody to hand the CPU to, so this thread keeps it and waits on
            // it — still running, and still the one the table charges, for
            // time spent halted, which is the honest accounting: it had the
            // CPU. The idle thread of `start_idle` is what removes this case
            // once it exists.
            None => {
                threads.threads[current].state = State::Running;
                let event = threads.next_event(current, now);
                (None, earliest(event, until))
            }
        }
    };

    reschedule_timer(event);
    match next {
        Some(next) => {
            if let Err(error) = task::switch_to(next) {
                panic!("park of slot {} refused: {error}", task::current());
            }
        }
        None => {
            // Enabling and halting has to be one step, or the interrupt that
            // was going to end the wait can land in between and leave the CPU
            // halted with nothing left to wake it.
            interrupt::enable_and_halt();
            interrupt::disable();
        }
    }
}

/// The sooner of two deadlines, either of which may be absent.
fn earliest(one: Option<u64>, other: Option<u64>) -> Option<u64> {
    match (one, other) {
        (Some(one), Some(other)) => Some(one.min(other)),
        (only, None) | (None, only) => only,
    }
}

/// Stop being runnable until [`unblock`] names this thread.
///
/// The wait-queue half of [`park`]: no deadline, so nothing is armed and a
/// machine with nothing else to do takes no interrupts until the event.
pub(crate) fn block(context: &ThreadContext) {
    BLOCKS.fetch_add(1, Ordering::Relaxed);
    park(context, State::Blocked, None);
}

/// Make every blocked thread in `slots` runnable, and report how many that
/// was.
///
/// A bitmask because the caller's caller is an interrupt handler, where D-55
/// forbids the heap. A set-but-not-blocked slot is skipped: a thread that
/// registered and then found nothing to wait for is still registered.
pub(crate) fn unblock(slots: u32) -> usize {
    let mut threads = THREADS.lock();
    let mut woken = 0;
    for slot in 0..CAPACITY {
        if slots & (1 << slot) == 0 || threads.threads[slot].state != State::Blocked {
            continue;
        }
        threads.make_ready(slot);
        woken += 1;
    }
    woken
}

/// The slot this CPU is running on.
pub(crate) fn current_slot() -> usize {
    task::current()
}

/// Trap exit's turn: charge the running thread, wake whatever is due, switch
/// if something else is owed the CPU, and arm for whatever comes next.
///
/// Called from a handler after its end-of-interrupt: by the timer always, and
/// by the keyboard when a keystroke made a reader runnable. Trap exit is where
/// a switch is legal at all (D-54).
///
/// Only the switch is skipped when the interrupted code had interrupts off
/// (D-54). Waking and re-arming happen either way: a sleeper whose deadline
/// has passed has no other appointment. Nothing here logs (D-31).
pub(crate) fn reschedule(frame: &TrapFrame) {
    let now = time::now_ns();
    let switchable = frame.flags & INTERRUPT_FLAG != 0;
    let (next, event) = {
        let mut threads = THREADS.lock();
        if threads.spawnable == 0 {
            return;
        }
        let current = task::current();
        threads.charge(current, now);
        threads.wake_due(now);
        let next = match switchable.then(|| threads.choose(current, true)).flatten() {
            Some(next) if next != current => {
                threads.hand_over(current, next, State::Ready, now);
                Some(next)
            }
            _ => None,
        };
        (next, threads.next_event(next.unwrap_or(current), now))
    };

    reschedule_timer(event);
    if let Some(next) = next
        && let Err(error) = task::switch_from_trap(next, frame)
    {
        panic!("preemption to slot {next} refused: {error}");
    }
}

/// Arm the timer for the next thing owed a decision, or stop it if nothing
/// is.
///
/// Every path that can change the answer ends here. An idle machine leaves
/// with the timer stopped.
fn reschedule_timer(event: Option<u64>) {
    match event {
        Some(deadline) => time::arm_at(deadline),
        None => time::disarm(),
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
    // Never restored: this context is ending, and an expiry between the
    // decision and the switch would find a thread that is neither running
    // nor free.
    let _off = interrupt::disable();
    let now = time::now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock();
        let current = task::current();
        assert!(current != BOOT, "the boot thread cannot exit");
        threads.threads[current] = Thread::FREE;
        threads.wake_due(now);
        let Some(next) = threads.choose(current, false) else {
            panic!("nothing is runnable after slot {current} exited");
        };
        threads.hand_over(current, next, State::Free, now);
        (next, threads.next_event(next, now))
    };

    reschedule_timer(event);
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
