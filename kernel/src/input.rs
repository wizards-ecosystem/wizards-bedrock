// Nothing here draws or logs: a producer may be a trap handler, and a glyph can cost a scroll.

use boot_record::{Fact, Record};
use ps2_keys::{Arriving, Decoder, KeyEvent, KeyState, SetProbe, Shift, us};
use wost::arch::current::interrupt;
use wost::sync::{Leaf, SpinLock};

use crate::arch::current::i8042::Controller;
use crate::thread::ThreadContext;
use crate::wait::WaitQueue;
use crate::{info, warn};

#[derive(Debug)]
struct Keyboard {
    controller: Controller,
    decoder: Decoder,
    // Every probe the controller answers lies on some machine, so the bytes decide the set.
    settled: bool,
    held: [u8; HELD_BYTES],
    held_len: usize,
    held_at: usize,
    shift: Shift,
}

// A keystroke decides within three bytes and set 2's longest sequence within five.
const HELD_BYTES: usize = 16;

impl Keyboard {
    fn next_event(&mut self) -> Option<KeyEvent> {
        // Stops a controller streaming bytes that never complete an event from pinning the lock.
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

    fn adopt(&mut self, arriving: Arriving) {
        let translated = match arriving {
            Arriving::Undecided => return,
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
        self.settled = true;
    }
}

static KEYBOARD: SpinLock<Leaf, Option<Keyboard>> = SpinLock::new(None);

// The character is resolved at decode time, since a reader may look milliseconds later.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Keystroke {
    pub(crate) event: KeyEvent,
    pub(crate) character: Option<char>,
}

// With the byte budget, no input stream can pin the keyboard lock for its whole length.
const BATCH: usize = 8;

// The identity is only the opening guess: it and the configuration byte each lie somewhere.
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

// The wake comes last, outside every other lock: the woken reader will want the queue.
pub(crate) fn poll() -> usize {
    // Two orders of magnitude past human typing, and finite either way.
    const BATCHES: usize = 8;

    let mut delivered = 0;
    for _ in 0..BATCHES {
        let (count, admitted) = produce_batch();
        delivered += admitted;
        if count < BATCH {
            break;
        }
    }
    if delivered > 0 { READERS.wake_all() } else { 0 }
}

// Interrupts stay off so a second producer cannot enqueue a later batch ahead of this one.
fn produce_batch() -> (usize, usize) {
    let exclusion = interrupt::disable();
    let mut keystrokes = [None; BATCH];
    let count = drain_batch(&mut keystrokes);
    let mut delivered = 0;
    for keystroke in keystrokes.iter().flatten() {
        if deliver(*keystroke) {
            delivered += 1;
        }
    }
    exclusion.restore();
    (count, delivered)
}

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

fn deliver(keystroke: Keystroke) -> bool {
    EVENTS.lock_leaf().push(keystroke)
}

// A fixed ring: a producer may be a trap handler, where the heap is forbidden.
const QUEUED: usize = 16;

#[derive(Debug)]
struct Events {
    ring: [Option<Keystroke>; QUEUED],
    head: usize,
    tail: usize,
}

impl Events {
    const fn new() -> Self {
        Self {
            ring: [None; QUEUED],
            head: 0,
            tail: 0,
        }
    }

    fn push(&mut self, keystroke: Keystroke) -> bool {
        if self.ring[self.head].is_some() {
            return false;
        }
        self.ring[self.head] = Some(keystroke);
        self.head = (self.head + 1) % QUEUED;
        true
    }

    fn has_any(&self) -> bool {
        self.ring[self.tail].is_some()
    }

    fn pop(&mut self) -> Option<Keystroke> {
        let keystroke = self.ring[self.tail].take()?;
        self.tail = (self.tail + 1) % QUEUED;
        Some(keystroke)
    }
}

static EVENTS: SpinLock<Leaf, Events> = SpinLock::new(Events::new());

static READERS: WaitQueue = WaitQueue::new();

// The emptiness check sits under the waiters lock a producer's wake takes, so no wake is lost.
pub(crate) fn read_key(context: &mut ThreadContext) -> Keystroke {
    loop {
        // Its own statement, so the queue guard drops before the wait reads the queue.
        let keystroke = EVENTS.lock(context).pop();
        if let Some(keystroke) = keystroke {
            return keystroke;
        }
        READERS.wait_unless(context, || EVENTS.lock_leaf().has_any());
    }
}

pub(crate) fn start_poller(context: &mut ThreadContext) {
    match crate::thread::spawn(context, "keyboard-poll", poll_forever, 0) {
        Ok(_) if crate::time::is_calibrated() && interrupt::are_enabled() => {
            info!("keyboard: polled every {POLL_INTERVAL_MS} ms");
            info!("{}", Record::Fact(Fact::KeyboardPolled));
        }
        Ok(_) => info!("keyboard: clockless poller yields between bounded drains"),
        Err(reason) => warn!("keyboard: no poller: {reason}; input is lost"),
    }
}

// The poller owns its interval: no periodic tick wakes it. It also carries the generator's reseed.
const POLL_INTERVAL_MS: u64 = 10;

fn poll_forever(context: &mut ThreadContext, _: usize) {
    loop {
        poll();
        crate::random::reseed_when_due();
        if crate::time::is_calibrated() && interrupt::are_enabled() {
            crate::thread::sleep_ms(context, POLL_INTERVAL_MS);
        } else {
            crate::thread::yield_now(context);
        }
    }
}
