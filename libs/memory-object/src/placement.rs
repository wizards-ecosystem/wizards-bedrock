use core::fmt;

use crate::records::align_up;
use crate::{Extent, Records};

pub const PLACEMENT_SAMPLES: u32 = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    pub first: u64,
    pub counter: u64,
}

// `samples[i]` is the caller's random word for counter value `counter + i`; each sample tried
// consumes one, so a stored counter replays the same placements.
pub fn place<T: Copy>(
    records: &Records<T>,
    samples: &[u64],
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
    let mut used = counter;
    let mut sample = 0;
    while sample < samples.len() && used < u64::MAX {
        used += 1;
        let first = lowest + (samples[sample] % starts) * align;
        if let Some(extent) = Extent::new(first, pages)
            && records.is_free(extent)
        {
            return Ok(Placed {
                first,
                counter: used,
            });
        }
        sample += 1;
    }
    match records.first_fit(range, pages, align) {
        Some(first) => Ok(Placed {
            first,
            counter: used,
        }),
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
