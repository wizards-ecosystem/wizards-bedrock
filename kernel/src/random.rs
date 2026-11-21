use core::fmt;

use entropy::{Base, Local, NotReady};
use wost::arch::current::cpu::{self, SeedReader, SeedSource};
use wost::mm;
use wost::sync::{Leaf, SpinLock};

use crate::time;

// Sixteen times the four healthy words a first seeding needs.
const SEED_WORDS: u32 = 64;

// The kernel's own `Local` draws world keys; each processor gets one once there are several.
#[derive(Debug)]
struct Generator {
    base: Base,
    local: Local,
    reader: SeedReader,
}

static GENERATOR: SpinLock<Leaf, Option<Generator>> = SpinLock::new(None);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SeedError {
    NoSource,
    Starved(SeedSource),
}

impl fmt::Display for SeedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSource => formatter.write_str("the processor offers neither rdseed nor rdrand"),
            Self::Starved(source) => write!(
                formatter,
                "{source} gave fewer than 256 healthy bits in {SEED_WORDS} words"
            ),
        }
    }
}

impl core::error::Error for SeedError {}

// Built in place, so no copy of the generator's state is left behind on a stack.
pub(crate) fn seed() -> Result<SeedSource, SeedError> {
    let reader = SeedReader::detect().ok_or(SeedError::NoSource)?;
    let sample = cpu::timestamp();
    GENERATOR
        .lock_leaf()
        .insert(Generator {
            base: Base::new(mm::wipe),
            local: Local::new(mm::wipe),
            reader,
        })
        .base
        .mix_sample(sample);
    let now_ns = time::now_ns();
    if (0..SEED_WORDS).any(|_| reader.read().is_some_and(|word| credit(word, now_ns))) {
        Ok(reader.source())
    } else {
        Err(SeedError::Starved(reader.source()))
    }
}

// A failed read ends the attempt and keeps the current key.
pub(crate) fn reseed_when_due() {
    let now_ns = time::now_ns();
    let Some(reader) = GENERATOR
        .lock_leaf()
        .as_ref()
        .filter(|generator| generator.base.is_due(now_ns))
        .map(|generator| generator.reader)
    else {
        return;
    };
    for word in (0..SEED_WORDS).map_while(|_| reader.read()) {
        if credit(word, now_ns) {
            return;
        }
    }
}

fn credit(word: u64, now_ns: u64) -> bool {
    GENERATOR.lock_leaf().as_mut().is_some_and(|generator| {
        generator.base.add_seed_word(word) && generator.base.reseed(now_ns)
    })
}

pub(crate) fn is_ready() -> bool {
    GENERATOR
        .lock_leaf()
        .as_ref()
        .is_some_and(|generator| generator.base.is_ready())
}

// Pure computation under the leaf guard: the keys never leave it.
pub(crate) fn fill(out: &mut [u8]) -> Result<(), NotReady> {
    let mut guard = GENERATOR.lock_leaf();
    let generator = guard.as_mut().ok_or(NotReady)?;
    generator.local.fill(&mut generator.base, out)
}

// A world's own stream, keyed from `Base` on its first draw and again after every reseed.
pub(crate) fn fill_stream(stream: &mut Local, out: &mut [u8]) -> Result<(), NotReady> {
    let mut guard = GENERATOR.lock_leaf();
    let generator = guard.as_mut().ok_or(NotReady)?;
    stream.fill(&mut generator.base, out)
}
