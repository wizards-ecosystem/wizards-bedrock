use core::sync::atomic::{AtomicBool, AtomicUsize};

use ps2_keys::{Decoder, Key, KeyEvent, KeyState};
use timebase::PollLimit;

use wost::sync::SpinLock;

use super::{EVENTS, KEYBOARD, Keystroke, is_interrupt_driven, poll};
use crate::arch::current::i8042::Controller;

/// How long a round waits for a person to press a key: time to read the on-screen instruction.
/// Every human round shares the bound so its prompt and measurement cannot expire at different
/// horizons.
#[cfg(feature = "selftest")]
pub(crate) const HUMAN_WAIT_MS: u64 = 45_000;

/// Take one already queued event without parking the self-test thread.
#[cfg(feature = "selftest")]
pub(crate) fn try_read_key_for_test() -> Option<Keystroke> {
    EVENTS.lock_leaf().pop()
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

#[cfg(feature = "selftest")]
pub(super) static POLLER_STARTED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static POLLER_PEER_PROGRESS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "selftest")]
pub(super) static POLLER_SAW_PEER: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

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

/// The decoder for whatever `controller` is actually delivering.
#[cfg(feature = "selftest")]
fn decoder_for(controller: &mut Controller) -> Decoder {
    if controller.identity() == Some(crate::arch::current::i8042::TRANSLATED_MF2) {
        Decoder::translated()
    } else {
        Decoder::new()
    }
}

/// Turn the controller's translation on, and reopen the question of what arrives.
/// The controller and decoder move together so their numbering cannot diverge.
/// The identity supplies only the opening guess: writing the bit or asking afterwards makes no
/// controller honest, and the next byte only one set could have sent settles it.
#[cfg(feature = "selftest")]
pub(crate) fn enable_translation() -> bool {
    let mut guard = KEYBOARD.lock_leaf();
    let Some(keyboard) = guard.as_mut() else {
        return false;
    };
    let stuck = keyboard.controller.enable_translation();
    keyboard.decoder = decoder_for(&mut keyboard.controller);
    keyboard.settled = false;
    keyboard.evidence = None;
    keyboard.held_len = 0;
    keyboard.held_at = 0;
    stuck
}

/// Seed the numbering the *other* way and let the stream put it right.
/// The wrong guess must exercise the correction path; test machines otherwise answer honestly.
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

/// A clock deadline bounds an honest wait and an iteration cap bounds a dead clock. Each pass
/// also bounds how much hostile controller input it drains.
/// Where the line is routed the handler is the only reader; polling here could claim a keystroke
/// the interrupt path never saw.
/// Otherwise this uses the fallback's bounded drain and delivery: the controller lock serializes
/// reads beside that thread, and `WATCH` observes only events admitted to readers.
#[cfg(feature = "selftest")]
pub(crate) fn await_armed_key(watch: ArmedKey) -> bool {
    /// The bound a dead clock falls back on, and only then: a fast loop must not shorten the
    /// forty-five-second human deadline.
    const WAIT_SPINS: u32 = 2_000_000;

    let mut patience = PollLimit::after_ms(clock_ns(), HUMAN_WAIT_MS, WAIT_SPINS);
    while patience.permit(clock_ns()) {
        if is_interrupt_driven() {
            if armed_key_arrived(watch) {
                return true;
            }
            core::hint::spin_loop();
            continue;
        }

        poll();
        if armed_key_arrived(watch) {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Keyboard interrupts this CPU has taken.
#[cfg(feature = "selftest")]
pub(super) static INTERRUPTS: AtomicUsize = AtomicUsize::new(0);

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
pub(crate) fn refuse_interrupt_readback() {
    Controller::refuse_interrupt_readback();
}

/// `None` selects [`PollLimit`]'s bounded-attempt fallback.
#[cfg(feature = "selftest")]
fn clock_ns() -> Option<u64> {
    crate::time::is_calibrated().then(crate::time::now_ns)
}
