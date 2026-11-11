//! The keyboard input path: drain, decode, queue, wake.
//! The routed handler drains the controller; without a route, a thread polls by sleeping or
//! yielding cooperatively. Both feed every byte through the decoder.
//! Each completed event is queued **if the queue has room**: sixteen deep, and a keystroke
//! arriving at a full one is dropped. Self-test counts losses; a waiting reader is woken.
//! Nothing here draws or logs: both are the reading thread's, which keeps a console scroll off
//! every other interrupt's latency path.
//! The singleton is module-local, not in `dev`, which holds the output devices the panic path must
//! reach; a panicking kernel has no use for its keyboard.

#[cfg(feature = "selftest")]
use core::sync::atomic::Ordering;

use ps2_keys::{Arriving, Decoder, KeyEvent, KeyState, SetProbe, Shift, us};
use wost::arch::current::interrupt;
use wost::sync::{Once, SpinLock};

use crate::arch::current::i8042::{Controller, InterruptEnableError};
use crate::thread::ThreadContext;
use crate::wait::WaitQueue;
use crate::{info, warn};

#[cfg(feature = "selftest")]
mod selftest;

#[cfg(feature = "selftest")]
use selftest::{
    FOREVER, INTERRUPTS, POLLER_PASSES, POLLER_PEER_PROGRESS, POLLER_ROUNDS, POLLER_RUNNING,
    POLLER_SAW_PEER, POLLER_STARTED, WATCH,
};
#[cfg(feature = "selftest")]
pub(crate) use selftest::{
    HUMAN_WAIT_MS, arm_key, armed_key_arrived, assume_wrong_numbering, await_armed_key,
    configuration_refusal_is_visible, controller_config, decoder_matches_identity,
    decoder_reads_what_arrives, dropped_events, echo_roundtrips, flush_refusals_are_visible,
    has_reader, identity, interrupt_count, interrupt_handoff_is_recorded, is_ready,
    mark_poller_peer_progress_for_test, poller_passes, poller_saw_peer_progress_for_test,
    poller_started_for_test, producer_order_is_causal, queue_admission_is_exact,
    queue_key_for_test, refuse_interrupt_readback, rereads_the_stream, scancode_set,
    set_translation, start_bounded_poller, transport_refusals_are_visible, try_read_key_for_test,
    watch_order_is_causal,
};

struct Keyboard {
    controller: Controller,
    decoder: Decoder,
    /// Whether the stream has said which numbering it is in: false until a byte only one set can
    /// produce arrives.
    /// Every probe the controller answers has been observed to lie one way or the other, so the
    /// bytes are the arbiter and `held` is what waiting for them costs.
    settled: bool,
    /// What the deciding byte proved to the self-test.
    #[cfg(feature = "selftest")]
    evidence: Option<Arriving>,
    /// Bytes seen before the numbering was known, replayed once it is; decoding one under the
    /// wrong numbering would name a different key.
    /// A keystroke decides within three bytes and set 2's longest sequence within five, so this is
    /// a keystroke's worth and then some.
    held: [u8; HELD_BYTES],
    held_len: usize,
    held_at: usize,
    shift: Shift,
}

/// Bytes held while the numbering is still undecided.
const HELD_BYTES: usize = 16;

impl Keyboard {
    /// Pull bytes until one completes an event, the buffer runs dry, or the byte budget is spent.
    /// The budget stops a controller streaming bytes that never complete an event from pinning the
    /// caller's lock, which "drain until empty" bounds only by the device's good behaviour.
    fn next_event(&mut self) -> Option<KeyEvent> {
        const BYTE_BUDGET: u32 = 16;

        for _ in 0..BYTE_BUDGET {
            let byte = self.next_byte()?;
            if let Some(event) = self.decoder.feed(byte) {
                self.shift.observe(event);
                return Some(event);
            }
        }
        None
    }

    /// The next byte to decode under the numbering the stream proved: held bytes first, then the
    /// controller. Nothing is ever dropped or latched: bytes wait until one answers, then replay.
    /// Holding ends when a byte answers or a keystroke's worth has not; those pass under the
    /// opening guess.
    fn next_byte(&mut self) -> Option<u8> {
        loop {
            let holding = !self.settled && self.held_len < HELD_BYTES;
            if !holding && self.held_at < self.held_len {
                let byte = self.held[self.held_at];
                self.held_at += 1;
                if self.held_at == self.held_len {
                    self.held_at = 0;
                    self.held_len = 0;
                }
                return Some(byte);
            }
            let byte = self.controller.poll_byte()?;
            if self.settled && self.held_len == 0 {
                return Some(byte);
            }
            self.held[self.held_len] = byte;
            self.held_len += 1;
            self.adopt(SetProbe::observe(byte));
        }
    }

    /// Adopt what the stream proved, and say whether it proved anything.
    /// Held bytes replay under it, so the keystroke that answered is not the one decoded wrongly.
    fn adopt(&mut self, arriving: Arriving) -> bool {
        let translated = match arriving {
            Arriving::Undecided => return false,
            Arriving::Set1 => true,
            Arriving::Set2 => false,
        };
        if translated != self.decoder.reads_translated() {
            self.decoder = if translated {
                Decoder::translated()
            } else {
                Decoder::new()
            };
        }
        #[cfg(feature = "selftest")]
        {
            self.evidence = Some(arriving);
        }
        self.settled = true;
        true
    }
}

static KEYBOARD: SpinLock<Option<Keyboard>> = SpinLock::new(None);

/// One decoded event, with its character resolved while the shift state around it was current;
/// a reader may not look for milliseconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Keystroke {
    /// The key, and whether it went down or up.
    pub(crate) event: KeyEvent,
    /// What it typed, if it typed anything.
    pub(crate) character: Option<char>,
}

/// Events decoded per hold of the keyboard lock; with [`Keyboard::next_event`]'s byte budget, no
/// stream of input can pin the lock for its whole length.
const BATCH: usize = 8;

/// Bring the keyboard up, or report why the kernel runs without one.
/// The identity is only the opening guess: it and the configuration byte are each wrong on some
/// machine, so the first decisive byte replaces it.
pub(crate) fn init() {
    match Controller::new() {
        Ok(controller) => {
            let translated = controller.is_translated();
            let decoder = if translated {
                Decoder::translated()
            } else {
                Decoder::new()
            };
            *KEYBOARD.lock_leaf() = Some(Keyboard {
                controller,
                decoder,
                settled: false,
                #[cfg(feature = "selftest")]
                evidence: None,
                held: [0; HELD_BYTES],
                held_len: 0,
                held_at: 0,
                shift: Shift::new(),
            });
            let set = if translated { 1 } else { 2 };
            info!("keyboard: up, set {set} until an arriving byte says otherwise");
        }
        Err(reason) => warn!("keyboard: {reason}; running without input"),
    }
}

/// Drain what the controller is holding, and return how many threads its queued events woke.
/// Bounded so a controller that never stops producing complete events cannot own the idle loop;
/// the lock is bounded per batch already, and what remains drains on the next pass.
/// The wake is last and outside every other lock: a woken reader may be picked the moment the
/// handler returns, and it will want the queue this filled.
/// The trap handler uses the count to decide whether the scheduler needs asking who runs now.
pub(crate) fn poll() -> usize {
    /// Batches per call: two orders of magnitude past human typing, and finite either way.
    const BATCHES: usize = 8;

    let mut delivered = 0;
    for _ in 0..BATCHES {
        let (count, admitted) = produce_batch(drain_batch, deliver, || {});
        delivered += admitted;
        if count < BATCH {
            break;
        }
    }
    if delivered > 0 { READERS.wake_all() } else { 0 }
}

/// A second polling producer must not enqueue a later batch ahead of this one, so interrupts stay
/// excluded; the keyboard and event guards stay sequential under this exclusion.
fn produce_batch(
    drain: impl FnOnce(&mut [Option<Keystroke>; BATCH]) -> usize,
    mut deliver: impl FnMut(Keystroke) -> bool,
    after_drain: impl FnOnce(),
) -> (usize, usize) {
    let exclusion = interrupt::disable();
    let mut keystrokes = [None; BATCH];
    let count = drain(&mut keystrokes);
    after_drain();
    let mut delivered = 0;
    for keystroke in keystrokes.iter().flatten() {
        if deliver(*keystroke) {
            delivered += 1;
        }
    }
    exclusion.restore();
    (count, delivered)
}

/// Decode up to one batch under the lock, resolving each event's character
/// against the shift state as it stood at that event.
fn drain_batch(keystrokes: &mut [Option<Keystroke>; BATCH]) -> usize {
    let mut guard = KEYBOARD.lock_leaf();
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
/// It touches no output device: a handler is refused the console lock, and a glyph can cost a
/// scroll.
fn deliver(keystroke: Keystroke) -> bool {
    admit_and_observe(
        keystroke,
        |keystroke| EVENTS.lock_leaf().push(keystroke),
        |event| {
            #[cfg(feature = "selftest")]
            WATCH.lock_leaf().observe(event);
            #[cfg(not(feature = "selftest"))]
            let _ = event;
        },
    )
}

fn admit_and_observe(
    keystroke: Keystroke,
    enqueue: impl FnOnce(Keystroke) -> bool,
    observe: impl FnOnce(KeyEvent),
) -> bool {
    let admitted = enqueue(keystroke);
    if admitted {
        observe(keystroke.event);
    }
    admitted
}

/// Decoded events waiting for a reader, in a fixed ring: the producer is an interrupt handler,
/// where the heap is forbidden.
const QUEUED: usize = 16;

/// The events themselves, and what has been lost.
#[derive(Debug)]
struct Events {
    ring: [Option<Keystroke>; QUEUED],
    /// Where the next event goes.
    head: usize,
    /// Where the next read comes from.
    tail: usize,
    /// Events the self-test saw dropped because nobody read fast enough.
    #[cfg(feature = "selftest")]
    dropped: usize,
}

impl Events {
    const fn new() -> Self {
        Self {
            ring: [None; QUEUED],
            head: 0,
            tail: 0,
            #[cfg(feature = "selftest")]
            dropped: 0,
        }
    }

    fn push(&mut self, keystroke: Keystroke) -> bool {
        if self.ring[self.head].is_some() {
            #[cfg(feature = "selftest")]
            {
                self.dropped = self.dropped.saturating_add(1);
            }
            return false;
        }
        self.ring[self.head] = Some(keystroke);
        self.head = (self.head + 1) % QUEUED;
        true
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

/// Wait for the next key event; nothing is armed and nothing polls until the interrupt wakes it.
/// The check and the registration happen with the flag clear, together: a wakeup arriving between
/// "empty" and "waiting" is the classic way to lose one for good.
/// The queue guard is its own statement: held across the restore, it would leave the queue locked
/// with interrupts on.
pub(crate) fn read_key(context: &mut ThreadContext) -> Keystroke {
    loop {
        let saved = interrupt::disable();
        let keystroke = EVENTS.lock(context).pop();
        if let Some(keystroke) = keystroke {
            saved.restore();
            return keystroke;
        }
        READERS.wait(context);
        saved.restore();
    }
}

/// Where the line could not be routed, give the drain a thread of its own; else do nothing.
/// The fallback owns its polling interval because no periodic tick wakes it.
pub(crate) fn start_poller() {
    if is_interrupt_driven() {
        return;
    }
    #[cfg(feature = "selftest")]
    {
        if POLLER_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        POLLER_ROUNDS.store(FOREVER, Ordering::Release);
    }
    match crate::thread::spawn("keyboard-poll", poll_forever) {
        Ok(_) if crate::time::is_calibrated() && interrupt::are_enabled() => {
            info!("keyboard: polled every {POLL_INTERVAL_MS} ms")
        }
        Ok(_) => info!("keyboard: clockless poller yields between bounded drains"),
        Err(reason) => {
            #[cfg(feature = "selftest")]
            POLLER_RUNNING.store(false, Ordering::Release);
            warn!("keyboard: no poller: {reason}; input is lost");
        }
    }
}

/// Interval required by the fallback polling path.
const POLL_INTERVAL_MS: u64 = 10;

/// Drain, wait, drain again: the whole of the fallback.
/// The round count uses `then`, since `then_some` would evaluate the subtraction at zero.
fn poll_forever() {
    let mut context = ThreadContext::require();
    loop {
        #[cfg(feature = "selftest")]
        if POLLER_ROUNDS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |rounds| {
            (rounds > 0).then(|| {
                if rounds == FOREVER {
                    FOREVER
                } else {
                    rounds - 1
                }
            })
        }) == Err(0)
        {
            break;
        }
        poll();
        #[cfg(feature = "selftest")]
        POLLER_PASSES.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "selftest")]
        POLLER_STARTED.store(true, Ordering::Release);
        if crate::time::is_calibrated() && interrupt::are_enabled() {
            crate::thread::sleep_ms(&mut context, POLL_INTERVAL_MS);
        } else {
            crate::thread::yield_now(&mut context);
        }
        #[cfg(feature = "selftest")]
        if POLLER_PEER_PROGRESS.load(Ordering::Acquire) {
            POLLER_SAW_PEER.store(true, Ordering::Release);
        }
    }
    #[cfg(feature = "selftest")]
    POLLER_RUNNING.store(false, Ordering::Release);
}

/// Set once the keyboard's line is routed and unmasked.
static INTERRUPT_DRIVEN: Once = Once::new();

/// Whether keystrokes arrive as interrupts rather than by polling.
pub(crate) fn is_interrupt_driven() -> bool {
    INTERRUPT_DRIVEN.is_claimed()
}

fn record_interrupt_handoff(latch: &Once) {
    assert!(
        latch.claim(),
        "the keyboard interrupt handoff happened more than once"
    );
}

/// Record that the controller's line is live, so the idle loop stops polling.
pub(crate) fn take_interrupts() {
    record_interrupt_handoff(&INTERRUPT_DRIVEN);
}

/// Drain the controller from its interrupt handler, as [`poll`] does and bounded the same way.
/// The keyboard and console locks are safe here, as `006-cpu-fundamentals.md` establishes:
/// `SpinLock` clears the interrupt flag while held, so no holder was interrupted by this handler.
pub(crate) fn on_interrupt() -> bool {
    #[cfg(feature = "selftest")]
    INTERRUPTS.fetch_add(1, Ordering::Relaxed);
    poll() > 0
}

/// Ask the controller to raise its line when a byte arrives.
/// Called only after the I/O APIC has been given somewhere to send it.
pub(crate) fn enable_interrupts() -> Result<(), InterruptEnableError> {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .ok_or(InterruptEnableError::Unavailable)?
        .controller
        .enable_interrupt()
}
