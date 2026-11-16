#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod blake2s;
mod chacha;
#[cfg(test)]
mod tests;

use blake2s::Blake2s;
use core::fmt;
use core::mem;

const READY_BITS: u32 = 256;
const SEED_WORD_BITS: u32 = 64;
const RESEED_INTERVAL_NS: u64 = 60_000_000_000;
const ZERO_NONCE: [u8; 12] = [0; 12];
// Keeps one keystream far below the 2^32 block counter limit.
const MAX_STREAM: usize = 1 << 20;

#[derive(Default)]
struct Key([u8; 32]);

impl Drop for Key {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

struct Pool {
    hash: Blake2s,
    credited: u32,
}

impl Pool {
    fn absorb(&mut self, input: &[u8], bits: u32) {
        self.hash.update(input);
        self.credited = self.credited.saturating_add(bits);
    }

    fn extract(&mut self) -> [u8; 32] {
        let digest = mem::replace(&mut self.hash, Blake2s::new()).finalize();
        self.hash.update(&digest);
        self.credited = 0;
        digest
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotReady;

impl fmt::Display for NotReady {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the generator has not absorbed 256 credited bits")
    }
}

impl core::error::Error for NotReady {}

pub struct Base {
    key: Key,
    pool: Pool,
    previous_word: Option<u64>,
    generation: u64,
    reseeded_at_ns: u64,
}

impl Default for Base {
    fn default() -> Self {
        Self {
            key: Key::default(),
            pool: Pool {
                hash: Blake2s::new(),
                credited: 0,
            },
            previous_word: None,
            generation: 0,
            reseeded_at_ns: 0,
        }
    }
}

impl fmt::Debug for Base {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Base(<key withheld>)")
    }
}

impl Base {
    #[must_use]
    pub fn add_seed_word(&mut self, word: u64) -> bool {
        let repeated = self.previous_word == Some(word);
        self.previous_word = Some(word);
        if repeated || word == 0 || word == u64::MAX {
            return false;
        }
        self.pool.absorb(&word.to_le_bytes(), SEED_WORD_BITS);
        true
    }

    pub fn mix_sample(&mut self, sample: u64) {
        self.pool.absorb(&sample.to_le_bytes(), 0);
    }

    #[must_use]
    pub fn reseed(&mut self, now_ns: u64) -> bool {
        let first = self.generation == 0;
        let due = first || now_ns.saturating_sub(self.reseeded_at_ns) >= RESEED_INTERVAL_NS;
        if !due || self.pool.credited < READY_BITS {
            return false;
        }
        let mut digest = self.pool.extract();
        let mut next = Blake2s::new();
        next.update(&self.key.0);
        next.update(&digest);
        self.key.0 = next.finalize();
        digest.fill(0);
        self.generation += 1;
        self.reseeded_at_ns = now_ns;
        true
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.generation != 0
    }

    fn take_local_key(&mut self) -> Key {
        let mut block = chacha::block(&self.key.0, 0, &ZERO_NONCE);
        let mut local = Key::default();
        self.key.0.copy_from_slice(&block[..32]);
        local.0.copy_from_slice(&block[32..]);
        block.fill(0);
        local
    }
}

#[derive(Default)]
pub struct Local {
    key: Key,
    generation: u64,
}

impl fmt::Debug for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Local(<key withheld>)")
    }
}

impl Local {
    pub fn fill(&mut self, base: &mut Base, out: &mut [u8]) -> Result<(), NotReady> {
        if !base.is_ready() {
            return Err(NotReady);
        }
        if self.generation != base.generation {
            self.key = base.take_local_key();
            self.generation = base.generation;
        }
        for chunk in out.chunks_mut(MAX_STREAM) {
            fill_erasing(&mut self.key, chunk);
        }
        Ok(())
    }
}

fn fill_erasing(key: &mut Key, out: &mut [u8]) {
    let mut block = chacha::block(&key.0, 0, &ZERO_NONCE);
    let (next, tail) = block.split_at(32);
    key.0.copy_from_slice(next);
    if let Some(bytes) = tail.get(..out.len()) {
        out.copy_from_slice(bytes);
    } else {
        let mut one_shot = [0u8; 32];
        one_shot.copy_from_slice(tail);
        chacha::keystream(&one_shot, out);
        one_shot.fill(0);
    }
    block.fill(0);
}

#[must_use]
pub fn derive(world_key: &[u8; 32], domain: &[u8], index: u64) -> [u8; 32] {
    let mut hash = Blake2s::keyed(world_key);
    hash.update(&index.to_le_bytes());
    hash.update(domain);
    hash.finalize()
}
