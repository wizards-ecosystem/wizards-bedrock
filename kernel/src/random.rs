use core::fmt;

use entropy::{Base, Local, NotReady};
use wost::arch::current::cpu::{self, SeedReader, SeedSource};
use wost::sync::{Leaf, SpinLock};

use crate::time;

// Sixteen times the four healthy words a first seeding needs.
const SEED_WORDS: u32 = 64;

// One `Local` today; each processor gets its own when there is more than one.
#[derive(Debug)]
struct Generator {
    base: Base,
    local: Local,
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

pub(crate) fn seed() -> Result<SeedSource, SeedError> {
    let reader = SeedReader::detect().ok_or(SeedError::NoSource)?;
    let mut base = Base::default();
    base.mix_sample(cpu::timestamp());
    let seeded = (0..SEED_WORDS).any(|_| {
        reader.read().is_some_and(|word| base.add_seed_word(word)) && base.reseed(time::now_ns())
    });
    if !seeded {
        return Err(SeedError::Starved(reader.source()));
    }
    *GENERATOR.lock_leaf() = Some(Generator {
        base,
        local: Local::default(),
    });
    Ok(reader.source())
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
