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

use core::fmt;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use eevdf::{BASE_WEIGHT, Entity};
use stack_layout::StackLayout;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::paging::{Binding, StackArena};
use wost::arch::current::task::{self, BOOT, TaskError};
use wost::sync::SpinLock;
pub(crate) use wost::sync::ThreadContext;

use crate::time::{self, NANOSECONDS_PER_MS};

/// Threads that can exist at once, besides the boot thread.
///
/// How many the arena holds. It was a real cap while mapping was one-way
/// (D-45); since D-86 and D-88 a stack could be mapped at spawn and unmapped
/// at exit, so this is now a default rather than a permanent ceiling. It stays
/// fixed until a measured caller needs more; D-180's first userland gate needs
/// four simultaneous processes, not more than sixteen.
pub(crate) const STACK_SLOTS: usize = 16;

/// Bytes of stack per thread.
///
/// Thirty-two kilobytes since D-113. Sixteen was enough until `spawn` grew:
/// building a process moves an `AddressSpace` — eight hundred bytes of device
/// table — through several frames, and a debug build elides none of the
/// copies. The overflow showed up as a double fault on a *guard page*, in the
/// debug boot D-65 added for exactly this class, which is the whole
/// arrangement working: an unguarded stack would have written into its
/// neighbour and been found much later, by something else (D-52, D-89).
const STACK_BYTES: u64 = 32 * 1024;

/// Bytes of unmapped gap below each stack.
const GUARD_BYTES: u64 = 4096;

/// Where the arena sits: below the device window, far above the direct map,
/// so a stack address is never mistaken for either (D-52).
const WINDOW: u64 = 0xffff_fe00_0000_0000;

/// Slots in the table, the boot thread's among them.
pub(crate) const CAPACITY: usize = STACK_SLOTS + 1;

/// Real time a thread may run before its deadline moves, in nanoseconds.
///
/// Three milliseconds, in the range mainline uses. The one tunable EEVDF has,
/// and a tenth of what a tick-bounded slice could be (D-57).
const SLICE_NS: u64 = 3 * NANOSECONDS_PER_MS;

/// The arena the kernel asks `wost` to map.
pub(crate) fn layout() -> Option<StackLayout> {
    StackLayout::new(WINDOW, STACK_BYTES, GUARD_BYTES, STACK_SLOTS)
}

/// Where the double-fault handler's guarded stack goes.
///
/// Immediately above the thread arena and derived from it, so this adds no new
/// hardcoded window and no entry to D-46's KASLR ledger. The geometry is the
/// arena's own — one slot, one guard gap below it — which is arithmetic
/// `libs/stack-layout` already has tests for.
pub(crate) fn fault_stack_layout() -> Option<StackLayout> {
    StackLayout::new(layout()?.end(), STACK_BYTES, GUARD_BYTES, 1)
}

/// Where the boot thread's own guarded stack goes.
///
/// Above the fault stack and derived from it, for the same reason: no new
/// hardcoded window, and no entry on D-46's ledger. The boot thread starts on
/// the bootloader's stack, which has nothing below it and lives inside the
/// unsplittable direct map, so the only way to guard it is to leave it (D-92).
pub(crate) fn boot_stack_layout() -> Option<StackLayout> {
    StackLayout::new(fault_stack_layout()?.end(), STACK_BYTES, GUARD_BYTES, 1)
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
    /// The caller could not publish the state the new thread needs.
    Arrangement,
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unavailable => f.write_str("threads are unavailable"),
            Self::Full => write!(f, "all {STACK_SLOTS} thread slots are live"),
            Self::Task(error) => write!(f, "{error}"),
            Self::Arrangement => f.write_str("the thread's launch state could not be arranged"),
        }
    }
}

impl core::error::Error for SpawnError {}

/// A completed thread-construction step at which a check may force refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpawnPoint {
    /// The scheduler slot is reserved, but no task exists.
    Reserved,
    /// The task exists, but has no user address space.
    Fabricated,
    /// The task names the user address space, but is not runnable.
    Bound,
}

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

/// Who a thread is.
///
/// Changed by `spawn` and by `exit`, and read by nothing that schedules: the
/// half of a thread the policy has no business seeing (D-109).
#[derive(Debug, Clone, Copy)]
struct Identity {
    name: &'static str,
    entry: Option<fn()>,
}

impl Identity {
    const FREE: Self = Self {
        name: "",
        entry: None,
    };
}

/// Everything the policy needs about one slot, and nothing else.
///
/// Deliberately without a name or an entry point. Swapping `libs/eevdf` for
/// another policy is a change to [`RunQueue`] and to this, and to nothing that
/// knows what a thread *is* (D-109).
#[derive(Debug, Clone, Copy)]
struct Scheduled {
    state: State,
    class: Class,
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

impl Scheduled {
    const FREE: Self = Self {
        state: State::Free,
        class: Class::Normal,
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

/// Who runs next.
///
/// The scheduler's whole world: one [`Scheduled`] per slot and the policy over
/// them. It cannot name a thread, which is the point of the boundary.
#[derive(Debug)]
struct RunQueue {
    slots: [Scheduled; CAPACITY],
}

/// Who exists, and who runs next.
///
/// One lock over both halves, because every decision that changes runnability
/// happens with the other half settled. The split is a *structural* boundary
/// and not a locking one: the queue's methods take `&RunQueue`, so a change to
/// the policy cannot reach a name (D-109).
#[derive(Debug)]
struct Threads {
    identities: [Identity; CAPACITY],
    queue: RunQueue,
    /// Slots the arena backs, or zero when threads are unavailable.
    spawnable: usize,
}

impl Threads {
    /// Give a slot back: nobody is there, and nothing schedules it.
    ///
    /// Both halves, in one call, because a slot free in one and not the other
    /// is a thread with no name or a name with no thread (D-109).
    fn release(&mut self, slot: usize) {
        self.identities[slot] = Identity::FREE;
        self.queue.slots[slot] = Scheduled::FREE;
    }
}

impl RunQueue {
    /// The fair-share point of everything the policy schedules.
    fn average(&self) -> Option<u64> {
        eevdf::average_vruntime(
            self.slots
                .iter()
                .filter(|thread| thread.is_scheduled())
                .map(|thread| &thread.entity),
        )
    }

    /// How many threads the policy could pick right now. The idle thread is
    /// not one of them, which is what makes an otherwise-empty machine
    /// tickless rather than merely quiet.
    fn runnable(&self) -> usize {
        self.slots
            .iter()
            .filter(|thread| thread.is_scheduled())
            .count()
    }

    /// The slot holding the idle thread, if it has been started.
    fn idle(&self) -> Option<usize> {
        self.slots
            .iter()
            .position(|thread| thread.class == Class::Idle && thread.is_runnable())
    }

    /// Charge the running thread for the time since it went on.
    fn charge(&mut self, slot: usize, now: u64) {
        let thread = &mut self.slots[slot];
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
            let thread = &self.slots[slot];
            if thread.state != State::Sleeping || thread.wake_at > now {
                continue;
            }
            self.make_ready(slot);
        }
    }

    /// Put a parked thread back on the queue, at the fair-share point offset
    /// by the lag it left with.
    fn make_ready(&mut self, slot: usize) {
        let average = self.average().unwrap_or(self.slots[slot].entity.vruntime());
        let lag = self.slots[slot].parked_with_lag;
        let thread = &mut self.slots[slot];
        thread.entity.place(average, lag, SLICE_NS);
        thread.state = State::Ready;
        WAKES.fetch_add(1, Ordering::Relaxed);
    }

    /// Record what a thread about to stop being runnable is owed, so that
    /// [`make_ready`](Self::make_ready) can give it back.
    fn park(&mut self, slot: usize) {
        let average = self.average().unwrap_or(0);
        let thread = &mut self.slots[slot];
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
                .filter(|slot| self.slots[*slot].is_scheduled())
        };
        // The idle thread is the answer only when there is no other, and it
        // is never the answer to "should the running thread be replaced" —
        // a normal thread with the CPU keeps it.
        let Some(average) = self.average() else {
            return self.idle().filter(|slot| *slot != current);
        };
        let picked = eevdf::pick(
            candidates().map(|slot| (slot, &self.slots[slot].entity)),
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
            .then(|| now.saturating_add(self.slots[running].entity.remaining_slice()));
        let earliest_sleeper = self
            .slots
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
        self.slots[current].state = leaving;
        self.slots[next].state = State::Running;
        self.slots[next].started = now;
    }
}

static THREADS: SpinLock<Threads> = SpinLock::new(Threads {
    identities: [Identity::FREE; CAPACITY],
    queue: RunQueue {
        slots: [Scheduled::FREE; CAPACITY],
    },
    spawnable: 0,
});

/// Times the policy had no answer and round-robin stood in for it.
///
/// Should stay at zero; a count that climbs is the policy disagreeing with
/// its own host tests, which is worth knowing about immediately.
static POLICY_FALLBACKS: AtomicUsize = AtomicUsize::new(0);

/// Parked threads put back on the queue.
static WAKES: AtomicUsize = AtomicUsize::new(0);

/// Threads that have actually left the run queue to wait on a queue.
static BLOCKS: AtomicUsize = AtomicUsize::new(0);

/// Trap-frame waits which actually handed the CPU to another thread.
static TRAP_WAITS: AtomicUsize = AtomicUsize::new(0);

/// Times the idle thread has reached its halt: times the machine had nothing
/// to do.
static IDLE_HALTS: AtomicUsize = AtomicUsize::new(0);

const IDLE_UNSTARTED: u8 = 0;
const IDLE_STARTING: u8 = 1;
const IDLE_STARTED: u8 = 2;

/// Reserved while an idle spawn is in flight and committed only on success.
static IDLE_STATE: AtomicU8 = AtomicU8::new(IDLE_UNSTARTED);

/// Take the mapped arena into use, making the caller the boot thread.
///
/// # Errors
///
/// Whatever `wost` found wrong with the arena.
pub(crate) fn init(arena: StackArena) -> Result<usize, TaskError> {
    let spawnable = task::init(arena)?;
    let mut threads = THREADS.lock_leaf();
    threads.spawnable = spawnable.min(STACK_SLOTS);
    let mut entity = Entity::new(BASE_WEIGHT);
    entity.place(0, 0, SLICE_NS);
    threads.identities[BOOT] = Identity {
        name: "boot",
        entry: None,
    };
    threads.queue.slots[BOOT] = Scheduled {
        state: State::Running,
        entity,
        started: time::now_ns(),
        ..Scheduled::FREE
    };
    Ok(threads.spawnable)
}

/// Whether threads can be spawned at all.
pub(crate) fn is_available() -> bool {
    THREADS.lock_leaf().spawnable > 0
}

/// Slots with no live thread in them.
#[cfg(feature = "selftest")]
pub(crate) fn free_slots() -> usize {
    let threads = THREADS.lock_leaf();
    threads.queue.slots[1..=threads.spawnable]
        .iter()
        .filter(|slot| slot.state == State::Free)
        .count()
}

/// How often the policy has been stood in for. See [`POLICY_FALLBACKS`].
#[cfg(feature = "selftest")]
pub(crate) fn policy_fallbacks() -> usize {
    POLICY_FALLBACKS.load(Ordering::Relaxed)
}

/// How many parked threads have been put back on the queue.
#[cfg(feature = "selftest")]
pub(crate) fn wakes() -> usize {
    WAKES.load(Ordering::Relaxed)
}

/// How many threads have blocked on a wait queue.
#[cfg(feature = "selftest")]
pub(crate) fn blocks() -> usize {
    BLOCKS.load(Ordering::Relaxed)
}

/// Ring-3 waits which switched through their saved trap frame.
#[cfg(feature = "selftest")]
pub(crate) fn trap_waits() -> usize {
    TRAP_WAITS.load(Ordering::Relaxed)
}

/// How many times the idle thread has had the CPU with nothing to do on it.
#[cfg(feature = "selftest")]
pub(crate) fn idle_halts() -> usize {
    IDLE_HALTS.load(Ordering::Relaxed)
}

/// Whether the idle thread is running.
#[cfg(feature = "selftest")]
pub(crate) fn idle_is_started() -> bool {
    IDLE_STATE.load(Ordering::Acquire) == IDLE_STARTED && THREADS.lock_leaf().queue.idle().is_some()
}

/// How many live slots are classified as idle.
#[cfg(feature = "selftest")]
pub(crate) fn idle_threads() -> usize {
    THREADS
        .lock_leaf()
        .queue
        .slots
        .iter()
        .filter(|slot| slot.class == Class::Idle && slot.state != State::Free)
        .count()
}

/// Release a deliberately burned in-progress reservation after its check.
#[cfg(feature = "selftest")]
pub(crate) fn restore_idle_retry_for_test() {
    let _ = IDLE_STATE.compare_exchange(
        IDLE_STARTING,
        IDLE_UNSTARTED,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

/// Start the thread that runs when nothing else can.
///
/// Outside the policy entirely (see [`Class::Idle`]), so a machine with
/// nothing to do halts on this thread and still takes no interrupts — the
/// difference being that everything else may now assume something is
/// runnable. Does nothing once started, while another attempt is active, or
/// with no threads. A refused spawn leaves a later call free to retry.
pub(crate) fn start_idle() {
    if !is_available()
        || IDLE_STATE
            .compare_exchange(
                IDLE_UNSTARTED,
                IDLE_STARTING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
    {
        return;
    }

    let spawned = spawn_prepared(
        "idle",
        idle_loop,
        BASE_WEIGHT,
        None,
        |slot| {
            THREADS.lock_leaf().queue.slots[slot].class = Class::Idle;
            true
        },
        |_| false,
    );
    if spawned.is_err() {
        IDLE_STATE.store(IDLE_UNSTARTED, Ordering::Release);
        return;
    }
    IDLE_STATE.store(IDLE_STARTED, Ordering::Release);
}

/// What the idle thread does: nothing, as cheaply as the hardware allows.
fn idle_loop() {
    loop {
        // Where a dead program's memory goes back. The idle thread *is* "the
        // machine has nothing better to do", which is exactly when freeing
        // memory should happen — and it is the one context that is neither the
        // dying thread nor holding anything (D-117).
        //
        // One per pass rather than all of them, so a halt is never far away.
        if crate::user::collect().is_some() {
            continue;
        }
        // And where what the syscall handler could not say gets said. A
        // handler cannot take the console's lock — that assert is what a
        // program used to halt the machine with (D-122) — so it leaves a
        // count, and this is a context where reporting one is legal.
        if crate::user::report_refusals() {
            continue;
        }
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
/// Weights other than the base have no shipped caller yet; see [`yield_now`].
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
    spawn_prepared(name, entry, weight, None, |_| true, |_| false)
}

/// Create a thread that runs on `binding`'s tables rather than the kernel's.
///
/// `user::spawn`'s, and so the shipped kernel's since D-116 ran a program in
/// one. It said the opposite until D-130 — "no shipped caller: the shipped
/// kernel reads the keyboard and runs no user program" — under an
/// `#[allow(dead_code)]` that had gone quietly untrue with it.
///
/// `arrange` is called with the new slot while the thread is still `Reserved`
/// — after `wost` has a slot to bind to, and before anything can schedule it.
/// That window is the only place a caller can name a thread that does not yet
/// run, and both the address space and whatever the entry point needs to find
/// have to be in place before its first instruction (D-106).
///
/// # Errors
///
/// As [`spawn_weighted`], plus [`SpawnError::Task`] if `wost` refuses the
/// binding or [`SpawnError::Arrangement`] if `arrange` returns false.
pub(crate) fn spawn_in_space(
    name: &'static str,
    entry: fn(),
    binding: Binding,
    arrange: impl FnOnce(usize) -> bool,
) -> Result<usize, SpawnError> {
    spawn_prepared(name, entry, BASE_WEIGHT, Some(binding), arrange, |_| false)
}

/// Create a user thread but refuse it after one exact construction step.
#[cfg(feature = "selftest")]
pub(crate) fn spawn_in_space_refusing(
    name: &'static str,
    entry: fn(),
    binding: Binding,
    point: SpawnPoint,
    arrange: impl FnOnce(usize) -> bool,
) -> Result<usize, SpawnError> {
    spawn_prepared(
        name,
        entry,
        BASE_WEIGHT,
        Some(binding),
        arrange,
        move |seen| seen == point,
    )
}

fn spawn_prepared(
    name: &'static str,
    entry: fn(),
    weight: u32,
    binding: Option<Binding>,
    arrange: impl FnOnce(usize) -> bool,
    mut refuse: impl FnMut(SpawnPoint) -> bool,
) -> Result<usize, SpawnError> {
    // Before looking for a slot: a machine busy enough never to idle would
    // otherwise never collect, and the thing most likely to need a slot is the
    // thing most likely to free one (D-117).
    while crate::user::has_zombies() && crate::user::collect().is_some() {}

    let slot = {
        let mut threads = THREADS.lock_leaf();
        if threads.spawnable == 0 {
            return Err(SpawnError::Unavailable);
        }
        let slot = (1..=threads.spawnable)
            .find(|slot| threads.queue.slots[*slot].state == State::Free)
            .ok_or(SpawnError::Full)?;
        threads.identities[slot] = Identity {
            name,
            entry: Some(entry),
        };
        threads.queue.slots[slot] = Scheduled {
            state: State::Reserved,
            entity: Entity::new(weight),
            ..Scheduled::FREE
        };
        slot
    };
    if refuse(SpawnPoint::Reserved) {
        THREADS.lock_leaf().release(slot);
        return Err(SpawnError::Full);
    }

    // Outside the lock: `fabricate` takes `wost`'s own, and this one is a
    // leaf. The slot is reserved, so no other spawn can choose it and no
    // expiry can pick it.
    if let Err(error) = task::fabricate(slot, start, slot) {
        THREADS.lock_leaf().release(slot);
        return Err(SpawnError::Task(error));
    }
    if refuse(SpawnPoint::Fabricated) {
        let _ = task::retire(slot);
        THREADS.lock_leaf().release(slot);
        return Err(SpawnError::Full);
    }

    // Still `Reserved`: nothing can schedule this slot, so the tables and
    // whatever the entry point reads are both in place before its first
    // instruction rather than racing it.
    if let Some(binding) = binding
        && let Err(error) = task::bind_address_space(slot, binding)
    {
        // `wost`'s slot is `Alive` and this one would go `Free`, so a later
        // spawn choosing it would be refused for ever — a permanent slot loss
        // rather than a leak that ends. Retired on `wost`'s side too, which is
        // what `exit` does and what leaves the two tables agreeing (D-115).
        //
        // Unreachable today: after a successful `fabricate` the slot is in
        // range, `Alive` and not the current one, which is every way
        // `bind_address_space` can refuse. Handled because "unreachable" is a
        // property of today's callers and not of this function.
        let _ = task::retire(slot);
        THREADS.lock_leaf().release(slot);
        return Err(SpawnError::Task(error));
    }
    if refuse(SpawnPoint::Bound) {
        let _ = task::retire(slot);
        THREADS.lock_leaf().release(slot);
        return Err(SpawnError::Full);
    }
    if !arrange(slot) {
        let _ = task::retire(slot);
        THREADS.lock_leaf().release(slot);
        return Err(SpawnError::Arrangement);
    }

    // The new thread is the second runnable one, which is the moment the
    // running thread's slice starts mattering — so this is an arming point
    // even though nothing switches here.
    let saved = interrupt::disable();
    let now = time::now_ns();
    let event = {
        let mut threads = THREADS.lock_leaf();
        let average = threads.queue.average().unwrap_or(0);
        threads.queue.slots[slot].entity.place(average, 0, SLICE_NS);
        threads.queue.slots[slot].state = State::Ready;
        threads.queue.next_event(task::current(), now)
    };
    reschedule_timer(event);
    saved.restore();
    Ok(slot)
}

/// Let another runnable thread have the CPU, if any is owed it.
///
/// No shipped caller yet, so the image carries it for the checks alone. The
/// attribute below states that as an expectation rather than a permission: it
/// is the build without the checks that has to find this dead, and the day a
/// shipped caller appears the gate says so (D-130).
///
/// Returns when this thread is picked again — at once, if nothing else is
/// eligible.
///
/// # Panics
///
/// Panics from trap context, where a switch would carry the handler's stack
/// frame away with it (D-55), or if `wost` refuses the switch: the two
/// tables disagree, which is a bug and not a condition.
#[cfg_attr(not(feature = "selftest"), expect(dead_code))]
pub(crate) fn yield_now(context: &mut ThreadContext) {
    // Flag clear from decision to switch, so an expiry cannot preempt between
    // them and act on a table that says `next` is running when it is not.
    let saved = interrupt::disable();
    let now = time::now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock(context);
        let current = task::current();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        let next = threads.queue.choose(current, false);
        if let Some(next) = next {
            threads.queue.hand_over(current, next, State::Ready, now);
        }
        (next, threads.queue.next_event(next.unwrap_or(current), now))
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
pub(crate) fn sleep_ms(context: &mut ThreadContext, milliseconds: u64) {
    sleep_ns(context, milliseconds.saturating_mul(NANOSECONDS_PER_MS));
}

/// Give the CPU up until `nanoseconds` have passed. See [`sleep_ms`].
///
/// # Panics
///
/// As [`sleep_ms`].
pub(crate) fn sleep_ns(context: &mut ThreadContext, nanoseconds: u64) {
    if !time::is_calibrated() {
        return;
    }

    let saved = interrupt::disable();
    let wake_at = time::now_ns().saturating_add(nanoseconds);
    // A loop rather than one switch: a thread can be woken by something other
    // than its own deadline — the halt below returns on any interrupt — and
    // the deadline, not the wakeup, is what this promised.
    while time::now_ns() < wake_at {
        THREADS.lock(context).queue.slots[task::current()].wake_at = wake_at;
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
fn park(context: &mut ThreadContext, leaving: State, until: Option<u64>) {
    let now = time::now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock(context);
        let current = task::current();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        threads.queue.park(current);
        threads.queue.slots[current].state = leaving;

        match threads.queue.choose(current, false) {
            Some(next) => {
                // Counted here, not at the call: a thread with nobody to hand
                // the CPU to never leaves the run queue.
                if leaving == State::Blocked {
                    BLOCKS.fetch_add(1, Ordering::Relaxed);
                }
                threads.queue.slots[next].state = State::Running;
                threads.queue.slots[next].started = now;
                (Some(next), threads.queue.next_event(next, now))
            }
            // Nobody to hand the CPU to, so this thread keeps it and waits on
            // it — still running, and still the one the table charges, for
            // time spent halted, which is the honest accounting: it had the
            // CPU. The idle thread of `start_idle` is what removes this case
            // once it exists.
            None => {
                threads.queue.slots[current].state = State::Running;
                let event = threads.queue.next_event(current, now);
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
            let _disabled = interrupt::disable();
        }
    }
}

/// Take the trapping thread off the run queue for good, from trap context.
///
/// What a thread that has crossed into ring 3 and called back needs: it cannot
/// `park`, because that wants a [`ThreadContext`] and trap context cannot mint
/// one by construction (D-62). So this is `park`'s trap-side twin — mark the
/// caller blocked with nothing that will ever wake it, and switch away at trap
/// exit exactly as preemption does (D-54).
///
/// `false` if there is nobody to switch to, which cannot happen once the idle
/// thread exists, or if `wost` refuses the switch.
pub(crate) fn retire_from_trap(frame: &TrapFrame) -> bool {
    let now = time::now_ns();
    let current = task::current();
    let (next, event) = {
        let mut threads = THREADS.lock_leaf();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        threads.queue.park(current);
        // Free, not blocked. Until D-117 a retired thread's slot stayed taken
        // for the life of the boot, which made the cap of sixteen a cap on
        // programs *ever run* rather than on programs running. The switch
        // below leaves this stack for good, so nothing resumes here.
        threads.identities[current] = Identity::FREE;
        threads.queue.slots[current] = Scheduled::FREE;
        match threads.queue.choose(current, false) {
            Some(next) => {
                threads.queue.slots[next].state = State::Running;
                threads.queue.slots[next].started = now;
                (Some(next), threads.queue.next_event(next, now))
            }
            None => (None, None),
        }
    };

    reschedule_timer(event);
    match next {
        Some(next) => {
            RETIREMENTS.fetch_add(1, Ordering::AcqRel);
            // Its resources become a zombie: this thread is standing on the
            // stack and running on the tables it would have to free, so it
            // cannot be the one to free them (D-117). Moving one `Option`
            // between two arrays is all that happens here.
            crate::user::orphan(current);
            // `retire_to`, not `switch_from_trap`: the thread is not coming
            // back, so `wost`'s slot must go `Dead` to match the `Free` this
            // table just wrote. Leaving it `Alive` makes the index unusable
            // for ever, which is the two tables disagreeing (D-117).
            task::retire_to(next, frame).is_ok()
        }
        None => false,
    }
}

/// How many threads have been retired from trap context.
///
/// A count rather than a state to inspect, because the retired thread's slot
/// stays taken and looks no different from a blocked one — which is what made
/// the first version of the check for this unable to fail (D-101).
static RETIREMENTS: AtomicUsize = AtomicUsize::new(0);

/// Threads retired from trap context since boot.
#[cfg(feature = "selftest")]
pub(crate) fn retirements() -> usize {
    RETIREMENTS.load(Ordering::Acquire)
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
pub(crate) fn block(context: &mut ThreadContext) {
    park(context, State::Blocked, None);
}

/// Make every blocked thread in `slots` runnable, and report how many that
/// was.
///
/// A bitmask because the caller's caller is an interrupt handler, where D-55
/// forbids the heap. A set-but-not-blocked slot is skipped: a thread that
/// registered and then found nothing to wait for is still registered.
pub(crate) fn unblock(slots: u32) -> usize {
    let woken = {
        let mut threads = THREADS.lock_leaf();
        let mut woken = 0;
        for slot in 0..CAPACITY {
            if slots & (1 << slot) == 0
                || !matches!(
                    threads.queue.slots[slot].state,
                    State::Blocked | State::Sleeping
                )
            {
                continue;
            }
            threads.queue.make_ready(slot);
            woken += 1;
        }
        woken
    };

    // A woken thread is runnable and nothing has been asked to run it, so this
    // is where the timer is armed for it — **unconditionally**. It used to skip
    // trap context, because the trap exit reschedules (D-54): true of the timer
    // and keyboard arms, which both end in `reschedule`, and false of the
    // syscall arm, which serves the ring and returns straight to ring 3. That
    // premise lived here and the arms it was about live in `trap.rs`, so
    // nothing compared them and they stopped agreeing (D-133).
    //
    // Arming twice costs one APIC write on a path that has just taken an
    // interrupt. Not arming at all costs a runnable thread nothing will run,
    // on a tickless kernel where there may be no other timer to save it
    // (D-57, D-93).
    if woken > 0 {
        if interrupt::in_trap() {
            HANDLER_ARMS.fetch_add(1, Ordering::Relaxed);
        }
        reschedule_timer(Some(time::now_ns()));
    }
    woken
}

/// Wakes delivered from a trap handler for which this armed the timer.
///
/// Zero for the whole of the kernel's life until D-133, because the arming
/// above excluded exactly this case. The keyboard is the only handler that
/// wakes anything today, which is what makes a zero here a falsifiable claim
/// rather than an absence of evidence.
static HANDLER_ARMS: AtomicUsize = AtomicUsize::new(0);

/// Wakes from a handler that armed the timer. See [`HANDLER_ARMS`].
#[cfg(feature = "selftest")]
pub(crate) fn handler_arms() -> usize {
    HANDLER_ARMS.load(Ordering::Relaxed)
}

/// The slot this CPU is running on.
pub(crate) fn current_slot() -> usize {
    task::current()
}

/// The name of the thread this CPU is running.
///
/// `try_lock`: the one caller is the panic handler, and a panic can be raised
/// by the code holding this.
pub(crate) fn current_name() -> &'static str {
    let slot = task::current();
    match THREADS.try_lock_leaf() {
        Some(threads) if !threads.identities[slot].name.is_empty() => threads.identities[slot].name,
        Some(_) => "unnamed",
        None => "unknown",
    }
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
    let switchable = frame.flags() & INTERRUPT_FLAG != 0;
    let (next, event) = {
        let mut threads = THREADS.lock_leaf();
        if threads.spawnable == 0 {
            return;
        }
        let current = task::current();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        let next = match switchable
            .then(|| threads.queue.choose(current, true))
            .flatten()
        {
            Some(next) if next != current => {
                threads.queue.hand_over(current, next, State::Ready, now);
                Some(next)
            }
            _ => None,
        };
        (next, threads.queue.next_event(next.unwrap_or(current), now))
    };

    reschedule_timer(event);
    if let Some(next) = next
        && let Err(error) = task::switch_from_trap(next, frame)
    {
        panic!("preemption to slot {next} refused: {error}");
    }
}

/// Give the CPU up until `wake_at`, from inside a trap handler.
///
/// The thread-context [`sleep_ns`] cannot be used from a syscall: it takes a
/// `ThreadContext`, and there is none inside a handler (D-55). But a ring-3
/// thread that has asked to wait has to stop running, or "wait" means "spin",
/// and a program spinning is a program holding a CPU it is not using.
///
/// So this is [`retire_from_trap`]'s shape without the retirement: mark the
/// caller `Sleeping` with its deadline, hand the CPU to whoever is next, and
/// switch **through the frame**, which is the same mechanism preemption uses
/// and the only place a switch is legal (D-54). The thread resumes inside its
/// own handler when the scheduler picks it again, and returns to ring 3 from
/// there.
///
/// Returns whether the caller actually gave up the CPU. `false` — no other
/// thread to run, or a clock that never started — means the caller must answer
/// its program now rather than pretend it waited, because a wait nobody can
/// end is a hang.
/// Give the CPU up for an event, optionally bounded by a deadline, from a trap.
///
/// A waiter with a deadline is `Sleeping`, but an event may make it ready
/// before the clock does. With no deadline it is `Blocked` until its wait
/// queue names it. Both resume through the saved trap frame (D-200).
pub(crate) fn wait_from_trap(frame: &TrapFrame, wake_at: Option<u64>) -> bool {
    if wake_at.is_some() && !time::is_calibrated() {
        return false;
    }
    let now = time::now_ns();
    if wake_at.is_some_and(|deadline| now >= deadline) {
        return false;
    }

    let current = task::current();
    let (next, event) = {
        let mut threads = THREADS.lock_leaf();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        if let Some(deadline) = wake_at {
            threads.queue.slots[current].wake_at = deadline;
        }
        threads.queue.park(current);
        let leaving = if wake_at.is_some() {
            State::Sleeping
        } else {
            State::Blocked
        };
        threads.queue.slots[current].state = leaving;

        match threads.queue.choose(current, false) {
            Some(next) => {
                if leaving == State::Blocked {
                    BLOCKS.fetch_add(1, Ordering::Relaxed);
                }
                threads.queue.slots[next].state = State::Running;
                threads.queue.slots[next].started = now;
                (Some(next), threads.queue.next_event(next, now))
            }
            // Nobody to hand the CPU to. A handler cannot halt — it is
            // standing on the interrupted thread's stack with that thread's
            // state in the frame — so the caller keeps the CPU and is told it
            // did not sleep. `park`'s own idle branch does the same thing for
            // the same reason.
            None => {
                threads.queue.slots[current].state = State::Running;
                (None, threads.queue.next_event(current, now))
            }
        }
    };

    reschedule_timer(earliest(event, wake_at));
    let switched = match next {
        Some(next) => task::switch_from_trap(next, frame).is_ok(),
        None => false,
    };
    if switched {
        TRAP_WAITS.fetch_add(1, Ordering::Relaxed);
    }
    switched
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
        let mut threads = THREADS.lock_leaf();
        let current = task::current();
        assert!(current != BOOT, "the boot thread cannot exit");
        // Charged before the slot is cleared, so a thread's last slice is
        // accounted like every other. It changes nothing while the slot is
        // freed immediately, and it is wrong the day exit accounting matters
        // (D-105).
        threads.queue.charge(current, now);
        threads.release(current);
        threads.queue.wake_due(now);
        let Some(next) = threads.queue.choose(current, false) else {
            panic!("nothing is runnable after slot {current} exited");
        };
        threads.queue.hand_over(current, next, State::Free, now);
        (next, threads.queue.next_event(next, now))
    };

    reschedule_timer(event);
    let error = task::exit_to(next);
    panic!("exit to slot {next} refused: {error}");
}

/// Where every spawned thread begins: run its entry, then leave.
extern "C" fn start(slot: usize) -> ! {
    let entry = THREADS.lock_leaf().identities[slot].entry;
    match entry {
        Some(entry) => entry(),
        None => panic!("slot {slot} started with no entry"),
    }
    exit()
}
