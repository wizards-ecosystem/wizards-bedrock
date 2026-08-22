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
use core::sync::atomic::{AtomicUsize, Ordering};

use ps2_keys::{Decoder, Key, KeyEvent, KeyState, Shift, us};
use wost::panic::PanicToken;
use wost::sync::{Once, SpinLock};

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
/// lock — anything still waiting is drained on the next tick, one tick later.
pub(crate) fn poll() {
    /// Batches per call. With [`BATCH`] events each, a tick's turn at the
    /// keyboard is two orders of magnitude more than a person can type into
    /// it, and finite either way.
    const BATCHES: usize = 8;

    for _ in 0..BATCHES {
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
    WATCH.lock().observe(delivery.event);
    if let Some(character) = delivery.character {
        dev::with_console(|console| {
            let _ = console.write_char(character);
        });
    }
}

/// Reclaim the keyboard lock on the terminal panic path.
///
/// A panicking kernel has no use for its keyboard, but it does have a use for
/// not deadlocking: the panic can have been raised by a fault taken while this
/// lock was held, and a fault is not masked by the interrupt flag.
pub(crate) fn force_unlock(token: &PanicToken) {
    KEYBOARD.force_unlock(token);
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
pub(crate) fn on_interrupt() {
    INTERRUPTS.fetch_add(1, Ordering::Relaxed);
    poll();
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

/// The last press and release seen, whichever path delivered them.
#[derive(Debug, Clone, Copy, Default)]
struct Watch {
    pressed: Option<Key>,
    released: Option<Key>,
}

impl Watch {
    /// Note an event.
    fn observe(&mut self, event: KeyEvent) {
        match event.state {
            KeyState::Pressed => self.pressed = Some(event.key),
            KeyState::Released => self.released = Some(event.key),
        }
    }

    /// Whether `key` has been both pressed and released.
    #[cfg(feature = "selftest")]
    const fn saw(&self, key: Key) -> bool {
        matches!((self.pressed, self.released), (Some(down), Some(up)) if down as u8 == key as u8 && up as u8 == key as u8)
    }
}

static WATCH: SpinLock<Watch> = SpinLock::new(Watch {
    pressed: None,
    released: None,
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
