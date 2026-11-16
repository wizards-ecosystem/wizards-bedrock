use core::fmt;
use core::ops::Deref;
use core::sync::atomic::{AtomicU64, Ordering};

use boot_record::{Ran, Record};
use eevdf::{BASE_WEIGHT, Entity};
use stack_layout::StackLayout;
use wake_state::{Event, State as Wake, Word};
use world_tree::NodeRef;
use wost::arch::current::interrupt::{self, InterruptState, TrapFrame};
use wost::arch::current::paging::{Binding, MapError};
use wost::arch::current::task::{self, Bound, Claimed, Released, STACK_FRAMES, Task, TaskError};
use wost::arch::current::{cpu, percpu};
use wost::mm::pool::{self as frames, ArenaFrame, Earmark, FrameKind, Refused};
use wost::mm::slab::{Hold, Refusal, Removal};
pub(crate) use wost::sync::ThreadContext;
use wost::sync::{Leaf, SpinLock};

use crate::arena::Local;
use crate::process::ProcessRef;
use crate::processors::MAX_PROCESSORS;
use crate::time::{self, NANOSECONDS_PER_MS};
use crate::timepage::WorldClock;
use crate::world::{self, ThreadPool, World, registry};
use crate::{mem, rank};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::{LIVE, RETIREMENTS};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{live, retirements};

// The boot thread builds processes, and the debug construction path exceeded 32 KiB.
const BOOT_STACK_BYTES: u64 = 64 * 1024;

const GUARD_BYTES: u64 = 4096;

// Below the device window, far above the direct map, so a stack address is never either.
const WINDOW: u64 = 0xffff_fe00_0000_0000;

// Past the boot and processor stacks, which start at `WINDOW`.
const TASK_WINDOW: u64 = WINDOW + (1 << 30);

// In the range mainline uses, and the one tunable EEVDF has.
const SLICE_NS: u64 = 3 * NANOSECONDS_PER_MS;

pub(crate) fn boot_stack_layout() -> Option<StackLayout> {
    StackLayout::new(WINDOW, BOOT_STACK_BYTES, GUARD_BYTES, 1)
}

pub(crate) const fn task_window() -> u64 {
    TASK_WINDOW
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SpawnError {
    Unavailable,
    Memory(Refused),
    Stack(MapError),
    Task(TaskError),
    Arrangement,
}

impl fmt::Display for SpawnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unavailable => formatter.write_str("threads are unavailable"),
            Self::Memory(reason) => write!(formatter, "no frames for a thread: {reason}"),
            Self::Stack(reason) => write!(formatter, "no kernel stack: {reason}"),
            Self::Task(reason) => write!(formatter, "{reason}"),
            Self::Arrangement => {
                formatter.write_str("the thread's launch state could not be arranged")
            }
        }
    }
}

impl core::error::Error for SpawnError {}

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

pub(crate) type Entry = fn(&mut ThreadContext, usize);

// Names a thread by its world and its place in that world's arena.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ThreadRef {
    world: NodeRef,
    thread: Local<Thread>,
}

impl ThreadRef {
    fn tag(self) -> [u64; 2] {
        [
            u64::from(self.world.node) << u32::BITS | u64::from(self.world.generation),
            self.thread.to_bits(),
        ]
    }

    fn from_tag(tag: [u64; 2]) -> Self {
        Self {
            world: NodeRef {
                node: (tag[0] >> u32::BITS) as u32,
                generation: tag[0] as u32,
            },
            thread: Local::from_bits(tag[1]),
        }
    }

    pub(crate) fn id(self) -> u64 {
        self.thread.to_bits()
    }

    fn hold(self) -> Option<Resolved> {
        let world = registry::hold(self.world)?;
        let thread = world.threads.hold(self.thread)?;
        Some(Resolved {
            thread,
            _world: world,
        })
    }
}

// The thread first, so its borrow ends before the world's.
struct Resolved {
    thread: Hold<Thread>,
    _world: Hold<World>,
}

impl Deref for Resolved {
    type Target = Thread;

    fn deref(&self) -> &Thread {
        &self.thread
    }
}

fn resolved(thread: ThreadRef) -> Resolved {
    thread
        .hold()
        .unwrap_or_else(|| panic!("a scheduled thread's record is gone"))
}

#[derive(Clone, Copy, Debug)]
struct Wait {
    sequence: u64,
    deadline: Option<u64>,
}

// Each link changes only under the lock of the list it is in: a run queue, a wait list, a run
// queue's sleepers or the reaper's list.
#[derive(Debug)]
struct Sched {
    entity: Entity,
    started: u64,
    lag: i64,
    cpu: usize,
    next: Option<ThreadRef>,
    wait: Option<Wait>,
    listed: bool,
    wait_next: Option<ThreadRef>,
    sleeping_on: Option<usize>,
    sleeper_next: Option<ThreadRef>,
    reap_next: Option<ThreadRef>,
}

impl Sched {
    const fn new(cpu: usize) -> Self {
        Self {
            entity: Entity::new(BASE_WEIGHT),
            started: 0,
            lag: 0,
            cpu,
            next: None,
            wait: None,
            listed: false,
            wait_next: None,
            sleeping_on: None,
            sleeper_next: None,
            reap_next: None,
        }
    }
}

// A thread of its world: its record, switch state and kernel stack all lie in that world's arena.
#[derive(Debug)]
pub(crate) struct Thread {
    task: Task,
    name: &'static str,
    entry: Option<(Entry, usize)>,
    owner: Option<ProcessRef>,
    // The world whose time page this thread reads while it runs in ring 3.
    clock: Option<&'static WorldClock>,
    wake: AtomicU64,
    sched: SpinLock<Leaf, Sched>,
}

fn sched<R>(thread: &Thread, change: impl FnOnce(&mut Sched) -> R) -> R {
    change(&mut thread.sched.lock_leaf())
}

fn sched_of<R>(thread: ThreadRef, change: impl FnOnce(&mut Sched) -> R) -> R {
    sched(&resolved(thread), change)
}

fn advance(thread: &Thread, event: Event) -> Option<Word> {
    let mut word = Word::from_bits(thread.wake.load(Ordering::Acquire));
    loop {
        let next = wake_state::step(word, event)?;
        match thread.wake.compare_exchange(
            word.bits(),
            next.bits(),
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(next),
            Err(seen) => word = Word::from_bits(seen),
        }
    }
}

// Succeeds only on the wait `sequence` names, so a stale link never ends a later wait.
fn claim(thread: &Thread, sequence: u64) -> Option<Wake> {
    advance(thread, Event::Claim { sequence }).and_then(Word::state)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    // The running thread stays a candidate and counts in the average.
    Preempt,
    // It counts in the average but is not a candidate.
    Yield,
    // It is leaving the processor and counts in nothing.
    Leave,
}

#[derive(Debug)]
struct RunQueue {
    first: Option<ThreadRef>,
    last: Option<ThreadRef>,
    ready: usize,
    // Threads blocked on this processor with a deadline, which its alarm ends.
    sleepers: Option<ThreadRef>,
    current: Option<ThreadRef>,
    // Never queued: any weight, however small, would win it the processor now and then.
    idle: Option<ThreadRef>,
}

impl RunQueue {
    const EMPTY: Self = Self {
        first: None,
        last: None,
        ready: 0,
        sleepers: None,
        current: None,
        idle: None,
    };

    fn push(&mut self, thread: ThreadRef) {
        sched_of(thread, |sched| sched.next = None);
        match self.last {
            Some(last) => sched_of(last, |sched| sched.next = Some(thread)),
            None => self.first = Some(thread),
        }
        self.last = Some(thread);
        self.ready += 1;
    }

    fn remove(&mut self, thread: ThreadRef) {
        let mut previous = None;
        let mut cursor = self.first;
        while let Some(at) = cursor {
            let next = sched_of(at, |sched| sched.next);
            if at == thread {
                match previous {
                    Some(previous) => sched_of(previous, |sched| sched.next = next),
                    None => self.first = next,
                }
                if self.last == Some(thread) {
                    self.last = previous;
                }
                sched_of(thread, |sched| sched.next = None);
                self.ready -= 1;
                return;
            }
            previous = cursor;
            cursor = next;
        }
    }

    fn entities(&self) -> Entities {
        Entities { cursor: self.first }
    }

    fn normal_current(&self) -> Option<ThreadRef> {
        self.current.filter(|current| Some(*current) != self.idle)
    }

    fn average(&self, with_current: bool) -> Option<u64> {
        let current = with_current
            .then(|| self.normal_current())
            .flatten()
            .map(|current| sched_of(current, |sched| sched.entity));
        eevdf::average_vruntime(self.entities().map(|(_, entity)| entity).chain(current))
    }

    fn choose(&self, mode: Mode) -> Option<ThreadRef> {
        // The idle thread answers only when there is no other, never replacing a normal thread.
        let Some(average) = self.average(mode != Mode::Leave) else {
            return self.idle.filter(|idle| Some(*idle) != self.current);
        };
        let current = (mode == Mode::Preempt)
            .then(|| self.normal_current())
            .flatten();
        let candidates = || {
            self.entities()
                .chain(current.map(|current| (current, sched_of(current, |sched| sched.entity))))
        };
        match eevdf::pick(candidates(), average) {
            Some(thread) => Some(thread),
            // With the running thread excluded, nobody eligible is honest: it keeps the CPU.
            None if mode == Mode::Yield => None,
            // With everyone considered `eevdf` always finds someone; first in line is the backstop.
            None => candidates().next().map(|(thread, _)| thread),
        }
    }

    fn next_event(&self, running: ThreadRef, now: u64) -> Option<u64> {
        let runnable = self.ready + usize::from(Some(running) != self.idle);
        let held = resolved(running);
        let remaining = sched(&held, |sched| sched.entity.remaining_slice());
        let slice_end = (runnable > 1).then(|| now.saturating_add(remaining));
        // Only while a thread with a world clock runs, so an idle kernel still takes no alarm.
        let clock_refresh = held
            .clock
            .map(|clock| now.saturating_add(clock.refresh_interval_ns()));
        drop(held);
        let mut earliest_sleeper = None;
        let mut cursor = self.sleepers;
        while let Some(at) = cursor {
            let (deadline, next) = sched_of(at, |sched| {
                (
                    sched.wait.and_then(|wait| wait.deadline),
                    sched.sleeper_next,
                )
            });
            earliest_sleeper = earliest(earliest_sleeper, deadline);
            cursor = next;
        }
        earliest(earliest(slice_end, earliest_sleeper), clock_refresh)
    }

    // The arriving thread's processor is this one, and it may not yet be off its last one.
    fn place(&mut self, thread: ThreadRef, held: &Thread) {
        let average = self.average(true);
        sched(held, |sched| {
            let average = average.unwrap_or(sched.entity.vruntime());
            sched.entity.place(average, sched.lag, SLICE_NS);
        });
        self.push(thread);
    }

    // A revoked clock wakes every sleeper: none may stay parked on an unreachable deadline.
    fn wake_due(&mut self, now: u64) {
        let clock_live = time::is_calibrated();
        let mut previous = None;
        let mut cursor = self.sleepers;
        while let Some(at) = cursor {
            let held = resolved(at);
            let (next, wait) = sched(&held, |sched| (sched.sleeper_next, sched.wait));
            let deadline = wait.and_then(|wait| wait.deadline);
            if clock_live && deadline.is_some_and(|deadline| deadline > now) {
                previous = cursor;
                cursor = next;
                continue;
            }
            match previous {
                Some(previous) => sched_of(previous, |sched| sched.sleeper_next = next),
                None => self.sleepers = next,
            }
            sched(&held, |sched| {
                sched.sleeper_next = None;
                sched.sleeping_on = None;
            });
            // It blocked on this processor, whose switch away finished before this lock returned.
            if let Some(wait) = wait
                && claim(&held, wait.sequence) == Some(Wake::Waking)
            {
                self.place(at, &held);
                assert!(
                    advance(&held, Event::Place).is_some(),
                    "a claimed sleeper could not be placed"
                );
            }
            cursor = next;
        }
    }

    fn unlink_sleeper(&mut self, cpu: usize, thread: ThreadRef) {
        if sched_of(thread, |sched| sched.sleeping_on) != Some(cpu) {
            return;
        }
        let mut previous = None;
        let mut cursor = self.sleepers;
        while let Some(at) = cursor {
            let next = sched_of(at, |sched| sched.sleeper_next);
            if at == thread {
                match previous {
                    Some(previous) => sched_of(previous, |sched| sched.sleeper_next = next),
                    None => self.sleepers = next,
                }
                sched_of(thread, |sched| {
                    sched.sleeper_next = None;
                    sched.sleeping_on = None;
                });
                return;
            }
            previous = cursor;
            cursor = next;
        }
    }

    // The first ready thread off every processor leaves, carrying its lag from this queue's average.
    fn take_off_cpu(&mut self, thief: usize) -> Option<ThreadRef> {
        let average = self.average(true);
        let mut cursor = self.first;
        while let Some(at) = cursor {
            let held = resolved(at);
            let next = sched(&held, |sched| sched.next);
            if !held.task.is_on_cpu() {
                drop(held);
                self.remove(at);
                sched_of(at, |sched| {
                    sched.lag = sched.entity.lag(average.unwrap_or(sched.entity.vruntime()));
                    sched.cpu = thief;
                });
                return Some(at);
            }
            cursor = next;
        }
        None
    }

    // `next` leaves the queue and is claimed, so no other processor can pick it or free it.
    fn switch_in(&mut self, cpu: usize, next: ThreadRef, now: u64) -> Claimed {
        self.remove(next);
        self.current = Some(next);
        let held = resolved(next);
        sched(&held, |sched| {
            sched.started = now;
            sched.cpu = cpu;
        });
        *NAMES[cpu].lock_leaf() = held.name;
        let bit = 1 << cpu;
        if Some(next) == self.idle {
            IDLE.fetch_or(bit, Ordering::AcqRel);
        } else {
            IDLE.fetch_and(!bit, Ordering::AcqRel);
            RAN.fetch_or(bit, Ordering::Relaxed);
        }
        held.task
            .claim()
            .unwrap_or_else(|| panic!("thread {} was picked while it is on a processor", held.name))
    }
}

struct Entities {
    cursor: Option<ThreadRef>,
}

impl Iterator for Entities {
    type Item = (ThreadRef, Entity);

    fn next(&mut self) -> Option<Self::Item> {
        let at = self.cursor?;
        let (entity, next) = sched_of(at, |sched| (sched.entity, sched.next));
        self.cursor = next;
        Some((at, entity))
    }
}

// Every switch happens after its guard drops, with interrupts off throughout.
static QUEUES: [SpinLock<rank::RunQueue, RunQueue>; MAX_PROCESSORS] =
    [const { SpinLock::new(RunQueue::EMPTY) }; MAX_PROCESSORS];

// What each processor runs, for a panic report that must not resolve a record.
static NAMES: [SpinLock<Leaf, &'static str>; MAX_PROCESSORS] =
    [const { SpinLock::new("") }; MAX_PROCESSORS];

// Processors whose idle thread runs, those that schedule at all, and those that ran another.
static IDLE: AtomicU64 = AtomicU64::new(0);
static SCHEDULING: AtomicU64 = AtomicU64::new(0);
static RAN: AtomicU64 = AtomicU64::new(0);

fn this_processor() -> usize {
    percpu::processor()
}

// Its last processor if idle, else an idle one, else its last; only processors that schedule.
fn target_for(previous: usize) -> usize {
    let scheduling = SCHEDULING.load(Ordering::Acquire);
    let idle = IDLE.load(Ordering::Acquire);
    wake_state::wake_target(previous as u32, idle & scheduling, scheduling)
        .map_or(previous, |cpu| cpu as usize)
}

// A busy processor may have no alarm armed, so any other target hears of its new thread.
fn notify(target: usize) {
    if target != this_processor() {
        percpu::send_reschedule(target);
    }
}

static REPORTED: AtomicU64 = AtomicU64::new(0);

// What syscalls and switches cannot log, reported by the service worker.
pub(crate) fn reports_pending() -> bool {
    crate::user::reports_pending()
        || RAN.load(Ordering::Acquire) != REPORTED.load(Ordering::Acquire)
}

pub(crate) fn report() {
    crate::user::report();
    report_processors();
}

pub(crate) fn report_processors() {
    let ran = RAN.load(Ordering::Acquire);
    if REPORTED.swap(ran, Ordering::AcqRel) == ran {
        return;
    }
    let processors = ran.count_ones() as usize;
    crate::info!("threads: threads ran on {processors} processors");
    crate::info!("{}", Record::Ran(Ran { processors }));
}

fn charge(thread: ThreadRef, now: u64) {
    sched_of(thread, |sched| {
        sched.entity.charge(now.saturating_sub(sched.started));
        sched.started = now;
        if sched.entity.slice_is_spent() {
            sched.entity.renew_deadline(SLICE_NS);
        }
    });
}

#[derive(Debug)]
pub(crate) struct WaitLinks {
    first: Option<ThreadRef>,
    last: Option<ThreadRef>,
}

impl WaitLinks {
    pub(crate) const fn new() -> Self {
        Self {
            first: None,
            last: None,
        }
    }

    fn push(&mut self, thread: ThreadRef) {
        sched_of(thread, |sched| {
            sched.wait_next = None;
            sched.listed = true;
        });
        match self.last {
            Some(last) => sched_of(last, |sched| sched.wait_next = Some(thread)),
            None => self.first = Some(thread),
        }
        self.last = Some(thread);
    }

    fn pop(&mut self) -> Option<ThreadRef> {
        let first = self.first?;
        let next = sched_of(first, |sched| {
            sched.listed = false;
            sched.wait_next.take()
        });
        self.first = next;
        if next.is_none() {
            self.last = None;
        }
        Some(first)
    }

    fn unlink(&mut self, thread: ThreadRef) {
        if !sched_of(thread, |sched| sched.listed) {
            return;
        }
        let mut previous = None;
        let mut cursor = self.first;
        while let Some(at) = cursor {
            let next = sched_of(at, |sched| sched.wait_next);
            if at == thread {
                match previous {
                    Some(previous) => sched_of(previous, |sched| sched.wait_next = next),
                    None => self.first = next,
                }
                if self.last == Some(thread) {
                    self.last = previous;
                }
                sched_of(thread, |sched| {
                    sched.wait_next = None;
                    sched.listed = false;
                });
                return;
            }
            previous = cursor;
            cursor = next;
        }
    }
}

pub(crate) type WaitList = SpinLock<rank::Wait, WaitLinks>;

pub(crate) fn current() -> Option<ThreadRef> {
    task::current_tag().map(ThreadRef::from_tag)
}

pub(crate) fn current_id() -> u64 {
    current().map_or(0, ThreadRef::id)
}

pub(crate) fn current_process() -> Option<ProcessRef> {
    current()?.hold()?.owner
}

pub(crate) fn is_available() -> bool {
    QUEUES[0].lock_leaf().current.is_some()
}

#[derive(Debug)]
struct Prepared {
    name: &'static str,
    entry: (Entry, usize),
    owner: Option<ProcessRef>,
    clock: Option<&'static WorldClock>,
}

fn take(earmark: &mut Earmark) -> ArenaFrame {
    earmark
        .take(FrameKind::ArenaObject)
        .unwrap_or_else(|| panic!("a thread outran the frames it earmarked"))
}

// Every frame the thread needs is earmarked first, so nothing after can run out.
fn create(
    context: &mut ThreadContext,
    world_ref: NodeRef,
    prepared: Prepared,
) -> Result<ThreadRef, SpawnError> {
    let world = registry::hold(world_ref).ok_or(SpawnError::Unavailable)?;
    let records = ThreadPool::frames_for(1).ok_or(SpawnError::Memory(Refused::NoMemory))?;
    let wanted = records + 1 + STACK_FRAMES as u32;
    let mut earmark =
        frames::earmark(context, world.arena(), wanted).map_err(SpawnError::Memory)?;
    let record = take(&mut earmark);
    let stack = core::array::from_fn(|_| take(&mut earmark));
    // Before the space lock, which clears the flag: the creator's own flag is wanted.
    let interruptible = interrupt::are_enabled();
    let task =
        mem::map_task(context, record, stack, start, interruptible).map_err(SpawnError::Stack)?;
    let thread = world.threads.insert(
        &mut earmark,
        Thread {
            task,
            name: prepared.name,
            entry: Some(prepared.entry),
            owner: prepared.owner,
            clock: prepared.clock,
            wake: AtomicU64::new(Word::default().bits()),
            sched: SpinLock::new(Sched::new(this_processor())),
        },
    );
    let thread = ThreadRef {
        world: world_ref,
        thread,
    };
    resolved(thread).task.name(thread.tag());
    #[cfg(feature = "selftest")]
    LIVE.fetch_add(1, Ordering::AcqRel);
    Ok(thread)
}

// The boot thread becomes a thread of the root world, so the scheduler can name it.
pub(crate) fn adopt_boot(context: &mut ThreadContext) {
    let Some(root) = world::root() else {
        crate::warn!("threads: no root world; the boot thread runs alone");
        return;
    };
    let Some(task) = Task::boot() else {
        return;
    };
    let Some(world) = registry::hold(root) else {
        panic!("the root world did not resolve");
    };
    let mut earmark = match ThreadPool::earmark(context, world.arena(), 1) {
        Ok(earmark) => earmark,
        Err(reason) => {
            crate::warn!("threads: no record for the boot thread: {reason}; it runs alone");
            return;
        }
    };
    let mut first = Sched::new(0);
    first.entity.place(0, 0, SLICE_NS);
    first.started = time::scheduler_now_ns();
    let thread = world.threads.insert(
        &mut earmark,
        Thread {
            task,
            name: "boot",
            entry: None,
            owner: None,
            clock: None,
            wake: AtomicU64::new(Word::default().bits()),
            sched: SpinLock::new(first),
        },
    );
    drop(world);
    let thread = ThreadRef {
        world: root,
        thread,
    };
    resolved(thread).task.name(thread.tag());
    *NAMES[0].lock_leaf() = "boot";
    QUEUES[0].lock(context).current = Some(thread);
    SCHEDULING.fetch_or(1, Ordering::AcqRel);
    RAN.fetch_or(1, Ordering::Relaxed);
}

// One per online processor; each other processor then leaves its park for its idle thread.
pub(crate) fn start_idle(context: &mut ThreadContext) {
    if !is_available() {
        return;
    }
    let Some(root) = world::root() else {
        return;
    };
    let online = percpu::online_set() | 1 << this_processor();
    for cpu in (0..MAX_PROCESSORS).filter(|cpu| online & (1 << cpu) != 0) {
        if QUEUES[cpu].lock(context).idle.is_some() {
            continue;
        }
        let prepared = Prepared {
            name: "idle",
            entry: (idle_loop, 0),
            owner: None,
            clock: None,
        };
        match create(context, root, prepared) {
            Ok(idle) => {
                QUEUES[cpu].lock(context).idle = Some(idle);
                notify(cpu);
            }
            Err(reason) => crate::warn!("threads: no idle thread for processor {cpu}: {reason}"),
        }
    }
}

// An online processor's bring-up context, interrupts on, until its idle thread exists.
pub(crate) fn run_processor(cpu: usize) -> ! {
    loop {
        let mut saved = interrupt::disable();
        let claimed = {
            let mut queue = QUEUES[cpu].lock_ranked();
            queue
                .idle
                .map(|idle| queue.switch_in(cpu, idle, time::scheduler_now_ns()))
        };
        if let Some(claimed) = claimed {
            SCHEDULING.fetch_or(1 << cpu, Ordering::AcqRel);
            task::enter(claimed, saved);
        }
        // Enabling and halting are one step, so the reschedule that announces the thread is kept.
        saved.wait_for_interrupt();
        saved.restore();
    }
}

// Holds one run-queue lock at a time: the source's to take a thread, then its own to place it.
fn steal(context: &mut ThreadContext) -> bool {
    let me = this_processor();
    if QUEUES[me].lock(context).ready > 0 {
        return true;
    }
    let scheduling = SCHEDULING.load(Ordering::Acquire);
    let mut ready = [0; MAX_PROCESSORS];
    for (cpu, count) in ready.iter_mut().enumerate() {
        if scheduling & (1 << cpu) != 0 {
            *count = QUEUES[cpu].lock(context).ready as u32;
        }
    }
    let Some(source) = wake_state::steal_source(&ready, me) else {
        return false;
    };
    let Some(taken) = QUEUES[source].lock(context).take_off_cpu(me) else {
        return false;
    };
    let mut queue = QUEUES[me].lock(context);
    queue.place(taken, &resolved(taken));
    true
}

// It runs only when nothing else can, so it never logs: a thread lock it held could wait forever.
fn idle_loop(context: &mut ThreadContext, _: usize) {
    loop {
        if reports_pending() {
            crate::queue::request_reports();
        }
        if steal(context) {
            yield_now(context);
            continue;
        }
        cpu::halt();
    }
}

pub(crate) fn spawn(
    context: &mut ThreadContext,
    name: &'static str,
    entry: Entry,
    argument: usize,
) -> Result<ThreadRef, SpawnError> {
    let root = world::root().ok_or(SpawnError::Unavailable)?;
    if !is_available() {
        return Err(SpawnError::Unavailable);
    }
    let thread = create(
        context,
        root,
        Prepared {
            name,
            entry: (entry, argument),
            owner: None,
            clock: None,
        },
    )?;
    mark_runnable(thread);
    Ok(thread)
}

// `arrange` must store the bound token with the process state before the thread can run.
pub(crate) fn spawn_in_space(
    context: &mut ThreadContext,
    name: &'static str,
    entry: Entry,
    binding: Binding,
    clock: &'static WorldClock,
    owner: ProcessRef,
    arrange: impl FnOnce(ThreadRef, Bound) -> Result<(), Bound>,
) -> Result<ThreadRef, SpaceSpawnError> {
    let refused = |reason, binding| SpaceSpawnError {
        reason,
        authority: SpaceAuthority::Released(task::release_uninstalled(binding)),
    };
    if !is_available() {
        return Err(refused(SpawnError::Unavailable, binding));
    }
    let prepared = Prepared {
        name,
        entry: (entry, 0),
        owner: Some(owner),
        clock: Some(clock),
    };
    let thread = match create(context, owner.world(), prepared) {
        Ok(thread) => thread,
        Err(reason) => return Err(refused(reason, binding)),
    };
    let bound = task::bind_address_space(&resolved(thread).task, binding);
    let bound = match bound {
        Ok(bound) => bound,
        Err((reason, binding)) => {
            discard(context, thread);
            return Err(refused(SpawnError::Task(reason), binding));
        }
    };
    if let Err(bound) = arrange(thread, bound) {
        retire_unstarted(thread);
        let Ok(released) = task::release_address_space(bound) else {
            panic!("a retired task kept its address-space binding");
        };
        discard(context, thread);
        return Err(SpaceSpawnError {
            reason: SpawnError::Arrangement,
            authority: SpaceAuthority::Released(released),
        });
    }
    mark_runnable(thread);
    Ok(thread)
}

// Every refusal `task::retire` can give repeats on a retry, so the first one is terminal.
fn retire_unstarted(thread: ThreadRef) {
    if let Err(reason) = task::retire(&resolved(thread).task) {
        panic!("an unstarted thread's retirement was refused: {reason}");
    }
}

fn discard(context: &mut ThreadContext, thread: ThreadRef) {
    let _ = task::retire(&resolved(thread).task);
    assert!(
        release(context, thread),
        "a thread that never ran was borrowed"
    );
}

// Arms though nothing switches: a second runnable thread makes the running slice matter.
fn mark_runnable(thread: ThreadRef) {
    let saved = interrupt::disable();
    let now = time::scheduler_now_ns();
    let cpu = target_for(this_processor());
    let event = {
        let mut queue = QUEUES[cpu].lock_ranked();
        let average = queue.average(true).unwrap_or(0);
        sched_of(thread, |sched| {
            sched.entity.place(average, 0, SLICE_NS);
            sched.cpu = cpu;
        });
        queue.push(thread);
        queue.current.map(|current| queue.next_event(current, now))
    };
    if cpu == this_processor() {
        if let Some(event) = event {
            reschedule_timer(event);
        }
    } else {
        notify(cpu);
    }
    saved.restore();
}

pub(crate) fn yield_now(context: &mut ThreadContext) {
    give_way(context, Mode::Yield);
}

// A ring 3 thread is preempted here, in its own handoff, rather than from the trap.
pub(crate) fn preempt(context: &mut ThreadContext) {
    give_way(context, Mode::Preempt);
}

fn give_way(context: &mut ThreadContext, mode: Mode) {
    // Interrupts off from decision to switch, or an expiry could act on a half-made decision.
    let saved = interrupt::disable();
    let now = time::scheduler_now_ns();
    let cpu = this_processor();
    let (next, event) = {
        let mut queue = QUEUES[cpu].lock(context);
        let Some(current) = queue.current else {
            drop(queue);
            saved.restore();
            return;
        };
        charge(current, now);
        queue.wake_due(now);
        let next = queue
            .choose(mode)
            .filter(|next| *next != current)
            .map(|next| {
                if Some(current) != queue.idle {
                    queue.push(current);
                }
                queue.switch_in(cpu, next, now)
            });
        let running = queue.current.unwrap_or(current);
        (next, queue.next_event(running, now))
    };

    reschedule_timer(event);
    if let Some(next) = next
        && let Err(reason) = task::switch_to(next)
    {
        panic!("a switch was refused: {reason}");
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
        let Some(me) = current() else {
            reschedule_timer(Some(wake_at));
            if saved.were_enabled() {
                saved.wait_for_interrupt();
            }
            continue;
        };
        let sequence = register(me, Some(wake_at));
        block(context, me, sequence, Some(wake_at), &mut saved);
        finish_wait(context, me, None);
    }
    saved.restore();
}

fn register(me: ThreadRef, deadline: Option<u64>) -> u64 {
    let held = resolved(me);
    let Some(word) = advance(&held, Event::Park) else {
        panic!("thread {} began a wait while not running", held.name);
    };
    let sequence = word.sequence();
    sched(&held, |sched| {
        sched.wait = Some(Wait { sequence, deadline });
    });
    sequence
}

// `ready` runs under the wait list's lock, so it may take only leaf locks no waker holds.
pub(crate) fn wait_on(
    context: &mut ThreadContext,
    list: &WaitList,
    until: Option<u64>,
    ready: impl Fn() -> bool,
) {
    if until.is_some() && !time::is_calibrated() {
        return;
    }
    let mut saved = interrupt::disable();
    if until.is_some_and(|deadline| time::now_ns() >= deadline) {
        saved.restore();
        return;
    }
    // The boot thread before its record: only an interrupt can change what it waits for.
    let Some(me) = current() else {
        if !ready() && saved.were_enabled() {
            saved.wait_for_interrupt();
        }
        saved.restore();
        return;
    };
    let sequence = {
        let mut links = list.lock(context);
        if ready() {
            drop(links);
            saved.restore();
            return;
        }
        let sequence = register(me, until);
        links.push(me);
        sequence
    };
    block(context, me, sequence, until, &mut saved);
    finish_wait(context, me, Some(list));
    saved.restore();
}

// Interrupts are off and the word is at Parking; it returns once the wait has ended.
fn block(
    context: &mut ThreadContext,
    me: ThreadRef,
    sequence: u64,
    until: Option<u64>,
    saved: &mut InterruptState,
) {
    let now = time::scheduler_now_ns();
    let cpu = this_processor();
    let mut queue = QUEUES[cpu].lock(context);
    charge(me, now);
    queue.wake_due(now);
    let average = queue.average(true);
    let Some(next) = queue.choose(Mode::Leave) else {
        // A sole runnable thread remains charged while it waits, because it still owns the CPU.
        let event = queue.next_event(me, now);
        drop(queue);
        reschedule_timer(earliest(event, until));
        // Enabling and halting must be one step (`sti; hlt`), or the waking interrupt is lost.
        if saved.were_enabled() {
            saved.wait_for_interrupt();
        }
        let held = resolved(me);
        let _ = claim(&held, sequence);
        assert!(
            advance(&held, Event::Resume).is_some(),
            "a thread that could not block was left unwoken"
        );
        return;
    };
    let held = resolved(me);
    if advance(&held, Event::Block).is_none() {
        // A waker claimed the wait after it registered: it is over before it blocked.
        assert!(
            advance(&held, Event::Resume).is_some(),
            "a woken thread could not resume"
        );
        drop(held);
        let event = queue.next_event(me, now);
        drop(queue);
        reschedule_timer(event);
        return;
    }
    let sleepers = queue.sleepers;
    sched(&held, |sched| {
        sched.lag = sched.entity.lag(average.unwrap_or(0));
        if until.is_some() {
            sched.sleeper_next = sleepers;
            sched.sleeping_on = Some(cpu);
        }
    });
    if until.is_some() {
        queue.sleepers = Some(me);
    }
    drop(held);
    let claimed = queue.switch_in(cpu, next, now);
    let event = queue.next_event(next, now);
    drop(queue);
    reschedule_timer(event);
    if let Err(reason) = task::switch_to(claimed) {
        panic!("a blocking switch was refused: {reason}");
    }
}

// Woken or not, the thread leaves every list its wait put it on.
fn finish_wait(context: &mut ThreadContext, me: ThreadRef, list: Option<&WaitList>) {
    if let Some(cpu) = sched_of(me, |sched| sched.sleeping_on) {
        QUEUES[cpu].lock(context).unlink_sleeper(cpu, me);
    }
    if let Some(list) = list {
        list.lock(context).unlink(me);
    }
    sched_of(me, |sched| sched.wait = None);
}

// A linked thread whose wait already ended is skipped: its sequence no longer matches.
pub(crate) fn wake_waiters(list: &WaitList) -> usize {
    let mut woken = 0;
    {
        let mut links = list.lock_ranked();
        while let Some(waiter) = links.pop() {
            let held = resolved(waiter);
            let Some(wait) = sched(&held, |sched| sched.wait) else {
                continue;
            };
            match claim(&held, wait.sequence) {
                Some(Wake::Woken) => woken += 1,
                Some(Wake::Waking) => {
                    place_woken(waiter, &held);
                    woken += 1;
                }
                _ => {}
            }
        }
    }

    // Armed always: a handler that woke it may return to ring 3 and strand the woken thread.
    if woken > 0 {
        reschedule_timer(Some(time::scheduler_now_ns()));
    }
    woken
}

// Its stack and record stay its last processor's until the thread after it there lets them go.
fn place_woken(thread: ThreadRef, held: &Thread) {
    while held.task.is_on_cpu() {
        core::hint::spin_loop();
    }
    let cpu = target_for(sched(held, |sched| sched.cpu));
    sched(held, |sched| sched.cpu = cpu);
    let mut queue = QUEUES[cpu].lock_ranked();
    queue.place(thread, held);
    assert!(
        advance(held, Event::Place).is_some(),
        "a claimed thread could not be placed"
    );
    drop(queue);
    notify(cpu);
}

// It stands on the stack and tables its teardown frees, so the reaper does that work.
pub(crate) fn retire(context: &mut ThreadContext) -> ! {
    #[cfg(feature = "selftest")]
    RETIREMENTS.fetch_add(1, Ordering::AcqRel);
    exit(context)
}

// Before every return to ring 3, so a program's page never lags its own last kernel entry.
pub(crate) fn refresh_clock() {
    let clock = current()
        .and_then(ThreadRef::hold)
        .and_then(|held| held.clock);
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
    match NAMES[this_processor()].try_lock_leaf() {
        Some(name) if !name.is_empty() => *name,
        Some(_) => "unnamed",
        None => "unknown",
    }
}

// A cleared interrupted flag prevents only the switch; a ring 3 frame switches in its handoff.
pub(crate) fn reschedule(frame: &TrapFrame) {
    let now = time::scheduler_now_ns();
    let switchable = frame.was_interruptible() && !frame.is_from_user();
    let cpu = this_processor();
    let (next, event) = {
        let mut queue = QUEUES[cpu].lock_ranked();
        let Some(current) = queue.current else {
            return;
        };
        charge(current, now);
        queue.wake_due(now);
        let next = switchable
            .then(|| queue.choose(Mode::Preempt))
            .flatten()
            .filter(|next| *next != current)
            .map(|next| {
                if Some(current) != queue.idle {
                    queue.push(current);
                }
                queue.switch_in(cpu, next, now)
            });
        let running = queue.current.unwrap_or(current);
        (next, queue.next_event(running, now))
    };

    reschedule_timer(event);
    if let Some(next) = next
        && let Err(reason) = task::switch_from_trap(next, frame)
    {
        panic!("a preemption was refused: {reason}");
    }
}

fn reschedule_timer(event: Option<u64>) {
    match event {
        Some(deadline) => time::arm_at(deadline),
        None => time::disarm(),
    }
}

pub(crate) fn exit(context: &mut ThreadContext) -> ! {
    // Never restored: an expiry before the switch would find a thread neither running nor free.
    let _off = interrupt::disable();
    let Some(me) = current() else {
        panic!("a thread with no record cannot exit");
    };
    assert!(resolved(me).entry.is_some(), "the boot thread cannot exit");
    crate::reaper::submit(context, me);
    let now = time::scheduler_now_ns();
    let cpu = this_processor();
    let mut queue = QUEUES[cpu].lock(context);
    // Charged before it leaves, so the last slice is accounted like every other.
    charge(me, now);
    queue.wake_due(now);
    assert!(
        advance(&resolved(me), Event::Exit).is_some(),
        "an exiting thread was not running"
    );
    let Some(next) = queue.choose(Mode::Leave) else {
        panic!("nothing is runnable after a thread exited");
    };
    let claimed = queue.switch_in(cpu, next, now);
    let event = queue.next_event(next, now);
    drop(queue);
    reschedule_timer(event);
    let reason = task::exit_to(claimed);
    panic!("an exit switch was refused: {reason}");
}

fn start(context: &mut ThreadContext, _: usize) -> ! {
    let entry = current()
        .and_then(ThreadRef::hold)
        .and_then(|held| held.entry);
    match entry {
        Some((entry, argument)) => entry(context, argument),
        None => panic!("a thread started with no entry"),
    }
    exit(context)
}

// Dead and off its processor: what the reaper and teardown wait for. A thread already gone counts.
pub(crate) fn is_off_cpu(thread: ThreadRef) -> bool {
    thread
        .hold()
        .is_none_or(|held| held.task.is_dead() && !held.task.is_on_cpu())
}

pub(crate) fn owner(thread: ThreadRef) -> Option<ProcessRef> {
    thread.hold().and_then(|held| held.owner)
}

pub(crate) fn reap_next(thread: ThreadRef) -> Option<ThreadRef> {
    sched_of(thread, |sched| sched.reap_next)
}

pub(crate) fn link_reap(thread: ThreadRef, next: Option<ThreadRef>) {
    sched_of(thread, |sched| sched.reap_next = next);
}

// The record leaves its pool, then the stack is unmapped and its frames wait for their flush.
pub(crate) fn release(context: &mut ThreadContext, thread: ThreadRef) -> bool {
    let Some(world) = registry::hold(thread.world) else {
        return true;
    };
    let removed = match world.threads.destroy(thread.thread) {
        Ok(Some(removed)) => removed,
        Ok(None) | Err(Removal::Borrowed) => return false,
        Err(Removal::Refused(Refusal::Stale)) => return true,
        Err(Removal::Refused(reason)) => panic!("a thread record refused its removal: {reason}"),
    };
    drop(world);
    let Thread { task, .. } = removed;
    mem::unmap_task(context, task).release(context);
    #[cfg(feature = "selftest")]
    LIVE.fetch_sub(1, Ordering::AcqRel);
    true
}
