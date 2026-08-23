//! The keyboard input path: drain, decode, queue, wake.
//!
//! Where the line is routed the handler does the draining (D-49); where it is
//! not, a thread of its own sleeps ten milliseconds at a time and does it in
//! between (D-37, D-57). Either way every pending byte is pulled through the
//! decoder and each completed event is queued for a reader, which is woken if
//! one is waiting (D-61).
//!
//! Nothing here draws or logs: both are the reading thread's, which is what
//! keeps a console scroll off every other interrupt's latency path (D-63).
//!
//! The singleton is module-local rather than in `dev`, whose charter is the
//! output devices the panic path must reach; a panicking kernel has no use
//! for its keyboard.

use core::sync::atomic::{AtomicUsize, Ordering};

use ps2_keys::{Decoder, Key, KeyEvent, KeyState, Shift, us};
use wost::arch::current::interrupt;
use wost::panic::PanicToken;
use wost::sync::{Once, SpinLock};

use crate::arch::current::i8042::Controller;
use crate::thread::ThreadContext;
use crate::wait::WaitQueue;
use crate::{info, warn};

struct Keyboard {
    controller: Controller,
    decoder: Decoder,
    shift: Shift,
}

impl Keyboard {
    /// Pull bytes until one completes an event, the buffer runs dry, or the
    /// per-call byte budget is spent.
    ///
    /// The budget is what makes a controller streaming protocol garbage —
    /// bytes that never complete an event — unable to pin the caller's lock:
    /// without it, "drain until empty" is bounded only by the device's good
    /// behaviour.
    fn next_event(&mut self) -> Option<KeyEvent> {
        const BYTE_BUDGET: u32 = 16;

        for _ in 0..BYTE_BUDGET {
            let byte = self.controller.poll_byte()?;
            if let Some(event) = self.decoder.feed(byte) {
                self.shift.observe(event);
                return Some(event);
            }
        }
        None
    }
}

static KEYBOARD: SpinLock<Option<Keyboard>> = SpinLock::new(None);

/// One decoded event, with the character it typed resolved while the shift
/// state around it was still current.
///
/// The character travels with the event: the shift state that decided it was
/// current then, and a reader may not look for milliseconds.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keystroke {
    /// The key, and whether it went down or up.
    pub(crate) event: KeyEvent,
    /// What it typed, if it typed anything.
    pub(crate) character: Option<char>,
}

/// Events decoded per hold of the keyboard lock. With the byte budget in
/// [`Keyboard::next_event`], a hold is bounded in bytes as well as events,
/// so no stream of input can pin the lock for its whole length.
const BATCH: usize = 8;

/// Bring the keyboard up, or report why the kernel runs without one.
pub(crate) fn init() {
    match Controller::new() {
        Ok(controller) => {
            *KEYBOARD.lock() = Some(Keyboard {
                controller,
                decoder: Decoder::new(),
                shift: Shift::new(),
            });
            info!("keyboard: up, scancode set 2");
        }
        Err(reason) => warn!("keyboard: {reason}; running without input"),
    }
}

/// Drain what the controller is holding. The idle loop's work.
///
/// Bounded like every other wait in this kernel, and for the same reason: a
/// controller that never stops producing complete events would otherwise own
/// the idle loop for as long as it kept producing. The lock is already
/// bounded per batch, so what this bound protects is the loop's turn, not the
/// lock — anything still waiting is drained on the next pass, one interval
/// later.
///
/// Returns how many threads the events it queued woke, which the trap handler
/// uses to decide whether the scheduler needs asking who should run now.
pub(crate) fn poll() -> usize {
    /// Batches per call. With [`BATCH`] events each, one pass at the keyboard
    /// is two orders of magnitude more than a person can type into it, and
    /// finite either way.
    const BATCHES: usize = 8;

    let mut delivered = 0;
    for _ in 0..BATCHES {
        let mut keystrokes = [None; BATCH];
        let count = drain_batch(&mut keystrokes);
        for keystroke in keystrokes.iter().flatten() {
            deliver(*keystroke);
            delivered += 1;
        }
        if count < BATCH {
            break;
        }
    }
    // Outside every other lock, and last: a woken reader may be picked the
    // moment the handler returns, and it will want the queue this filled.
    if delivered > 0 { READERS.wake_all() } else { 0 }
}

/// Decode up to one batch under the lock, resolving each event's character
/// against the shift state as it stood at that event.
fn drain_batch(keystrokes: &mut [Option<Keystroke>; BATCH]) -> usize {
    let mut guard = KEYBOARD.lock();
    let Some(keyboard) = guard.as_mut() else {
        return 0;
    };

    let mut count = 0;
    while count < BATCH {
        let Some(event) = keyboard.next_event() else {
            break;
        };
        let character = (event.state == KeyState::Pressed)
            .then(|| us::char_for(event.key, keyboard.shift))
            .flatten();
        keystrokes[count] = Some(Keystroke { event, character });
        count += 1;
    }
    count
}

/// Queue the event for whoever is reading, and note it for the self-checks.
///
/// Touches no output device: the console lock is one a handler is now refused
/// (D-63), and a glyph can cost a scroll.
fn deliver(keystroke: Keystroke) {
    WATCH.lock().observe(keystroke.event);
    EVENTS.lock().push(keystroke);
}

/// Decoded events waiting for a reader.
///
/// A fixed ring: the producer is an interrupt handler, where D-55 forbids the
/// heap.
const QUEUED: usize = 16;

/// The events themselves, and what has been lost.
#[derive(Debug)]
struct Events {
    ring: [Option<Keystroke>; QUEUED],
    /// Where the next event goes.
    head: usize,
    /// Where the next read comes from.
    tail: usize,
    /// Events dropped because nobody was reading fast enough; should stay at
    /// zero. The newest goes, not the oldest: the keystrokes worth keeping
    /// are the ones already typed.
    dropped: usize,
}

impl Events {
    const fn new() -> Self {
        Self {
            ring: [None; QUEUED],
            head: 0,
            tail: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, keystroke: Keystroke) {
        if self.ring[self.head].is_some() {
            self.dropped += 1;
            return;
        }
        self.ring[self.head] = Some(keystroke);
        self.head = (self.head + 1) % QUEUED;
    }

    fn pop(&mut self) -> Option<Keystroke> {
        let keystroke = self.ring[self.tail].take()?;
        self.tail = (self.tail + 1) % QUEUED;
        Some(keystroke)
    }
}

static EVENTS: SpinLock<Events> = SpinLock::new(Events::new());

/// Threads waiting for a keystroke.
static READERS: WaitQueue = WaitQueue::new();

/// Wait for the next key event, blocking until one arrives.
///
/// The thread stops being runnable, nothing is armed for it, and the
/// keyboard's interrupt is what makes it runnable again. Nothing polls.
///
/// The check and the registration happen with the flag clear, together: a
/// wakeup arriving between "empty" and "waiting" is the classic way to lose
/// one for good.
pub(crate) fn read_key(context: &ThreadContext) -> Keystroke {
    loop {
        let saved = interrupt::disable();
        // A statement of its own: held across the restore below, this guard
        // would leave the queue locked with the flag set, and the next
        // keyboard interrupt would spin on it for ever (D-66).
        let keystroke = EVENTS.lock().pop();
        if let Some(keystroke) = keystroke {
            saved.restore();
            return keystroke;
        }
        READERS.wait(context);
        saved.restore();
    }
}

/// Events dropped because no reader kept up. See [`Events::dropped`].
#[cfg(feature = "selftest")]
pub(crate) fn dropped_events() -> usize {
    EVENTS.lock().dropped
}

/// Whether anything is waiting for a keystroke.
#[cfg(feature = "selftest")]
pub(crate) fn has_reader() -> bool {
    !READERS.is_idle()
}

/// Where the line could not be routed, give the drain a thread of its own.
///
/// The retired tick used to wake whoever did this (D-37). Now the fallback
/// asks for the interval it needs and pays for it alone. Does nothing where
/// the interrupt does the work.
pub(crate) fn start_poller() {
    if is_interrupt_driven() {
        return;
    }
    match crate::thread::spawn("keyboard-poll", poll_forever) {
        Ok(_) => info!("keyboard: polled every {POLL_INTERVAL_MS} ms"),
        Err(reason) => warn!("keyboard: no poller: {reason}; input is lost"),
    }
}

/// What the retired tick used to give this path, restated as the interval it
/// always was.
const POLL_INTERVAL_MS: u64 = 10;

/// Rounds the poller has left, or [`FOREVER`] for the shipped one. Bounded
/// only so the boot-time check can watch this path work and then leave.
static POLLER_ROUNDS: AtomicUsize = AtomicUsize::new(FOREVER);

/// A round count that never runs out.
const FOREVER: usize = usize::MAX;

/// Drain, wait, drain again — the whole of the fallback.
fn poll_forever() {
    let context = ThreadContext::require();
    // `then`, not `then_some`: the latter evaluates the subtraction even at
    // zero, which wraps in release and panics in debug.
    while POLLER_ROUNDS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |rounds| {
        (rounds > 0).then(|| {
            if rounds == FOREVER {
                FOREVER
            } else {
                rounds - 1
            }
        })
    }) != Err(0)
    {
        poll();
        POLLER_PASSES.fetch_add(1, Ordering::Relaxed);
        crate::thread::sleep_ms(&context, POLL_INTERVAL_MS);
    }
}

/// Times the poller has been round its loop.
static POLLER_PASSES: AtomicUsize = AtomicUsize::new(0);

/// Times the poller has been round its loop.
#[cfg(feature = "selftest")]
pub(crate) fn poller_passes() -> usize {
    POLLER_PASSES.load(Ordering::Relaxed)
}

/// Start a bounded poller whatever the routing says, for the boot-time check
/// on a machine whose line is routed.
#[cfg(feature = "selftest")]
pub(crate) fn start_bounded_poller(rounds: usize) -> bool {
    POLLER_ROUNDS.store(rounds, Ordering::Release);
    crate::thread::spawn("keyboard-poll-check", poll_forever).is_ok()
}

/// Reclaim the keyboard lock on the terminal panic path.
///
/// A panicking kernel has no use for its keyboard, but it does have a use for
/// not deadlocking: the panic can have been raised by a fault taken while this
/// lock was held, and a fault is not masked by the interrupt flag.
pub(crate) fn force_unlock(token: &PanicToken) {
    KEYBOARD.force_unlock(token);
    EVENTS.force_unlock(token);
    READERS.force_unlock(token);
}

/// Whether the controller came up.
#[cfg(feature = "selftest")]
pub(crate) fn is_ready() -> bool {
    KEYBOARD.lock().is_some()
}

/// The controller's configuration byte, read live from the hardware.
#[cfg(feature = "selftest")]
pub(crate) fn controller_config() -> Option<u8> {
    let mut guard = KEYBOARD.lock();
    guard
        .as_mut()
        .and_then(|keyboard| keyboard.controller.config())
}

/// Whether an echo command round-trips to the keyboard and back.
#[cfg(feature = "selftest")]
pub(crate) fn echo_roundtrips() -> bool {
    let mut guard = KEYBOARD.lock();
    guard
        .as_mut()
        .is_some_and(|keyboard| keyboard.controller.echo_roundtrips())
}

/// The scancode set the keyboard reports itself speaking.
#[cfg(feature = "selftest")]
pub(crate) fn scancode_set() -> Option<u8> {
    let mut guard = KEYBOARD.lock();
    guard
        .as_mut()
        .and_then(|keyboard| keyboard.controller.scancode_set())
}

/// Wait for `key` to arrive through the whole path — controller, decoder —
/// pressed and then released, in that order.
///
/// Doubly bounded: a deadline on the clock for the honest case, and an
/// iteration cap so a dead clock converts the wait into a red check rather
/// than a hang. The harness kills the run at sixty seconds regardless.
#[cfg(feature = "selftest")]
pub(crate) fn await_key(key: Key) -> bool {
    /// Ten seconds — generous, because injection crosses a process, a socket,
    /// and an emulated bus.
    const WAIT_MS: u64 = 10_000;
    /// Each spin is a lock and a status read — bus transactions — so this is
    /// seconds' worth: the bound a dead clock falls back on.
    const WAIT_SPINS: u32 = 2_000_000;

    let deadline = crate::time::uptime_ms() + WAIT_MS;
    let mut pressed = false;
    for _ in 0..WAIT_SPINS {
        if crate::time::uptime_ms() > deadline {
            return false;
        }

        // Where the line is routed, the handler is the only reader: polling
        // here would race it for the byte and could report success on a
        // keystroke the interrupt path never saw, which is the one thing this
        // check exists to rule out.
        if is_interrupt_driven() {
            if WATCH.lock().saw(key) {
                return true;
            }
            core::hint::spin_loop();
            continue;
        }

        let mut guard = KEYBOARD.lock();
        let Some(keyboard) = guard.as_mut() else {
            return false;
        };
        while let Some(event) = keyboard.next_event() {
            if event.key != key {
                continue;
            }
            match event.state {
                KeyState::Pressed => pressed = true,
                KeyState::Released if pressed => return true,
                KeyState::Released => {}
            }
        }
        drop(guard);
        core::hint::spin_loop();
    }
    false
}

/// Set once the keyboard's line is routed and unmasked.
static INTERRUPT_DRIVEN: Once = Once::new();

/// Whether keystrokes arrive as interrupts rather than by polling.
pub(crate) fn is_interrupt_driven() -> bool {
    INTERRUPT_DRIVEN.is_claimed()
}

/// Record that the controller's line is live, so the idle loop stops polling.
pub(crate) fn take_interrupts() {
    INTERRUPT_DRIVEN.claim();
}

/// Drain the controller from its interrupt handler.
///
/// The same work [`poll`] does, and bounded the same way. Safe to take the
/// keyboard and console locks from here for the reason `006-cpu-fundamentals.md`
/// establishes: `SpinLock` clears the interrupt flag while a lock is held, so
/// code holding either one cannot have been interrupted by this handler.
pub(crate) fn on_interrupt() -> bool {
    INTERRUPTS.fetch_add(1, Ordering::Relaxed);
    poll() > 0
}

/// Keyboard interrupts this CPU has taken.
static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);

/// How many keyboard interrupts have been delivered.
///
/// The evidence that routing worked. Nothing else distinguishes a keystroke
/// the handler decoded from one the idle loop happened to poll — which is
/// exactly the confusion the boot-time check has to avoid.
#[cfg(feature = "selftest")]
pub(crate) fn interrupt_count() -> usize {
    INTERRUPTS.load(Ordering::Relaxed)
}

/// Every key seen pressed, and every key seen released, since boot.
///
/// Accumulated, not sampled: a burst of keystrokes can be decoded in a single
/// drain, and a reader that only saw the last of them would wait for ever on
/// the first. Two bitmasks over `Key as u8`, whose width is checked because
/// the enum is non-exhaustive.
#[derive(Debug, Clone, Copy, Default)]
struct Watch {
    pressed: u128,
    released: u128,
}

impl Watch {
    /// Note an event.
    fn observe(&mut self, event: KeyEvent) {
        let Some(bit) = Self::bit(event.key) else {
            return;
        };
        match event.state {
            KeyState::Pressed => self.pressed |= bit,
            KeyState::Released => self.released |= bit,
        }
    }

    /// Where a key sits in the masks, or `None` for one past their width.
    fn bit(key: Key) -> Option<u128> {
        1u128.checked_shl(u32::from(key as u8))
    }

    /// Whether `key` has been both pressed and released.
    #[cfg(feature = "selftest")]
    fn saw(&self, key: Key) -> bool {
        Self::bit(key).is_some_and(|bit| self.pressed & bit != 0 && self.released & bit != 0)
    }
}

static WATCH: SpinLock<Watch> = SpinLock::new(Watch {
    pressed: 0,
    released: 0,
});

/// Reclaim the watch's lock on the terminal panic path.
pub(crate) fn force_unlock_watch(token: &PanicToken) {
    WATCH.force_unlock(token);
}

/// Ask the controller to raise its line when a byte arrives.
///
/// Called only after the I/O APIC has been given somewhere to send it.
pub(crate) fn enable_interrupts() -> bool {
    let mut guard = KEYBOARD.lock();
    guard
        .as_mut()
        .is_some_and(|keyboard| keyboard.controller.enable_interrupt())
}
