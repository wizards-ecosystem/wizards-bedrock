use core::fmt;
use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use eevdf::{BASE_WEIGHT, Entity};
use stack_layout::StackLayout;
use wost::arch::current::cpu;
use wost::arch::current::interrupt::{self, TrapFrame};
use wost::arch::current::paging::{AddressSpace, Binding, StackArena};
use wost::arch::current::task::{self, BOOT, Bound, InitFailure, Released, TaskError};
use wost::sync::SpinLock;
pub(crate) use wost::sync::ThreadContext;

use crate::rank;
use crate::time::{self, NANOSECONDS_PER_MS};
use crate::timepage::WorldClock;
use crate::warn;

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::RETIREMENTS;
#[cfg(feature = "selftest")]
pub(crate) use selftest::{free_slots, retirements};

// At least four processes must be able to coexist.
pub(crate) const STACK_SLOTS: usize = 16;

// The debug process-construction path exceeded 16 KiB.
const STACK_BYTES: u64 = 32 * 1024;

const GUARD_BYTES: u64 = 4096;

// Below the device window, far above the direct map, so a stack address is never either.
const WINDOW: u64 = 0xffff_fe00_0000_0000;

pub(crate) const CAPACITY: usize = STACK_SLOTS + 1;

// In the range mainline uses, and the one tunable EEVDF has.
const SLICE_NS: u64 = 3 * NANOSECONDS_PER_MS;

pub(crate) fn layout() -> Option<StackLayout> {
    StackLayout::new(WINDOW, STACK_BYTES, GUARD_BYTES, STACK_SLOTS)
}

// Derived from the arena, so it adds no hardcoded window and no KASLR ledger entry.
pub(crate) fn boot_stack_layout() -> Option<StackLayout> {
    StackLayout::new(layout()?.end(), STACK_BYTES, GUARD_BYTES, 1)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SpawnError {
    Unavailable,
    Full,
    Task(TaskError),
    Arrangement,
}

#[derive(Debug)]
pub(crate) struct SpaceSpawnError {
    pub(crate) reason: SpawnError,
    pub(crate) authority: SpaceAuthority,
}

#[derive(Debug)]
pub(crate) enum SpaceAuthority {
    None,
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unavailable => formatter.write_str("threads are unavailable"),
            Self::Full => write!(formatter, "all {STACK_SLOTS} thread slots are live"),
            Self::Task(reason) => write!(formatter, "{reason}"),
            Self::Arrangement => {
                formatter.write_str("the thread's launch state could not be arranged")
            }
        }
    }
}

impl core::error::Error for SpawnError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Free,
    Reserved,
    Ready,
    Running,
    Sleeping,
    Blocked,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Normal,
    // Outside the policy: any weight, however small, would win it the CPU now and then.
    Idle,
}

#[derive(Clone, Copy, Debug)]
struct Identity {
    name: &'static str,
    entry: Option<(Entry, usize)>,
}

impl Identity {
    const FREE: Self = Self {
        name: "",
        entry: None,
    };
}

#[derive(Clone, Copy, Debug)]
struct Scheduled {
    state: State,
    class: Class,
    entity: Entity,
    started: u64,
    wake_at: u64,
    parked_with_lag: i64,
    // The world whose time page this thread reads while it runs in ring 3.
    clock: Option<&'static WorldClock>,
}

impl Scheduled {
    const FREE: Self = Self {
        state: State::Free,
        class: Class::Normal,
        entity: Entity::new(BASE_WEIGHT),
        started: 0,
        wake_at: 0,
        parked_with_lag: 0,
        clock: None,
    };

    const fn is_runnable(&self) -> bool {
        matches!(self.state, State::Ready | State::Running)
    }

    const fn is_scheduled(&self) -> bool {
        self.is_runnable() && matches!(self.class, Class::Normal)
    }
}

#[derive(Debug)]
struct RunQueue {
    slots: [Scheduled; CAPACITY],
}

pub(crate) const NO_WAITER: usize = usize::MAX;

#[derive(Debug)]
struct Threads {
    identities: [Identity; CAPACITY],
    queue: RunQueue,
    spawnable: usize,
    // A wait queue's head and these links change only under `THREADS`, so `Relaxed` suffices.
    next_waiter: [usize; CAPACITY],
}

impl Threads {
    fn release(&mut self, slot: usize) {
        assert!(
            slot != BOOT && slot <= self.spawnable && self.queue.slots[slot].state != State::Free,
            "only a live non-boot thread slot can be released"
        );
        assert!(
            self.next_waiter[slot] == NO_WAITER,
            "a released thread is still linked into a wait queue"
        );
        self.identities[slot] = Identity::FREE;
        self.queue.slots[slot] = Scheduled::FREE;
    }

    fn link_waiter(&mut self, head: &AtomicUsize, slot: usize) {
        self.next_waiter[slot] = NO_WAITER;
        let first = head.load(Ordering::Relaxed);
        if first == NO_WAITER {
            head.store(slot, Ordering::Relaxed);
            return;
        }
        let mut last = first;
        while self.next_waiter[last] != NO_WAITER {
            last = self.next_waiter[last];
        }
        self.next_waiter[last] = slot;
    }

    fn unlink_waiter(&mut self, head: &AtomicUsize, slot: usize) {
        let first = head.load(Ordering::Relaxed);
        if first == slot {
            head.store(self.next_waiter[slot], Ordering::Relaxed);
        } else {
            let mut cursor = first;
            while cursor != NO_WAITER && self.next_waiter[cursor] != slot {
                cursor = self.next_waiter[cursor];
            }
            if cursor != NO_WAITER {
                self.next_waiter[cursor] = self.next_waiter[slot];
            }
        }
        self.next_waiter[slot] = NO_WAITER;
    }
}

impl RunQueue {
    fn average(&self) -> Option<u64> {
        eevdf::average_vruntime(
            self.slots
                .iter()
                .filter(|thread| thread.is_scheduled())
                .map(|thread| &thread.entity),
        )
    }

    fn runnable(&self) -> usize {
        self.slots
            .iter()
            .filter(|thread| thread.is_scheduled())
            .count()
    }

    fn idle(&self) -> Option<usize> {
        self.slots
            .iter()
            .position(|thread| thread.class == Class::Idle && thread.is_runnable())
    }

    fn charge(&mut self, slot: usize, now: u64) {
        let thread = &mut self.slots[slot];
        thread.entity.charge(now.saturating_sub(thread.started));
        thread.started = now;
        if thread.entity.slice_is_spent() {
            thread.entity.renew_deadline(SLICE_NS);
        }
    }

    // A revoked clock wakes every sleeper: none may stay parked on an unreachable deadline.
    fn wake_due(&mut self, now: u64) {
        let clock_live = time::is_calibrated();
        for slot in 0..CAPACITY {
            let thread = &self.slots[slot];
            if thread.state != State::Sleeping || (clock_live && thread.wake_at > now) {
                continue;
            }
            self.ready(slot);
        }
    }

    fn ready(&mut self, slot: usize) {
        let average = self.average().unwrap_or(self.slots[slot].entity.vruntime());
        let lag = self.slots[slot].parked_with_lag;
        let thread = &mut self.slots[slot];
        thread.entity.place(average, lag, SLICE_NS);
        thread.state = State::Ready;
    }

    fn park(&mut self, slot: usize) {
        let average = self.average().unwrap_or(0);
        let thread = &mut self.slots[slot];
        thread.parked_with_lag = thread.entity.lag(average);
    }

    fn choose(&self, current: usize, eligibility: CurrentEligibility) -> Option<usize> {
        let candidates = || {
            (1..=CAPACITY)
                .map(|offset| (current + offset) % CAPACITY)
                .filter(|slot| *slot != current || eligibility == CurrentEligibility::Eligible)
                .filter(|slot| self.slots[*slot].is_scheduled())
        };
        // The idle thread answers only when there is no other, never replacing a normal thread.
        let Some(average) = self.average() else {
            return self.idle().filter(|slot| *slot != current);
        };
        let picked = eevdf::pick(
            candidates().map(|slot| (slot, &self.slots[slot].entity)),
            average,
        );
        match picked {
            Some(slot) => Some(slot),
            // With the running thread excluded, nobody eligible is honest: it keeps the CPU.
            None if eligibility == CurrentEligibility::Excluded => None,
            // With everyone considered `eevdf` always finds someone; round-robin is the backstop.
            None => candidates().next(),
        }
    }

    fn next_event(&self, running: usize, now: u64) -> Option<u64> {
        let slice_end = (self.runnable() > 1)
            .then(|| now.saturating_add(self.slots[running].entity.remaining_slice()));
        let earliest_sleeper = self
            .slots
            .iter()
            .filter(|thread| thread.state == State::Sleeping)
            .map(|thread| thread.wake_at)
            .min();
        // Only while a thread with a world clock runs, so an idle kernel still takes no alarm.
        let clock_refresh = self.slots[running]
            .clock
            .map(|clock| now.saturating_add(clock.refresh_interval_ns()));

        earliest(earliest(slice_end, earliest_sleeper), clock_refresh)
    }

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

// Every switch happens after its guard drops, with interrupts off throughout.
static THREADS: SpinLock<rank::Scheduler, Threads> = SpinLock::new(Threads {
    identities: [Identity::FREE; CAPACITY],
    queue: RunQueue {
        slots: [Scheduled::FREE; CAPACITY],
    },
    spawnable: 0,
    next_waiter: [NO_WAITER; CAPACITY],
});

const IDLE_UNSTARTED: u8 = 0;
const IDLE_STARTING: u8 = 1;
const IDLE_STARTED: u8 = 2;

static IDLE_STATE: AtomicU8 = AtomicU8::new(IDLE_UNSTARTED);

fn init(arena: StackArena) -> Result<usize, InitFailure> {
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

pub(crate) fn start_threads(space: &mut AddressSpace, arena: StackArena) -> Option<usize> {
    match init(arena) {
        Ok(slots) => Some(slots),
        Err(failure) => {
            let reason = failure.reason;
            space.unmap_stacks(failure.arena);
            warn!("threads: {reason}; no threads");
            None
        }
    }
}

pub(crate) fn is_available() -> bool {
    THREADS.lock_leaf().spawnable > 0
}

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
            argument: 0,
            binding: None,
            clock: None,
        },
        |slot, bound| {
            if bound.is_some() {
                return Err(bound);
            }
            THREADS.lock_leaf().queue.slots[slot].class = Class::Idle;
            Ok(())
        },
    );
    if spawned.is_err() {
        IDLE_STATE.store(IDLE_UNSTARTED, Ordering::Release);
        return;
    }
    IDLE_STATE.store(IDLE_STARTED, Ordering::Release);
}

fn idle_loop(_: &mut ThreadContext, _: usize) {
    loop {
        // Syscall handlers defer reporting because they cannot take the console lock.
        if crate::user::report_execution() || crate::user::report_refusals() {
            continue;
        }
        cpu::halt();
    }
}

pub(crate) type Entry = fn(&mut ThreadContext, usize);

pub(crate) fn spawn(
    name: &'static str,
    entry: Entry,
    argument: usize,
) -> Result<usize, SpawnError> {
    spawn_prepared(
        Prepared {
            name,
            entry,
            argument,
            binding: None,
            clock: None,
        },
        |_, bound| {
            if bound.is_none() { Ok(()) } else { Err(bound) }
        },
    )
    .map_err(|failure| failure.reason)
}

// `arrange` must store the bound token with the process state while the slot is reserved.
pub(crate) fn spawn_in_space(
    name: &'static str,
    entry: Entry,
    argument: usize,
    binding: Binding,
    clock: &'static WorldClock,
    arrange: impl FnOnce(usize, Bound) -> Result<(), Bound>,
) -> Result<usize, SpaceSpawnError> {
    spawn_prepared(
        Prepared {
            name,
            entry,
            argument,
            binding: Some(binding),
            clock: Some(clock),
        },
        move |slot, bound| {
            let Some(bound) = bound else {
                return Err(None);
            };
            arrange(slot, bound).map_err(Some)
        },
    )
    .map_err(|failure| SpaceSpawnError {
        reason: failure.reason,
        authority: failure.authority,
    })
}

// Every refusal `task::retire` can give repeats on a retry, so the first one is terminal.
fn retire_prepared(slot: usize) {
    if let Err(reason) = task::retire(slot) {
        panic!("slot {slot} retirement refused: {reason}");
    }
    THREADS.lock_leaf().release(slot);
}

#[derive(Debug)]
struct Prepared {
    name: &'static str,
    entry: Entry,
    argument: usize,
    binding: Option<Binding>,
    clock: Option<&'static WorldClock>,
}

fn spawn_prepared(
    prepared: Prepared,
    arrange: impl FnOnce(usize, Option<Bound>) -> Result<(), Option<Bound>>,
) -> Result<usize, PreparedFailure> {
    let Prepared {
        name,
        entry,
        argument,
        binding,
        clock,
    } = prepared;
    // Before looking for a slot: a spawn most needs a freed one, and the worker may lag.
    while crate::user::has_zombies() && crate::user::collect().is_reclaimed() {}

    let slot = match reserve(name, (entry, argument), clock) {
        Ok(slot) => slot,
        Err(reason) => return Err(PreparedFailure::before_binding(reason, binding)),
    };

    // Outside the leaf: `fabricate` takes `wost`'s own lock; a reserved slot cannot be chosen.
    if let Err(reason) = task::fabricate(slot, start, slot) {
        THREADS.lock_leaf().release(slot);
        return Err(PreparedFailure::before_binding(
            SpawnError::Task(reason),
            binding,
        ));
    }

    let bound = bind(slot, binding)?;
    if let Err(bound) = arrange(slot, bound) {
        retire_prepared(slot);
        return Err(PreparedFailure::after_retirement(
            SpawnError::Arrangement,
            bound,
        ));
    }

    mark_runnable(slot);
    Ok(slot)
}

fn reserve(
    name: &'static str,
    entry: (Entry, usize),
    clock: Option<&'static WorldClock>,
) -> Result<usize, SpawnError> {
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
            entity: Entity::new(BASE_WEIGHT),
            clock,
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

// While the slot is still `Reserved`, so its tables exist before its first instruction.
fn bind(slot: usize, binding: Option<Binding>) -> Result<Option<Bound>, PreparedFailure> {
    let Some(binding) = binding else {
        return Ok(None);
    };
    match task::bind_address_space(slot, binding) {
        Ok(bound) => Ok(Some(bound)),
        Err((reason, binding)) => {
            retire_prepared(slot);
            Err(PreparedFailure::before_binding(
                SpawnError::Task(reason),
                Some(binding),
            ))
        }
    }
}

// Arms though nothing switches: a second runnable thread makes the running slice matter.
fn mark_runnable(slot: usize) {
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

pub(crate) fn yield_now(context: &mut ThreadContext) {
    give_way(context, CurrentEligibility::Excluded);
}

// A ring 3 thread is preempted here, in its own handoff, rather than from the trap.
pub(crate) fn preempt(context: &mut ThreadContext) {
    give_way(context, CurrentEligibility::Eligible);
}

fn give_way(context: &mut ThreadContext, eligibility: CurrentEligibility) {
    // Interrupts off from decision to switch, or an expiry could act on a half-made decision.
    let saved = interrupt::disable();
    let now = time::scheduler_now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock(context);
        let current = task::current();
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        let next = match threads.queue.choose(current, eligibility) {
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
        && let Err(reason) = task::switch_to(next)
    {
        panic!("switch to slot {next} refused: {reason}");
    }
    saved.restore();
}

pub(crate) fn sleep_ms(context: &mut ThreadContext, milliseconds: u64) {
    if !time::is_calibrated() {
        return;
    }

    let mut saved = interrupt::disable();
    let wake_at = time::now_ns().saturating_add(milliseconds.saturating_mul(NANOSECONDS_PER_MS));
    // A loop: the halt below returns on any interrupt, and the deadline is what this promised.
    while time::is_calibrated() && time::now_ns() < wake_at {
        park(
            context,
            State::Sleeping,
            Some(wake_at),
            &mut saved,
            |threads, current| {
                threads.queue.slots[current].wake_at = wake_at;
                true
            },
        );
    }
    saved.restore();
}

// `ready` runs under `THREADS`, so it may take only leaf locks no waker holds while waking.
pub(crate) fn wait_on(
    context: &mut ThreadContext,
    head: &AtomicUsize,
    until: Option<u64>,
    ready: impl Fn() -> bool,
) {
    if until.is_some() && !time::is_calibrated() {
        return;
    }
    // Off from the check to the block, which makes them one step on one CPU.
    let mut saved = interrupt::disable();
    if until.is_some_and(|deadline| time::now_ns() >= deadline) {
        saved.restore();
        return;
    }
    let leaving = if until.is_some() {
        State::Sleeping
    } else {
        State::Blocked
    };
    let parked = park(context, leaving, until, &mut saved, |threads, current| {
        if ready() {
            return false;
        }
        threads.link_waiter(head, current);
        if let Some(deadline) = until {
            threads.queue.slots[current].wake_at = deadline;
        }
        true
    });
    // Woken or not: a wake already unlinked this thread, and a timeout or halt did not.
    if parked {
        THREADS.lock(context).unlink_waiter(head, task::current());
    }
    saved.restore();
}

// A linked thread that is no longer waiting is skipped: something else already woke it.
pub(crate) fn wake_waiters(head: &AtomicUsize) -> usize {
    let woken = {
        let mut threads = THREADS.lock_leaf();
        let mut woken = 0;
        let mut slot = head.swap(NO_WAITER, Ordering::Relaxed);
        while slot != NO_WAITER {
            let next = core::mem::replace(&mut threads.next_waiter[slot], NO_WAITER);
            if matches!(
                threads.queue.slots[slot].state,
                State::Blocked | State::Sleeping
            ) {
                threads.queue.ready(slot);
                woken += 1;
            }
            slot = next;
        }
        woken
    };

    // Armed always: a handler that woke it may return to ring 3 and strand the woken thread.
    if woken > 0 {
        reschedule_timer(Some(time::scheduler_now_ns()));
    }
    woken
}

// Called with interrupts off; they stay off until after the switch. `arrange` may refuse.
fn park(
    context: &mut ThreadContext,
    leaving: State,
    until: Option<u64>,
    saved: &mut interrupt::InterruptState,
    arrange: impl FnOnce(&mut Threads, usize) -> bool,
) -> bool {
    let now = time::scheduler_now_ns();
    let (next, event) = {
        let mut threads = THREADS.lock(context);
        let current = task::current();
        if !arrange(&mut threads, current) {
            return false;
        }
        threads.queue.charge(current, now);
        threads.queue.wake_due(now);
        threads.queue.park(current);
        threads.queue.slots[current].state = leaving;

        match threads.queue.choose(current, CurrentEligibility::Excluded) {
            Some(next) => {
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
        // Enabling and halting must be one step (`sti; hlt`), or the waking interrupt is lost.
        None if saved.were_enabled() => saved.wait_for_interrupt(),
        // A nested or disabled exclusion cannot halt; the caller re-checks as after any wake.
        None => {}
    }
    true
}

// A zombie: it stands on the stack and tables it would free, so it is orphaned before exiting.
pub(crate) fn retire() -> ! {
    let _off = interrupt::disable();
    crate::user::orphan(task::current());
    // After `orphan`, so an acquiring observer of the new count can read the last rites.
    #[cfg(feature = "selftest")]
    RETIREMENTS.fetch_add(1, Ordering::AcqRel);
    exit()
}

// Before every return to ring 3, so a program's page never lags its own last kernel entry.
pub(crate) fn refresh_clock() {
    let clock = THREADS.lock_leaf().queue.slots[task::current()].clock;
    if let Some(clock) = clock {
        clock.now_ns();
    }
}

fn earliest(one: Option<u64>, other: Option<u64>) -> Option<u64> {
    match (one, other) {
        (Some(one), Some(other)) => Some(one.min(other)),
        (only, None) | (None, only) => only,
    }
}

// `try_lock`: the caller is the panic handler, and the panicking code may hold this.
pub(crate) fn current_name() -> &'static str {
    let slot = task::current();
    match THREADS.try_lock_leaf() {
        Some(threads) if !threads.identities[slot].name.is_empty() => threads.identities[slot].name,
        Some(_) => "unnamed",
        None => "unknown",
    }
}

// A cleared interrupted flag prevents only the switch; a ring 3 frame switches in its handoff.
pub(crate) fn reschedule(frame: &TrapFrame) {
    let now = time::scheduler_now_ns();
    let switchable = frame.was_interruptible() && !frame.is_from_user();
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

fn reschedule_timer(event: Option<u64>) {
    match event {
        Some(deadline) => time::arm_at(deadline),
        None => time::disarm(),
    }
}

pub(crate) fn exit() -> ! {
    // Never restored: an expiry before the switch would find a thread neither running nor free.
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

fn start(context: &mut ThreadContext, slot: usize) -> ! {
    let entry = THREADS.lock_leaf().identities[slot].entry;
    match entry {
        Some((entry, argument)) => entry(context, argument),
        None => panic!("slot {slot} started with no entry"),
    }
    exit()
}
