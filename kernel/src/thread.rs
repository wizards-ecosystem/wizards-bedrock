//! Kernel threads: who exists, who is ready, who is asleep, and who runs next.
//!
//! The mechanism is `wost::task`'s; this is policy in safe Rust: names, states and scheduling
//! entities over the same slot indices, fed to the host-tested `eevdf` with a round-robin
//! fallback if its answer is ever missing. The timer preempts at trap exit, and nothing runs on
//! a tick: every path that changes who is runnable ends by arming the timer for the next due
//! decision, or stopping it when there is none.
//!
//! Lock order (`docs/009-multitasking.md`): `THREADS` is a leaf, so nothing is taken while it is
//! held. Every switch happens after the guard has dropped, with the interrupt flag clear
//! throughout, so no expiry can preempt a half-made decision.

use core::fmt;
use core::sync::atomic::{AtomicU8, Ordering};

use eevdf::{BASE_WEIGHT, Entity};
use stack_layout::StackLayout;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::paging::{Binding, StackArena};
use wost::arch::current::task::{self, BOOT, Bound, InitFailure, Released, TaskError};
use wost::sync::SpinLock;
pub(crate) use wost::sync::ThreadContext;

use crate::time::{self, NANOSECONDS_PER_MS};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::{
    BLOCKS, HANDLER_ARMS, IDLE_HALTS, POLICY_FALLBACKS, RETIREMENTS, TRAP_WAITS, WAKES,
};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    blocks, free_slots, handler_arms, idle_halts, idle_is_started, idle_threads, policy_fallbacks,
    rearm_current_for_test, restore_idle_retry_for_test, retirements, spawn_in_space_refusing,
    spawn_weighted_for_test, trap_waits, wakes,
};

/// Threads that can exist at once besides the boot thread, matching the recyclable stack arena's
/// initial capacity; fixed until a measured caller needs more, since at least four processes must
/// be able to coexist.
pub(crate) const STACK_SLOTS: usize = 16;

/// Bytes per thread; the debug process-construction path exceeded 16 KiB.
const STACK_BYTES: u64 = 32 * 1024;

/// Bytes of unmapped gap below each stack.
const GUARD_BYTES: u64 = 4096;

/// Where the arena sits: below the device window, far above the direct map, so a stack address is
/// never mistaken for either.
const WINDOW: u64 = 0xffff_fe00_0000_0000;

/// Slots in the table, the boot thread's among them.
pub(crate) const CAPACITY: usize = STACK_SLOTS + 1;

/// Real time a thread may run before its deadline moves, in nanoseconds: 3 ms, in the range
/// mainline uses, and the one tunable EEVDF has.
const SLICE_NS: u64 = 3 * NANOSECONDS_PER_MS;

/// The arena the kernel asks `wost` to map.
pub(crate) fn layout() -> Option<StackLayout> {
    StackLayout::new(WINDOW, STACK_BYTES, GUARD_BYTES, STACK_SLOTS)
}

/// Where the double-fault handler's guarded stack goes: one arena slot and guard gap derived from
/// the thread arena, so it adds no hardcoded window and no entry to the KASLR ledger.
pub(crate) fn fault_stack_layout() -> Option<StackLayout> {
    StackLayout::new(layout()?.end(), STACK_BYTES, GUARD_BYTES, 1)
}

/// Where the boot thread's own guarded stack goes, derived from the fault stack for the same
/// reason. The bootloader's stack has nothing below it and lives in the unsplittable direct map,
/// so the only way to guard it is to leave it.
pub(crate) fn boot_stack_layout() -> Option<StackLayout> {
    StackLayout::new(fault_stack_layout()?.end(), STACK_BYTES, GUARD_BYTES, 1)
}

/// Why a thread could not be spawned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SpawnError {
    /// The arena was never mapped, so there is nothing to run a thread on.
    Unavailable,
    /// Every slot holds a live thread.
    Full,
    /// `wost` refused the slot.
    Task(TaskError),
    /// The caller could not publish the state the new thread needs.
    Arrangement,
    /// The first task-table retirement refused before an exact retry succeeded.
    Retirement {
        /// Slot withheld until the retry completed.
        slot: usize,
        /// Why the first retirement attempt refused.
        reason: TaskError,
    },
}

/// A refused user-space spawn with any returned address-space authority.
#[derive(Debug)]
pub(crate) struct SpaceSpawnError {
    /// Why thread construction stopped.
    pub(crate) reason: SpawnError,
    /// What remains of the attempted address-space installation.
    pub(crate) authority: SpaceAuthority,
}

/// Address-space authority returned by a refused thread construction.
#[derive(Debug)]
pub(crate) enum SpaceAuthority {
    /// No address space was supplied.
    None,
    /// Nothing can install or run the address space.
    Released(Released),
}

#[derive(Debug)]
struct PreparedFailure {
    reason: SpawnError,
    authority: SpaceAuthority,
}

impl PreparedFailure {
    fn before_binding(reason: SpawnError, binding: Option<Binding>) -> Self {
        Self {
            reason,
            authority: binding.map_or(SpaceAuthority::None, |binding| {
                SpaceAuthority::Released(task::release_uninstalled(binding))
            }),
        }
    }

    fn after_retirement(reason: SpawnError, bound: Option<Bound>) -> Self {
        let authority = match bound.map(task::release_address_space) {
            None => SpaceAuthority::None,
            Some(Ok(released)) => SpaceAuthority::Released(released),
            Some(Err(_)) => {
                panic!("a retired task kept its address-space binding");
            }
        };
        Self { reason, authority }
    }
}

impl fmt::Display for SpawnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unavailable => f.write_str("threads are unavailable"),
            Self::Full => write!(f, "all {STACK_SLOTS} thread slots are live"),
            Self::Task(reason) => write!(f, "{reason}"),
            Self::Arrangement => f.write_str("the thread's launch state could not be arranged"),
            Self::Retirement { slot, reason } => {
                write!(
                    f,
                    "slot {slot} retirement first refused with {reason}; retry completed"
                )
            }
        }
    }
}

impl core::error::Error for SpawnError {}

/// A completed thread-construction step at which a check may force refusal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SpawnPoint {
    /// The scheduler slot is reserved, but no task exists.
    Reserved,
    /// The task exists, but has no user address space.
    Fabricated,
    /// The task names the user address space, but is not runnable.
    Bound,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    /// A thread the policy schedules.
    Normal,
    /// The thread that runs when no normal one can, and never otherwise: outside the policy,
    /// because any weight, however small, would win it the CPU now and then for nothing.
    Idle,
}

/// Who a thread is: changed by `spawn` and `exit`, and read by nothing that schedules.
#[derive(Clone, Copy, Debug)]
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

/// Everything the policy needs about one slot and nothing else, so swapping `libs/eevdf` changes
/// [`RunQueue`] and this, and nothing that knows what a thread *is*.
#[derive(Clone, Copy, Debug)]
struct Scheduled {
    state: State,
    class: Class,
    entity: Entity,
    /// Uptime, in nanoseconds, when it last went on the CPU.
    started: u64,
    /// Uptime at which a sleeper becomes runnable again; meaningless in every other state.
    wake_at: u64,
    /// The lag it stopped being runnable with, which it is placed back with; the policy clamps it
    /// to a slice either way, so a brief wait sheds no debt and a long one banks no credit.
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

/// Who runs next: one [`Scheduled`] per slot and the policy over them, unable to name a thread.
#[derive(Debug)]
struct RunQueue {
    slots: [Scheduled; CAPACITY],
}

/// Who exists, and who runs next, under one lock because every runnability decision happens with
/// the other half settled; the queue's methods take `&RunQueue`, so the policy cannot reach a name.
#[derive(Debug)]
struct Threads {
    identities: [Identity; CAPACITY],
    queue: RunQueue,
    /// Slots the arena backs, or zero when threads are unavailable.
    spawnable: usize,
}

impl Threads {
    /// Give a slot back in both halves in one call: a slot free in only one is a thread with no
    /// name or a name with no thread.
    fn release(&mut self, slot: usize) {
        assert!(
            slot != BOOT && slot <= self.spawnable && self.queue.slots[slot].state != State::Free,
            "only a live non-boot thread slot can be released"
        );
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

    /// How many threads the policy could pick right now, never the idle thread, which is what
    /// makes an otherwise-empty machine tickless.
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

    /// Make every sleeper whose deadline has passed runnable again, at the fair-share point offset
    /// by its lag; revoking the clock wakes them all, since none may remain parked on an
    /// unreachable deadline.
    fn wake_due(&mut self, now: u64) {
        let clock_live = crate::time::is_calibrated();
        for slot in 0..CAPACITY {
            let thread = &self.slots[slot];
            if thread.state != State::Sleeping || (clock_live && thread.wake_at > now) {
                continue;
            }
            self.make_ready(slot);
        }
    }

    /// Put a parked thread back on the queue, at the fair-share point offset by its lag.
    fn make_ready(&mut self, slot: usize) {
        let average = self.average().unwrap_or(self.slots[slot].entity.vruntime());
        let lag = self.slots[slot].parked_with_lag;
        let thread = &mut self.slots[slot];
        thread.entity.place(average, lag, SLICE_NS);
        thread.state = State::Ready;
        #[cfg(feature = "selftest")]
        WAKES.fetch_add(1, Ordering::Relaxed);
    }

    /// Record what a thread about to stop being runnable is owed, so that
    /// [`make_ready`](Self::make_ready) can give it back.
    fn park(&mut self, slot: usize) {
        let average = self.average().unwrap_or(0);
        let thread = &mut self.slots[slot];
        thread.parked_with_lag = thread.entity.lag(average);
    }

    /// Which runnable slot should run, searched from just after `current` so equals take turns;
    /// `eligibility` says whether `current` is a candidate. A thread *leaving* the CPU records that
    /// first, making the idle thread reachable; one merely offering it keeps it if nobody else is
    /// owed one.
    fn choose(&self, current: usize, eligibility: CurrentEligibility) -> Option<usize> {
        let candidates = || {
            (1..=CAPACITY)
                .map(|offset| (current + offset) % CAPACITY)
                .filter(|slot| *slot != current || eligibility == CurrentEligibility::Eligible)
                .filter(|slot| self.slots[*slot].is_scheduled())
        };
        // The idle thread answers only when there is no other, never replacing a normal thread
        // that has the CPU.
        let Some(average) = self.average() else {
            return self.idle().filter(|slot| *slot != current);
        };
        let picked = eevdf::pick(
            candidates().map(|slot| (slot, &self.slots[slot].entity)),
            average,
        );
        match picked {
            Some(slot) => Some(slot),
            // With the running thread excluded, nobody eligible is an honest
            // answer: it is behind the average and keeps the CPU.
            None if eligibility == CurrentEligibility::Excluded => None,
            // With everyone considered, nobody eligible cannot happen per `eevdf`'s tests;
            // counted, and answered with round-robin.
            None => {
                let fallback = core::iter::Iterator::next(&mut candidates());
                #[cfg(feature = "selftest")]
                if fallback.is_some() {
                    POLICY_FALLBACKS.fetch_add(1, Ordering::Relaxed);
                }
                fallback
            }
        }
    }

    /// When the kernel next has a reason to be interrupted while `running` is on the CPU, if ever.
    /// Two reasons only: a slice end when somebody else could take the CPU, and a sleeper's
    /// deadline always, because nothing else will notice it.
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CurrentEligibility {
    Eligible,
    Excluded,
}

static THREADS: SpinLock<Threads> = SpinLock::new(Threads {
    identities: [Identity::FREE; CAPACITY],
    queue: RunQueue {
        slots: [Scheduled::FREE; CAPACITY],
    },
    spawnable: 0,
});

const IDLE_UNSTARTED: u8 = 0;
const IDLE_STARTING: u8 = 1;
const IDLE_STARTED: u8 = 2;

/// Reserved while an idle spawn is in flight and committed only on success.
static IDLE_STATE: AtomicU8 = AtomicU8::new(IDLE_UNSTARTED);

/// Take the mapped arena into use, making the caller the boot thread, or say what `wost` found
/// wrong with it.
pub(crate) fn init(arena: StackArena) -> Result<usize, InitFailure> {
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
        started: time::scheduler_now_ns(),
        ..Scheduled::FREE
    };
    Ok(threads.spawnable)
}

/// Whether threads can be spawned at all.
pub(crate) fn is_available() -> bool {
    THREADS.lock_leaf().spawnable > 0
}

/// Start the thread that runs when nothing else can, outside the policy ([`Class::Idle`]): an idle
/// machine halts on it and takes no interrupts, and everything else may assume something runs.
/// Does nothing once started, while another attempt is active, or with no threads; a refused
/// spawn leaves a later call free to retry.
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
        Prepared {
            name: "idle",
            entry: idle_loop,
            weight: BASE_WEIGHT,
            binding: None,
        },
        |slot, bound| {
            if bound.is_some() {
                return Err(bound);
            }
            THREADS.lock_leaf().queue.slots[slot].class = Class::Idle;
            Ok(())
        },
        |_| false,
    );
    if spawned.is_err() {
        IDLE_STATE.store(IDLE_UNSTARTED, Ordering::Release);
        return;
    }
    IDLE_STATE.store(IDLE_STARTED, Ordering::Release);
}

fn idle_loop() {
    loop {
        // Reclaim one process per idle pass, away from its dying stack.
        if crate::user::collect().is_some() {
            continue;
        }
        // Syscall handlers defer reporting because they cannot take the console lock.
        if crate::user::report_execution() || crate::user::report_refusals() {
            continue;
        }
        #[cfg(feature = "selftest")]
        IDLE_HALTS.fetch_add(1, Ordering::Relaxed);
        cpu::halt();
    }
}

/// Create a thread of the base weight that will run `entry`.
pub(crate) fn spawn(name: &'static str, entry: fn()) -> Result<usize, SpawnError> {
    spawn_prepared(
        Prepared {
            name,
            entry,
            weight: BASE_WEIGHT,
            binding: None,
        },
        |_, bound| {
            if bound.is_none() { Ok(()) } else { Err(bound) }
        },
        |_| false,
    )
    .map_err(|failure| failure.reason)
}

/// Create a thread that runs on `binding`'s tables, refused as [`spawn`] is, or when `wost`
/// refuses the binding or `arrange` returns the token. `arrange` receives the installed-space
/// token while the new slot is still reserved, and must store it with the process state before
/// the slot can become runnable.
pub(crate) fn spawn_in_space(
    name: &'static str,
    entry: fn(),
    binding: Binding,
    arrange: impl FnOnce(usize, Bound) -> Result<(), Bound>,
) -> Result<usize, SpaceSpawnError> {
    spawn_prepared(
        Prepared {
            name,
            entry,
            weight: BASE_WEIGHT,
            binding: Some(binding),
        },
        move |slot, bound| {
            let Some(bound) = bound else {
                return Err(None);
            };
            arrange(slot, bound).map_err(Some)
        },
        |_| false,
    )
    .map_err(|failure| SpaceSpawnError {
        reason: failure.reason,
        authority: failure.authority,
    })
}

fn retire_prepared(slot: usize) -> Result<(), TaskError> {
    task::retire(slot)?;
    THREADS.lock_leaf().release(slot);
    Ok(())
}

fn finish_refused_retirement(slot: usize, first: TaskError) {
    if let Err(second) = retire_prepared(slot) {
        panic!("slot {slot} retirement refused twice: {first}; {second}");
    }
}

/// What a thread is spawned from: its identity, its weight, and any user address space.
#[derive(Debug)]
struct Prepared {
    name: &'static str,
    entry: fn(),
    weight: u32,
    binding: Option<Binding>,
}

fn spawn_prepared(
    prepared: Prepared,
    arrange: impl FnOnce(usize, Option<Bound>) -> Result<(), Option<Bound>>,
    mut refuse: impl FnMut(SpawnPoint) -> bool,
) -> Result<usize, PreparedFailure> {
    let Prepared {
        name,
        entry,
        weight,
        binding,
    } = prepared;
    // Before looking for a slot: a machine too busy to idle would otherwise never collect, and a
    // spawn is what most needs a freed slot.
    while crate::user::has_zombies() && crate::user::collect().is_some() {}

    let slot = match reserve(name, entry, weight) {
        Ok(slot) => slot,
        Err(reason) => return Err(PreparedFailure::before_binding(reason, binding)),
    };
    if refuse(SpawnPoint::Reserved) {
        THREADS.lock_leaf().release(slot);
        return Err(PreparedFailure::before_binding(SpawnError::Full, binding));
    }

    // Outside the leaf: `fabricate` takes `wost`'s own lock, and the reserved slot cannot be
    // chosen by another spawn or an expiry.
    if let Err(reason) = task::fabricate(slot, start, slot) {
        THREADS.lock_leaf().release(slot);
        return Err(PreparedFailure::before_binding(
            SpawnError::Task(reason),
            binding,
        ));
    }
    if refuse(SpawnPoint::Fabricated) {
        if let Err(reason) = retire_prepared(slot) {
            finish_refused_retirement(slot, reason);
            return Err(PreparedFailure::before_binding(
                SpawnError::Retirement { slot, reason },
                binding,
            ));
        }
        return Err(PreparedFailure::before_binding(SpawnError::Full, binding));
    }

    let bound = bind(slot, binding)?;
    if refuse(SpawnPoint::Bound) {
        if let Err(reason) = retire_prepared(slot) {
            finish_refused_retirement(slot, reason);
            return Err(PreparedFailure::after_retirement(
                SpawnError::Retirement { slot, reason },
                bound,
            ));
        }
        return Err(PreparedFailure::after_retirement(SpawnError::Full, bound));
    }
    if let Err(bound) = arrange(slot, bound) {
        if let Err(reason) = retire_prepared(slot) {
            finish_refused_retirement(slot, reason);
            return Err(PreparedFailure::after_retirement(
                SpawnError::Retirement { slot, reason },
                bound,
            ));
        }
        return Err(PreparedFailure::after_retirement(
            SpawnError::Arrangement,
            bound,
        ));
    }

    make_runnable(slot);
    Ok(slot)
}

/// Reserve a free slot no zombie still owns for `name`, not yet runnable.
fn reserve(name: &'static str, entry: fn(), weight: u32) -> Result<usize, SpawnError> {
    let zombie_slots = crate::user::zombie_slots_for_spawn();
    let slot = {
        let mut threads = THREADS.lock_leaf();
        if threads.spawnable == 0 {
            return Err(SpawnError::Unavailable);
        }
        let Some(slot) = (1..=threads.spawnable)
            .find(|slot| threads.queue.slots[*slot].state == State::Free && !zombie_slots[*slot])
        else {
            return Err(SpawnError::Full);
        };
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
    if crate::user::zombie_slots_for_spawn()[slot] {
        THREADS.lock_leaf().release(slot);
        return Err(SpawnError::Full);
    }
    Ok(slot)
}

/// Bind a fabricated slot to its user tables while it is still `Reserved`, so they and the launch
/// state are in place before the first instruction; a refusal retires the slot.
fn bind(slot: usize, binding: Option<Binding>) -> Result<Option<Bound>, PreparedFailure> {
    let Some(binding) = binding else {
        return Ok(None);
    };
    match task::bind_address_space(slot, binding) {
        Ok(bound) => Ok(Some(bound)),
        Err((reason, binding)) => {
            if let Err(retirement) = retire_prepared(slot) {
                finish_refused_retirement(slot, retirement);
                return Err(PreparedFailure::before_binding(
                    SpawnError::Retirement {
                        slot,
                        reason: retirement,
                    },
                    Some(binding),
                ));
            }
            Err(PreparedFailure::before_binding(
                SpawnError::Task(reason),
                Some(binding),
            ))
        }
    }
}

/// Place a reserved slot at the fair-share point and make it ready, arming even though nothing
/// switches here: a second runnable thread is when the running slice starts mattering.
fn make_runnable(slot: usize) {
    let saved = interrupt::disable();
    let now = time::scheduler_now_ns();
    let event = {
        let mut threads = THREADS.lock_leaf();
        let average = threads.queue.average().unwrap_or(0);
        threads.queue.slots[slot].entity.place(average, 0, SLICE_NS);
        threads.queue.slots[slot].state = State::Ready;
        threads.queue.next_event(task::current(), now)
    };
    reschedule_timer(event);
    saved.restore();
}

/// Let another runnable thread have the CPU, if any is owed it, returning when this thread is
/// picked again, at once if nothing else is eligible. Panics from trap context, where a switch
/// would carry the handler's stack frame away, or if `wost` refuses the switch: the two tables
/// disagree, which is a bug.
pub(crate) fn yield_now(context: &mut ThreadContext) {
    // Flag clear from decision to switch, so an expiry cannot preempt between them and act on a
    // table that says `next` is running when it is not.
    let saved = interrupt::disable();
    let now = time::scheduler_now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock(context);
        let current = task::current();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        let next = threads.queue.choose(current, CurrentEligibility::Excluded);
        if let Some(next) = next {
            threads.queue.hand_over(current, next, State::Ready, now);
        }
        (next, threads.queue.next_event(next.unwrap_or(current), now))
    };

    reschedule_timer(event);
    if let Some(next) = next
        && let Err(reason) = task::switch_to(next)
    {
        panic!("yield to slot {next} refused: {reason}");
    }
    saved.restore();
}

/// Give the CPU up until `milliseconds` have passed, to another runnable thread or to a halt until
/// the armed timer fires; nothing is interrupted earlier unless somebody else asked for it.
/// Returns no earlier than the deadline, possibly later, or at once on a machine whose clock
/// never started, where a wait would hang. Panics from trap context, where a switch would carry
/// the handler's stack frame away, or if `wost` refuses a switch.
pub(crate) fn sleep_ms(context: &mut ThreadContext, milliseconds: u64) {
    sleep_ns(context, milliseconds.saturating_mul(NANOSECONDS_PER_MS));
}

/// Give the CPU up until `nanoseconds` have passed, as [`sleep_ms`] does.
pub(crate) fn sleep_ns(context: &mut ThreadContext, nanoseconds: u64) {
    if !time::is_calibrated() {
        return;
    }

    let mut saved = interrupt::disable();
    let wake_at = time::now_ns().saturating_add(nanoseconds);
    // A loop: the halt below returns on any interrupt, and the deadline is what this promised.
    while time::is_calibrated() && time::now_ns() < wake_at {
        THREADS.lock(context).queue.slots[task::current()].wake_at = wake_at;
        park(context, State::Sleeping, Some(wake_at), &mut saved);
    }
    saved.restore();
}

/// Mark the running thread `leaving`, record what it is owed and hand the CPU on; with nobody to
/// take it, keep it and halt, and the caller re-checks, since a halt returns on any interrupt.
/// `until` bounds the arming so a sleeper halting alone wakes on its own timer. Called with the
/// flag already clear, and it stays clear until after the switch.
fn park(
    context: &mut ThreadContext,
    leaving: State,
    until: Option<u64>,
    saved: &mut interrupt::InterruptState,
) {
    let now = time::scheduler_now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock(context);
        let current = task::current();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        threads.queue.park(current);
        threads.queue.slots[current].state = leaving;

        match threads.queue.choose(current, CurrentEligibility::Excluded) {
            Some(next) => {
                // Counted here, not at the call: a thread with nobody to hand the CPU to never
                // leaves the run queue.
                #[cfg(feature = "selftest")]
                if leaving == State::Blocked {
                    BLOCKS.fetch_add(1, Ordering::Relaxed);
                }
                threads.queue.slots[next].state = State::Running;
                threads.queue.slots[next].started = now;
                (Some(next), threads.queue.next_event(next, now))
            }
            // A sole runnable thread remains charged while it waits, because it still owns the CPU.
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
            if let Err(reason) = task::switch_to(next) {
                panic!("park of slot {} refused: {reason}", task::current());
            }
        }
        None => {
            // Enabling and halting must be one step, or the ending interrupt can land between them
            // and nothing is left to wake the CPU.
            saved.wait_for_interrupt();
        }
    }
}

/// Take the trapping thread off the run queue for good, from trap context, which cannot mint the
/// [`ThreadContext`] `park` wants; it switches away at trap exit exactly as preemption does.
/// `false` if there is nobody to switch to, impossible once the idle thread exists, or if `wost`
/// refuses the switch.
pub(crate) fn retire_from_trap(frame: &TrapFrame) -> bool {
    let now = time::scheduler_now_ns();
    let current = task::current();
    let (next, event) = {
        let mut threads = THREADS.lock_leaf();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        threads.queue.park(current);
        // The switch below leaves this stack permanently, so the scheduler slot can represent
        // another live program.
        threads.identities[current] = Identity::FREE;
        threads.queue.slots[current] = Scheduled::FREE;
        match threads.queue.choose(current, CurrentEligibility::Excluded) {
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
            // A zombie: this thread stands on the stack and tables it would have to free, so it
            // cannot be the one to free them.
            crate::user::orphan(current);
            // Published after `orphan` records the last rites, so an acquiring observer of the new
            // generation can read them.
            #[cfg(feature = "selftest")]
            RETIREMENTS.fetch_add(1, Ordering::AcqRel);
            // `retire_to`, not `switch_from_trap`: `wost`'s slot must go `Dead` to match the
            // `Free` just written, or the index is lost.
            task::retire_to(next, frame).is_ok()
        }
        None => false,
    }
}

/// The sooner of two deadlines, either of which may be absent.
fn earliest(one: Option<u64>, other: Option<u64>) -> Option<u64> {
    match (one, other) {
        (Some(one), Some(other)) => Some(one.min(other)),
        (only, None) | (None, only) => only,
    }
}

/// Stop being runnable until [`unblock`] names this thread: no deadline, so nothing is armed and
/// a machine with nothing else to do takes no interrupts until the event.
pub(crate) fn block(context: &mut ThreadContext, saved: &mut interrupt::InterruptState) {
    park(context, State::Blocked, None, saved);
}

/// Make every blocked thread in `slots` runnable and report how many: a bitmask because the
/// caller's caller is an interrupt handler, where the heap is forbidden. A set slot that is not
/// blocked is skipped: a thread that registered and then found nothing to wait for is still
/// registered.
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

    // Arm unconditionally: the syscall path returns straight to ring 3 and would otherwise strand
    // the woken thread on a tickless machine.
    if woken > 0 {
        #[cfg(feature = "selftest")]
        if interrupt::in_trap() {
            HANDLER_ARMS.fetch_add(1, Ordering::Relaxed);
        }
        reschedule_timer(Some(time::scheduler_now_ns()));
    }
    woken
}

/// The slot this CPU is running on.
pub(crate) fn current_slot() -> usize {
    task::current()
}

/// The name of the thread this CPU is running, by `try_lock` because the one caller is the panic
/// handler, and a panic can be raised by the code holding this.
pub(crate) fn current_name() -> &'static str {
    let slot = task::current();
    match THREADS.try_lock_leaf() {
        Some(threads) if !threads.identities[slot].name.is_empty() => threads.identities[slot].name,
        Some(_) => "unnamed",
        None => "unknown",
    }
}

/// Charge, wake, optionally switch, and re-arm at trap exit; a cleared interrupted flag prevents
/// only the switch, and deadlines still advance and re-arm because no later interrupt is promised.
pub(crate) fn reschedule(frame: &TrapFrame) {
    let now = time::scheduler_now_ns();
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
            .then(|| threads.queue.choose(current, CurrentEligibility::Eligible))
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
        && let Err(reason) = task::switch_from_trap(next, frame)
    {
        panic!("preemption to slot {next} refused: {reason}");
    }
}

/// Give up the CPU for an event, optionally bounded by `wake_at`, from a trap, switching through
/// the saved frame because a handler has no [`ThreadContext`]. `false` means no peer or usable
/// clock can end the wait; a refused switch is terminal, because returning would leave two
/// scheduler states claiming the CPU.
pub(crate) fn wait_from_trap(frame: &TrapFrame, wake_at: Option<u64>) -> bool {
    if wake_at.is_some() && !time::is_calibrated() {
        return false;
    }
    let now = time::scheduler_now_ns();
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

        match threads.queue.choose(current, CurrentEligibility::Excluded) {
            Some(next) => {
                #[cfg(feature = "selftest")]
                if leaving == State::Blocked {
                    BLOCKS.fetch_add(1, Ordering::Relaxed);
                }
                threads.queue.slots[next].state = State::Running;
                threads.queue.slots[next].started = now;
                (Some(next), threads.queue.next_event(next, now))
            }
            // Nobody to hand the CPU to, and a handler cannot halt on the interrupted thread's
            // stack, so the caller is told it did not sleep.
            None => {
                threads.queue.slots[current].state = State::Running;
                (None, threads.queue.next_event(current, now))
            }
        }
    };

    reschedule_timer(earliest(event, wake_at));
    let Some(next) = next else {
        return false;
    };
    if let Err(reason) = task::switch_from_trap(next, frame) {
        panic!("trap wait switch to slot {next} refused: {reason}");
    }
    #[cfg(feature = "selftest")]
    TRAP_WAITS.fetch_add(1, Ordering::Relaxed);
    true
}

/// Arm the timer for the next thing owed a decision, or stop it if nothing is; every path that
/// can change the answer ends here, so an idle machine leaves with the timer stopped.
fn reschedule_timer(event: Option<u64>) {
    match event {
        Some(deadline) => time::arm_at(deadline),
        None => time::disarm(),
    }
}

/// `RFLAGS` bit 9, restated here because the test is against what the CPU pushed.
const INTERRUPT_FLAG: u64 = 1 << 9;

/// End the running thread and let the next runnable one run. Panics from the boot thread, which
/// owns the stack everything else was started from, or if nothing is runnable; neither can happen
/// while the boot thread is ready, which it is whenever it is not running.
pub(crate) fn exit() -> ! {
    // Never restored: an expiry between the decision and the switch would find a thread neither
    // running nor free.
    let _off = interrupt::disable();
    let now = time::scheduler_now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock_leaf();
        let current = task::current();
        assert!(current != BOOT, "the boot thread cannot exit");
        // Charged before the slot clears, so the last slice is accounted like every other.
        threads.queue.charge(current, now);
        threads.release(current);
        threads.queue.wake_due(now);
        let Some(next) = threads.queue.choose(current, CurrentEligibility::Excluded) else {
            panic!("nothing is runnable after slot {current} exited");
        };
        threads.queue.hand_over(current, next, State::Free, now);
        (next, threads.queue.next_event(next, now))
    };

    reschedule_timer(event);
    let reason = task::exit_to(next);
    panic!("exit to slot {next} refused: {reason}");
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
