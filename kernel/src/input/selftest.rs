use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ps2_keys::{Arriving, Decoder, Key, KeyEvent, KeyState};
use timebase::PollLimit;
use wost::arch::current::interrupt;
use wost::sync::{Once, SpinLock};

use super::{
    BATCH, EVENTS, Events, KEYBOARD, Keystroke, QUEUED, READERS, admit_and_observe, deliver,
    is_interrupt_driven, poll, poll_forever, produce_batch, record_interrupt_handoff,
};
use crate::arch::current::i8042::Controller;

/// How long a round waits for the person D-18 requires to press a key.
///
/// Forty-five seconds gives a person time to read the on-screen instruction.
/// Every human round shares the bound so its prompt and measurement cannot
/// expire at different horizons (D-240).
#[cfg(feature = "selftest")]
pub(crate) const HUMAN_WAIT_MS: u64 = 45_000;

/// Take one already queued event without parking the self-test thread.
#[cfg(feature = "selftest")]
pub(crate) fn try_read_key_for_test() -> Option<Keystroke> {
    EVENTS.lock_leaf().pop()
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

/// Rounds a self-test poller has left, or [`FOREVER`] for the fallback.
#[cfg(feature = "selftest")]
pub(super) static POLLER_ROUNDS: AtomicUsize = AtomicUsize::new(FOREVER);

/// A round count that never runs out.
#[cfg(feature = "selftest")]
pub(super) const FOREVER: usize = usize::MAX;

/// Times the poller has been round its loop.
#[cfg(feature = "selftest")]
pub(super) static POLLER_PASSES: AtomicUsize = AtomicUsize::new(0);

/// Whether a bounded check or the permanent fallback currently owns polling.
#[cfg(feature = "selftest")]
pub(super) static POLLER_RUNNING: AtomicBool = AtomicBool::new(false);

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
pub(super) static POLLER_STARTED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static POLLER_PEER_PROGRESS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static POLLER_SAW_PEER: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

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
#[derive(Clone, Copy, Debug)]
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

/// Exercise the exact routed-path state transition without claiming that this
/// polling machine has a routed keyboard.
#[cfg(feature = "selftest")]
pub(crate) fn interrupt_handoff_is_recorded() -> bool {
    let handoff = Once::new();
    record_interrupt_handoff(&handoff);
    handoff.is_claimed()
}

/// Keyboard interrupts this CPU has taken.
#[cfg(feature = "selftest")]
pub(super) static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);

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
#[derive(Clone, Copy, Debug)]
pub(super) struct Watch {
    down: u128,
    completed: [u64; 128],
}

#[cfg(feature = "selftest")]
impl Watch {
    const EMPTY: Self = Self {
        down: 0,
        completed: [0; 128],
    };

    pub(super) fn observe(&mut self, event: KeyEvent) {
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
pub(super) static WATCH: SpinLock<Watch> = SpinLock::new(Watch::EMPTY);

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

/// Exercise the production batch boundary with a finite competing producer.
#[cfg(feature = "selftest")]
pub(crate) fn producer_order_is_causal() -> bool {
    use core::cell::{Cell, RefCell};

    if !interrupt::are_enabled() {
        return false;
    }
    let events = RefCell::new(Events::new());
    let watch = RefCell::new(Watch::EMPTY);
    let peer_ran = Cell::new(false);
    let excluded = Cell::new(false);
    let pair = |keys: &mut [Option<Keystroke>; BATCH], key| {
        for (slot, state) in keys.iter_mut().zip([KeyState::Pressed, KeyState::Released]) {
            *slot = Some(Keystroke {
                event: KeyEvent { key, state },
                character: None,
            });
        }
        2
    };
    let deliver = |keystroke| {
        admit_and_observe(
            keystroke,
            |keystroke| events.borrow_mut().push(keystroke),
            |event| watch.borrow_mut().observe(event),
        )
    };
    let peer = || {
        peer_ran.set(true);
        produce_batch(|keys| pair(keys, Key::B), deliver, || {}) == (2, 2)
    };
    let peer_admitted = Cell::new(false);
    let first = produce_batch(
        |keys| pair(keys, Key::A),
        deliver,
        || {
            excluded.set(!interrupt::are_enabled());
            // This deterministic reentry models the producer a timer could
            // select; it does not claim that a hardware interrupt was delivered.
            if interrupt::are_enabled() {
                peer_admitted.set(peer());
            }
        },
    );
    let restored = interrupt::are_enabled();
    let deferred = !peer_ran.get();
    if deferred {
        peer_admitted.set(peer());
    }
    let mut events = events.into_inner();
    let ordered = [
        (Key::A, KeyState::Pressed),
        (Key::A, KeyState::Released),
        (Key::B, KeyState::Pressed),
        (Key::B, KeyState::Released),
    ]
    .into_iter()
    .all(|(key, state)| {
        events
            .pop()
            .is_some_and(|keystroke| keystroke.event.key == key && keystroke.event.state == state)
    });
    let watch = watch.into_inner();
    first == (2, 2)
        && excluded.get()
        && restored
        && deferred
        && peer_admitted.get()
        && interrupt::are_enabled()
        && ordered
        && events.pop().is_none()
        && events.dropped == 0
        && watch.generation(Key::A) == Some(1)
        && watch.generation(Key::B) == Some(1)
}

#[cfg(feature = "selftest")]
fn refused_events_do_not_complete_a_watch() -> bool {
    let keystroke = |key, state| Keystroke {
        event: KeyEvent { key, state },
        character: None,
    };
    let mut events = Events::new();
    let mut watch = Watch::EMPTY;
    let filled = (0..QUEUED).all(|_| events.push(keystroke(Key::B, KeyState::Pressed)));
    let before = watch.generation(Key::A);
    let pressed = admit_and_observe(
        keystroke(Key::A, KeyState::Pressed),
        |stroke| events.push(stroke),
        |event| watch.observe(event),
    );
    let released = admit_and_observe(
        keystroke(Key::A, KeyState::Released),
        |stroke| events.push(stroke),
        |event| watch.observe(event),
    );
    let unobserved =
        !pressed && !released && events.dropped == 2 && watch.generation(Key::A) == before;
    let drained = (0..QUEUED).all(|_| events.pop().is_some());
    let pressed = admit_and_observe(
        keystroke(Key::A, KeyState::Pressed),
        |stroke| events.push(stroke),
        |event| watch.observe(event),
    );
    let released = admit_and_observe(
        keystroke(Key::A, KeyState::Released),
        |stroke| events.push(stroke),
        |event| watch.observe(event),
    );
    filled
        && unobserved
        && drained
        && pressed
        && released
        && watch.generation(Key::A) == Some(1)
        && events
            .pop()
            .is_some_and(|stroke| stroke.event.state == KeyState::Pressed)
        && events
            .pop()
            .is_some_and(|stroke| stroke.event.state == KeyState::Released)
        && events.pop().is_none()
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

    filled && refused && saturated && refused_events_do_not_complete_a_watch()
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
