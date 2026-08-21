//! The polled keyboard input path: drain, decode, deliver.
//!
//! The driver raises no interrupts (D-37), so this runs from the idle loop
//! each time the tick wakes it: every pending byte is pulled through the
//! decoder, and each completed event is logged at debug level and — when it
//! types a character — echoed to the framebuffer console. Latency is
//! bounded by one tick, and the keyboard's own buffering holds an order of
//! magnitude more than a tick of typing.
//!
//! The singleton is module-local rather than in `dev`, whose charter is the
//! output devices the panic path must reach; a panicking kernel has no use
//! for its keyboard.

use core::fmt::Write;

#[cfg(feature = "selftest")]
use ps2_keys::Key;
use ps2_keys::{Decoder, KeyEvent, KeyState, Shift, us};
use wost::sync::SpinLock;

use crate::arch::current::i8042::Controller;
use crate::{debug, dev, info, warn};

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
#[derive(Clone, Copy)]
struct Delivery {
    event: KeyEvent,
    character: Option<char>,
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
            info!("keyboard: polled, scancode set 2");
        }
        Err(reason) => warn!("keyboard: {reason}; running without input"),
    }
}

/// Drain everything the controller is holding. The idle loop's work.
pub(crate) fn poll() {
    loop {
        let mut deliveries = [None; BATCH];
        let count = drain_batch(&mut deliveries);
        for delivery in deliveries.iter().flatten() {
            deliver(*delivery);
        }
        if count < BATCH {
            return;
        }
    }
}

/// Decode up to one batch under the lock, resolving each event's character
/// against the shift state as it stood at that event.
fn drain_batch(deliveries: &mut [Option<Delivery>; BATCH]) -> usize {
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
        deliveries[count] = Some(Delivery { event, character });
        count += 1;
    }
    count
}

/// Log the event, and echo what it typed. Runs outside the keyboard lock,
/// in mainline context, where taking the console locks is the sanctioned
/// nesting.
fn deliver(delivery: Delivery) {
    debug!("key: {:?} {:?}", delivery.event.key, delivery.event.state);
    if let Some(character) = delivery.character {
        dev::with_console(|console| {
            let _ = console.write_char(character);
        });
    }
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
/// Doubly bounded: a tick budget for the honest case, and an iteration cap
/// so a dead timer converts the wait into a red check rather than a hang.
/// The harness kills the run at sixty seconds regardless.
#[cfg(feature = "selftest")]
pub(crate) fn await_key(key: Key) -> bool {
    /// Ten seconds of ticks — generous, because injection crosses a process,
    /// a socket, and an emulated bus.
    const WAIT_TICKS: u64 = 1_000;
    /// Each spin is a lock and a status read — bus transactions — so this is
    /// seconds' worth: the bound a dead timer falls back on.
    const WAIT_SPINS: u32 = 2_000_000;

    let deadline = crate::time::ticks() + WAIT_TICKS;
    let mut pressed = false;
    for _ in 0..WAIT_SPINS {
        if crate::time::ticks() > deadline {
            return false;
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
