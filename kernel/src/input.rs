//! The keyboard input path: drain, decode, queue, wake.
//!
//! The routed handler drains the controller; without a route, a thread polls
//! by sleeping or yielding cooperatively (D-37, D-49, D-57). Both paths feed
//! every byte through the
//! decoder, and each completed event is queued for a reader **if the queue has
//! room** — sixteen deep, and a keystroke arriving at a full one is dropped
//! rather than queued (D-145). Self-test counts losses; a waiting reader is
//! woken (D-61).
//!
//! Nothing here draws or logs: both are the reading thread's, which is what
//! keeps a console scroll off every other interrupt's latency path (D-63).
//!
//! The singleton is module-local rather than in `dev`, whose charter is the
//! output devices the panic path must reach; a panicking kernel has no use
//! for its keyboard.

#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(feature = "selftest")]
use ps2_keys::Key;
use ps2_keys::{Arriving, Decoder, KeyEvent, KeyState, SetProbe, Shift, us};
#[cfg(feature = "selftest")]
use timebase::PollLimit;
use wost::arch::current::interrupt;
use wost::sync::{Once, SpinLock};

use crate::arch::current::i8042::{Controller, InterruptEnableError};
use crate::thread::ThreadContext;
use crate::wait::WaitQueue;
use crate::{info, warn};

struct Keyboard {
    controller: Controller,
    decoder: Decoder,
    /// Whether the stream has said which numbering it is in.
    ///
    /// False until a byte only one set can produce arrives. Every probe the
    /// controller answers has been observed to lie in one direction or the
    /// other, so the bytes are the arbiter and `held` is what waiting for
    /// them costs (D-239).
    settled: bool,
    /// What the deciding byte proved to the self-test.
    #[cfg(feature = "selftest")]
    evidence: Option<Arriving>,
    /// Bytes seen before the numbering was known, replayed once it is.
    ///
    /// A keystroke decides within three bytes and set 2's longest sequence
    /// within five, so this is a keystroke's worth and then some. Decoding a
    /// held byte under the wrong numbering would name a different key, which
    /// is the whole defect.
    held: [u8; HELD_BYTES],
    held_len: usize,
    held_at: usize,
    shift: Shift,
}

/// Bytes held while the numbering is still undecided.
const HELD_BYTES: usize = 16;

/// How long a round waits for the person D-18 requires to press a key.
///
/// Forty-five seconds gives a person time to read the on-screen instruction.
/// Every human round shares the bound so its prompt and measurement cannot
/// expire at different horizons (D-240).
#[cfg(feature = "selftest")]
pub(crate) const HUMAN_WAIT_MS: u64 = 45_000;

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
            let byte = self.next_byte()?;
            if let Some(event) = self.decoder.feed(byte) {
                self.shift.observe(event);
                return Some(event);
            }
        }
        None
    }

    /// The next byte to decode, under the numbering the stream proved.
    ///
    /// Held bytes first, then the controller. Nothing is ever dropped and
    /// nothing is latched: until a byte answers the question, bytes wait; when
    /// one does, what waited is replayed under the numbering it proved.
    fn next_byte(&mut self) -> Option<u8> {
        loop {
            // Holding ends when a byte answers or a keystroke's worth has not;
            // the latter go through under the opening guess, nothing is dropped.
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
    ///
    /// Replaying what was held under the numbering it decided on is what keeps
    /// the keystroke that answered the question from being the one keystroke
    /// decoded wrongly.
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
            // The identity is only the opening guess: it and the config byte
            // are each wrong on some machine, so the first decisive byte
            // replaces it (D-178, D-179, D-239).
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
            if deliver(*keystroke) {
                delivered += 1;
            }
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
///
/// Touches no output device: the console lock is one a handler is now refused
/// (D-63), and a glyph can cost a scroll.
fn deliver(keystroke: Keystroke) -> bool {
    #[cfg(feature = "selftest")]
    WATCH.lock_leaf().observe(keystroke.event);
    EVENTS.lock_leaf().push(keystroke)
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

/// Wait for the next key event, blocking until one arrives.
///
/// The thread stops being runnable, nothing is armed for it, and the
/// keyboard's interrupt is what makes it runnable again. Nothing polls.
///
/// The check and the registration happen with the flag clear, together: a
/// wakeup arriving between "empty" and "waiting" is the classic way to lose
/// one for good.
pub(crate) fn read_key(context: &mut ThreadContext) -> Keystroke {
    loop {
        let saved = interrupt::disable();
        // Its own statement: held across the restore, this guard would leave
        // the queue locked with interrupts on (D-66).
        let keystroke = EVENTS.lock(context).pop();
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
    EVENTS.lock_leaf().dropped
}

/// Whether anything is waiting for a keystroke.
#[cfg(feature = "selftest")]
pub(crate) fn has_reader() -> bool {
    !READERS.is_idle()
}

/// Where the line could not be routed, give the drain a thread of its own.
///
/// The fallback owns its polling interval because no periodic tick wakes it
/// (D-37). Does nothing where the interrupt does the work.
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

/// Rounds a self-test poller has left, or [`FOREVER`] for the fallback.
#[cfg(feature = "selftest")]
static POLLER_ROUNDS: AtomicUsize = AtomicUsize::new(FOREVER);

/// A round count that never runs out.
#[cfg(feature = "selftest")]
const FOREVER: usize = usize::MAX;

/// Drain, wait, drain again — the whole of the fallback.
fn poll_forever() {
    let mut context = ThreadContext::require();
    loop {
        #[cfg(feature = "selftest")]
        if POLLER_ROUNDS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |rounds| {
            // `then_some` would evaluate the subtraction at zero.
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

/// Times the poller has been round its loop.
#[cfg(feature = "selftest")]
static POLLER_PASSES: AtomicUsize = AtomicUsize::new(0);

/// Whether a bounded check or the permanent fallback currently owns polling.
#[cfg(feature = "selftest")]
static POLLER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Times the poller has been round its loop.
#[cfg(feature = "selftest")]
pub(crate) fn poller_passes() -> usize {
    POLLER_PASSES.load(Ordering::Relaxed)
}

/// Start a bounded poller whatever the routing says, for the boot-time check
/// on a machine whose line is routed.
#[cfg(feature = "selftest")]
pub(crate) fn start_bounded_poller(rounds: usize) -> bool {
    // Refuse alongside any other poller because both share the round counter;
    // bounding one would terminate the other (D-105).
    if POLLER_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    POLLER_STARTED.store(false, Ordering::Release);
    POLLER_PEER_PROGRESS.store(false, Ordering::Release);
    POLLER_SAW_PEER.store(false, Ordering::Release);
    POLLER_ROUNDS.store(rounds, Ordering::Release);
    match crate::thread::spawn("keyboard-poll-check", poll_forever) {
        Ok(_) => true,
        Err(_) => {
            POLLER_RUNNING.store(false, Ordering::Release);
            false
        }
    }
}

#[cfg(feature = "selftest")]
static POLLER_STARTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "selftest")]
static POLLER_PEER_PROGRESS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "selftest")]
static POLLER_SAW_PEER: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(crate) fn poller_started_for_test() -> bool {
    POLLER_STARTED.load(Ordering::Acquire)
}

#[cfg(feature = "selftest")]
pub(crate) fn mark_poller_peer_progress_for_test() {
    POLLER_PEER_PROGRESS.store(true, Ordering::Release);
}

#[cfg(feature = "selftest")]
pub(crate) fn poller_saw_peer_progress_for_test() -> bool {
    POLLER_SAW_PEER.load(Ordering::Acquire)
}

#[cfg(feature = "selftest")]
pub(crate) fn queue_key_for_test(key: Key) -> bool {
    let admitted = deliver(Keystroke {
        event: KeyEvent {
            key,
            state: KeyState::Pressed,
        },
        character: None,
    });
    if admitted {
        READERS.wake_all();
    }
    admitted
}

/// Whether the controller came up.
#[cfg(feature = "selftest")]
pub(crate) fn is_ready() -> bool {
    KEYBOARD.lock_leaf().is_some()
}

/// The controller's configuration byte, read live from the hardware.
#[cfg(feature = "selftest")]
pub(crate) fn controller_config() -> Option<u8> {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .and_then(|keyboard| keyboard.controller.config())
}

#[cfg(feature = "selftest")]
pub(crate) fn configuration_refusal_is_visible() -> bool {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .is_some_and(|keyboard| keyboard.controller.configuration_refusal_is_visible())
}

#[cfg(feature = "selftest")]
pub(crate) fn flush_refusals_are_visible() -> bool {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .is_some_and(|keyboard| keyboard.controller.flush_refusals_are_visible())
}

#[cfg(feature = "selftest")]
pub(crate) fn transport_refusals_are_visible() -> bool {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .is_some_and(|keyboard| keyboard.controller.transport_refusals_are_visible())
}

/// Whether an echo command round-trips to the keyboard and back.
#[cfg(feature = "selftest")]
pub(crate) fn echo_roundtrips() -> bool {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .is_some_and(|keyboard| keyboard.controller.echo_roundtrips())
}

/// The scancode set the keyboard reports itself speaking.
#[cfg(feature = "selftest")]
pub(crate) fn scancode_set() -> Option<u8> {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .and_then(|keyboard| keyboard.controller.scancode_set())
}

/// The decoder for whatever `controller` is actually delivering.
#[cfg(feature = "selftest")]
fn decoder_for(controller: &mut Controller) -> Decoder {
    if controller.identity() == Some(crate::arch::current::i8042::TRANSLATED_MF2) {
        Decoder::translated()
    } else {
        Decoder::new()
    }
}

/// Turn the controller's translation on or off, and re-open the question of
/// what arrives.
///
/// The controller and decoder move together so their numbering cannot diverge
/// (D-179).
///
/// The identity supplies the opening guess and no longer the answer: writing
/// the bit does not make a controller honest, and neither does asking it
/// afterwards. What settles it is the next byte only one set could have sent
/// (D-239).
#[cfg(feature = "selftest")]
pub(crate) fn set_translation(translation: crate::arch::current::i8042::Translation) -> bool {
    let mut guard = KEYBOARD.lock_leaf();
    let Some(keyboard) = guard.as_mut() else {
        return false;
    };
    let stuck = keyboard.controller.set_translation(translation);
    keyboard.decoder = decoder_for(&mut keyboard.controller);
    keyboard.settled = false;
    keyboard.evidence = None;
    keyboard.held_len = 0;
    keyboard.held_at = 0;
    stuck
}

/// Whether the configured decoder matches the controller's opening identity.
#[cfg(feature = "selftest")]
pub(crate) fn decoder_matches_identity() -> bool {
    let mut guard = KEYBOARD.lock_leaf();
    let Some(keyboard) = guard.as_mut() else {
        return false;
    };
    match keyboard.controller.identity() {
        Some(crate::arch::current::i8042::TRANSLATED_MF2) => keyboard.decoder.reads_translated(),
        Some(crate::arch::current::i8042::UNTRANSLATED_MF2) => !keyboard.decoder.reads_translated(),
        Some(_) | None => false,
    }
}

/// Seed the numbering the *other* way and let the stream put it right.
///
/// Marks an incorrect guess settled so the stream must exercise its correction
/// path; available test machines otherwise answer their probes honestly
/// (D-28, D-239).
#[cfg(feature = "selftest")]
pub(crate) fn assume_wrong_numbering() {
    let mut guard = KEYBOARD.lock_leaf();
    let Some(keyboard) = guard.as_mut() else {
        return;
    };
    keyboard.decoder = if keyboard.decoder.reads_translated() {
        Decoder::new()
    } else {
        Decoder::translated()
    };
    keyboard.settled = false;
    keyboard.evidence = None;
    keyboard.held_len = 0;
    keyboard.held_at = 0;
}

/// Whether the driver is waiting on the stream rather than standing on a
/// numbering it inferred from a command.
#[cfg(feature = "selftest")]
pub(crate) fn rereads_the_stream() -> bool {
    KEYBOARD
        .lock_leaf()
        .as_ref()
        .is_some_and(|keyboard| !keyboard.settled && keyboard.evidence.is_none())
}

/// Whether the decoder in place reads the set the arriving bytes proved.
///
/// Uses a byte only one set could have sent, independently of the identity that
/// configured the decoder (D-239).
///
/// `None` evidence is not a pass: a keyboard that has delivered nothing has
/// proved nothing, and this is called where keystrokes have already arrived.
#[cfg(feature = "selftest")]
pub(crate) fn decoder_reads_what_arrives() -> bool {
    let guard = KEYBOARD.lock_leaf();
    let Some(keyboard) = guard.as_ref() else {
        return false;
    };
    match keyboard.evidence {
        Some(Arriving::Set1) => keyboard.decoder.reads_translated(),
        Some(Arriving::Set2) => !keyboard.decoder.reads_translated(),
        Some(Arriving::Undecided) | None => false,
    }
}

/// The two bytes the keyboard identifies itself with.
///
/// See [`crate::arch::current::i8042::Controller::identity`]: the pair tells a
/// translating controller from an honest one (D-178).
#[cfg(feature = "selftest")]
pub(crate) fn identity() -> Option<(u8, u8)> {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .and_then(|keyboard| keyboard.controller.identity())
}

#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArmedKey {
    key: Key,
    generation: u64,
}

#[cfg(feature = "selftest")]
pub(crate) fn arm_key(key: Key) -> Option<ArmedKey> {
    WATCH
        .lock_leaf()
        .generation(key)
        .map(|generation| ArmedKey { key, generation })
}

/// Whether a complete press/release pair arrived after `watch` was armed.
#[cfg(feature = "selftest")]
pub(crate) fn armed_key_arrived(watch: ArmedKey) -> bool {
    WATCH
        .lock_leaf()
        .completed_after(watch.key, watch.generation)
}

/// A clock deadline bounds an honest wait and an iteration cap bounds a dead
/// clock. Each pass also bounds how much hostile controller input it drains.
#[cfg(feature = "selftest")]
pub(crate) fn await_armed_key(watch: ArmedKey) -> bool {
    /// The bound a dead clock falls back on, and only then.
    ///
    /// Attempts are spent only when the clock is unavailable; otherwise a fast
    /// loop must not shorten the forty-five-second human deadline (D-200,
    /// D-240).
    const WAIT_SPINS: u32 = 2_000_000;

    let mut patience = PollLimit::after_ms(clock_ns(), HUMAN_WAIT_MS, WAIT_SPINS);
    while patience.permit(clock_ns()) {
        // Where the line is routed the handler is the only reader; polling
        // here could claim a keystroke the interrupt path never saw.
        if is_interrupt_driven() {
            if armed_key_arrived(watch) {
                return true;
            }
            core::hint::spin_loop();
            continue;
        }

        // Use the same bounded drain and delivery path as the permanent
        // fallback. A waiter may run beside that thread; the controller lock
        // serializes reads, and WATCH observes only events admitted to readers.
        poll();
        if armed_key_arrived(watch) {
            return true;
        }
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

/// Exercise the exact routed-path state transition without claiming that this
/// polling machine has a routed keyboard.
#[cfg(feature = "selftest")]
pub(crate) fn interrupt_handoff_is_recorded() -> bool {
    let handoff = Once::new();
    record_interrupt_handoff(&handoff);
    handoff.is_claimed()
}

/// Drain the controller from its interrupt handler.
///
/// The same work [`poll`] does, and bounded the same way. Safe to take the
/// keyboard and console locks from here for the reason `006-cpu-fundamentals.md`
/// establishes: `SpinLock` clears the interrupt flag while a lock is held, so
/// code holding either one cannot have been interrupted by this handler.
pub(crate) fn on_interrupt() -> bool {
    #[cfg(feature = "selftest")]
    INTERRUPTS.fetch_add(1, Ordering::Relaxed);
    poll() > 0
}

/// Keyboard interrupts this CPU has taken.
#[cfg(feature = "selftest")]
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

#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy)]
struct Watch {
    down: u128,
    completed: [u64; 128],
}

#[cfg(feature = "selftest")]
impl Watch {
    const EMPTY: Self = Self {
        down: 0,
        completed: [0; 128],
    };

    fn observe(&mut self, event: KeyEvent) {
        let Some(index) = Self::index(event.key) else {
            return;
        };
        let bit = 1u128 << index;
        match event.state {
            KeyState::Pressed => self.down |= bit,
            KeyState::Released => {
                if self.down & bit != 0 {
                    self.completed[index] = self.completed[index].wrapping_add(1);
                }
                self.down &= !bit;
            }
        }
    }

    fn index(key: Key) -> Option<usize> {
        let index = usize::from(key as u8);
        (index < 128).then_some(index)
    }

    fn generation(&self, key: Key) -> Option<u64> {
        Self::index(key).map(|index| self.completed[index])
    }

    fn completed_after(&self, key: Key, generation: u64) -> bool {
        self.generation(key)
            .is_some_and(|current| current != generation)
    }
}

#[cfg(feature = "selftest")]
static WATCH: SpinLock<Watch> = SpinLock::new(Watch::EMPTY);

#[cfg(feature = "selftest")]
pub(crate) fn watch_order_is_causal() -> bool {
    let key = Key::A;
    let mut watch = Watch::EMPTY;
    let start = watch.generation(key).unwrap_or(u64::MAX);
    watch.observe(KeyEvent {
        key,
        state: KeyState::Released,
    });
    let release_first = !watch.completed_after(key, start);
    watch.observe(KeyEvent {
        key,
        state: KeyState::Pressed,
    });
    let press_alone = !watch.completed_after(key, start);
    watch.observe(KeyEvent {
        key,
        state: KeyState::Released,
    });
    let ordered = watch.completed_after(key, start);
    let stale = watch
        .generation(key)
        .is_some_and(|snapshot| !watch.completed_after(key, snapshot));
    let index = Watch::index(key).unwrap_or(0);
    watch.completed[index] = u64::MAX;
    watch.observe(KeyEvent {
        key,
        state: KeyState::Pressed,
    });
    watch.observe(KeyEvent {
        key,
        state: KeyState::Released,
    });
    let wrapped = watch.generation(key) == Some(0);

    release_first && press_alone && ordered && stale && wrapped
}

#[cfg(feature = "selftest")]
pub(crate) fn queue_admission_is_exact() -> bool {
    let keystroke = Keystroke {
        event: KeyEvent {
            key: Key::A,
            state: KeyState::Pressed,
        },
        character: Some('a'),
    };
    let mut events = Events::new();
    let filled = (0..QUEUED).all(|_| events.push(keystroke));
    let refused = !events.push(keystroke) && events.dropped == 1;
    events.dropped = usize::MAX;
    let saturated = !events.push(keystroke) && events.dropped == usize::MAX;

    filled && refused && saturated
}

/// Ask the controller to raise its line when a byte arrives.
///
/// Called only after the I/O APIC has been given somewhere to send it.
pub(crate) fn enable_interrupts() -> Result<(), InterruptEnableError> {
    let mut guard = KEYBOARD.lock_leaf();
    guard
        .as_mut()
        .ok_or(InterruptEnableError::Unavailable)?
        .controller
        .enable_interrupt()
}

#[cfg(feature = "selftest")]
pub(crate) fn refuse_interrupt_readback() {
    Controller::refuse_interrupt_readback();
}

/// `None` selects [`PollLimit`]'s bounded-attempt fallback (D-200, D-240).
#[cfg(feature = "selftest")]
fn clock_ns() -> Option<u64> {
    crate::time::is_calibrated().then(crate::time::now_ns)
}
