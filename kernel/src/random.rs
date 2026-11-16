use core::fmt;

use entropy::Base;
use wost::arch::current::cpu::{self, SeedReader, SeedSource};
use wost::sync::SpinLock;

use crate::time;

// Sixteen times the four healthy words a first seeding needs.
const SEED_WORDS: u32 = 64;

static BASE: SpinLock<Option<Base>> = SpinLock::new(None);

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
    *BASE.lock_leaf() = Some(base);
    Ok(reader.source())
}

pub(crate) fn is_ready() -> bool {
    BASE.lock_leaf().as_ref().is_some_and(Base::is_ready)
}
