use core::fmt;

use crate::records::align_up;
use crate::{Extent, Records};

pub const PLACEMENT_SAMPLES: u32 = 8;

const DOMAIN: &[u8] = b"placement";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    pub first: u64,
    pub counter: u64,
}

// Each sample consumes one counter value, so a stored counter replays the same placements.
pub fn place<T: Copy>(
    records: &Records<T>,
    seed: &[u8; 32],
    counter: u64,
    range: Extent,
    pages: u64,
    align_log2: u8,
) -> Result<Placed, NoAddress> {
    if u32::from(align_log2) >= u64::BITS {
        return Err(NoAddress);
    }
    let align = 1u64 << align_log2;
    let Some(lowest) = align_up(range.first(), align) else {
        return Err(NoAddress);
    };
    let Some(highest) = range.end().checked_sub(pages) else {
        return Err(NoAddress);
    };
    if pages == 0 || lowest > highest {
        return Err(NoAddress);
    }
    let starts = (highest - lowest) / align + 1;
    let mut counter = counter;
    let mut sample = 0;
    while sample < PLACEMENT_SAMPLES {
        let Some(next) = counter.checked_add(1) else {
            break;
        };
        let drawn = entropy::derive(seed, DOMAIN, counter);
        counter = next;
        let mut word = [0u8; 8];
        let mut at = 0;
        while at < word.len() {
            word[at] = drawn[at];
            at += 1;
        }
        let first = lowest + (u64::from_le_bytes(word) % starts) * align;
        if let Some(extent) = Extent::new(first, pages)
            && records.is_free(extent)
        {
            return Ok(Placed { first, counter });
        }
        sample += 1;
    }
    match records.first_fit(range, pages, align) {
        Some(first) => Ok(Placed { first, counter }),
        None => Err(NoAddress),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NoAddress;

impl fmt::Display for NoAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("no aligned gap in the placement range fits the pages")
    }
}

impl core::error::Error for NoAddress {}
