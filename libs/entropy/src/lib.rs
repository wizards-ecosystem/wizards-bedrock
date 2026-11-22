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
const RECENT_WORDS: usize = (READY_BITS / SEED_WORD_BITS) as usize;
const RESEED_INTERVAL_NS: u64 = 60_000_000_000;
const ZERO_NONCE: [u8; 12] = [0; 12];
// Keeps one keystream far below the 2^32 block counter limit.
const MAX_STREAM: usize = 1 << 20;

// The kernel passes a volatile wipe, which this crate cannot write without unsafe.
pub type Wipe = fn(&mut [u8]);

struct Secret<const N: usize> {
    bytes: [u8; N],
    wipe: Wipe,
}

impl<const N: usize> Secret<N> {
    const fn zero(wipe: Wipe) -> Self {
        Self {
            bytes: [0; N],
            wipe,
        }
    }
}

impl<const N: usize> Drop for Secret<N> {
    fn drop(&mut self) {
        (self.wipe)(&mut self.bytes);
    }
}

// Working state lives in bytes a `Secret` owns, read and written a little-endian word at a time.
fn word(bytes: &[u8], at: usize) -> u32 {
    let at = at * 4;
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn set_word(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at * 4..at * 4 + 4].copy_from_slice(&value.to_le_bytes());
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

    // The pool restarts empty, so nothing it keeps reaches the key this digest makes.
    fn extract(&mut self, digest: &mut Secret<32>) {
        mem::replace(&mut self.hash, Blake2s::new(digest.wipe)).finalize_into(&mut digest.bytes);
        self.credited = 0;
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
    key: Secret<32>,
    pool: Pool,
    // Words credited since the last reseed; zero, which is never credited, marks a free slot.
    recent: Secret<{ RECENT_WORDS * 8 }>,
    next_recent: usize,
    generation: u64,
    reseeded_at_ns: u64,
}

impl fmt::Debug for Base {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Base(<key withheld>)")
    }
}

impl Base {
    #[must_use]
    pub fn new(wipe: Wipe) -> Self {
        Self {
            key: Secret::zero(wipe),
            pool: Pool {
                hash: Blake2s::new(wipe),
                credited: 0,
            },
            recent: Secret::zero(wipe),
            next_recent: 0,
            generation: 0,
            reseeded_at_ns: 0,
        }
    }

    #[must_use]
    pub fn add_seed_word(&mut self, word: u64) -> bool {
        let mut bytes = Secret::<8>::zero(self.recent.wipe);
        bytes.bytes = word.to_le_bytes();
        let repeated = self
            .recent
            .bytes
            .chunks_exact(8)
            .any(|seen| seen == bytes.bytes);
        if repeated || word == 0 || word == u64::MAX {
            return false;
        }
        let at = self.next_recent * 8;
        self.recent.bytes[at..at + 8].copy_from_slice(&bytes.bytes);
        self.next_recent = (self.next_recent + 1) % RECENT_WORDS;
        self.pool.absorb(&bytes.bytes, SEED_WORD_BITS);
        true
    }

    pub fn mix_sample(&mut self, sample: u64) {
        self.pool.absorb(&sample.to_le_bytes(), 0);
    }

    #[must_use]
    pub fn reseed(&mut self, now_ns: u64) -> bool {
        if !self.is_due(now_ns) || self.pool.credited < READY_BITS {
            return false;
        }
        let mut digest = Secret::zero(self.key.wipe);
        self.pool.extract(&mut digest);
        let mut next = Blake2s::new(self.key.wipe);
        next.update(&self.key.bytes);
        next.update(&digest.bytes);
        next.finalize_into(&mut self.key.bytes);
        (self.recent.wipe)(&mut self.recent.bytes);
        self.next_recent = 0;
        self.generation += 1;
        self.reseeded_at_ns = now_ns;
        true
    }

    #[must_use]
    pub const fn is_due(&self, now_ns: u64) -> bool {
        self.generation == 0 || now_ns.saturating_sub(self.reseeded_at_ns) >= RESEED_INTERVAL_NS
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.generation != 0
    }

    fn rekey(&mut self, local: &mut Secret<32>) {
        let mut block = Secret::<64>::zero(self.key.wipe);
        chacha::block(&self.key.bytes, 0, &ZERO_NONCE, &mut block.bytes);
        let (next, tail) = block.bytes.split_at(32);
        self.key.bytes.copy_from_slice(next);
        local.bytes.copy_from_slice(tail);
    }
}

pub struct Local {
    key: Secret<32>,
    generation: u64,
}

impl fmt::Debug for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Local(<key withheld>)")
    }
}

impl Local {
    #[must_use]
    pub const fn new(wipe: Wipe) -> Self {
        Self {
            key: Secret::zero(wipe),
            generation: 0,
        }
    }

    pub fn fill(&mut self, base: &mut Base, out: &mut [u8]) -> Result<(), NotReady> {
        if !base.is_ready() {
            return Err(NotReady);
        }
        if self.generation != base.generation {
            base.rekey(&mut self.key);
            self.generation = base.generation;
        }
        for chunk in out.chunks_mut(MAX_STREAM) {
            fill_erasing(&mut self.key, chunk);
        }
        Ok(())
    }
}

fn fill_erasing(key: &mut Secret<32>, out: &mut [u8]) {
    let mut block = Secret::<64>::zero(key.wipe);
    chacha::block(&key.bytes, 0, &ZERO_NONCE, &mut block.bytes);
    let (next, tail) = block.bytes.split_at(32);
    key.bytes.copy_from_slice(next);
    if let Some(bytes) = tail.get(..out.len()) {
        out.copy_from_slice(bytes);
    } else {
        let mut one_shot = Secret::<32>::zero(key.wipe);
        one_shot.bytes.copy_from_slice(tail);
        chacha::keystream(&one_shot.bytes, out, key.wipe);
    }
}

#[must_use]
pub fn derive(world_key: &[u8; 32], domain: &[u8], index: u64, wipe: Wipe) -> [u8; 32] {
    let mut hash = Blake2s::keyed(world_key, wipe);
    hash.update(&index.to_le_bytes());
    hash.update(domain);
    let mut out = [0; 32];
    hash.finalize_into(&mut out);
    out
}
