#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{verus, verus_verify};

use crate::records::align_up;
use crate::{Extent, Records};

#[verus_verify]
pub const PLACEMENT_SAMPLES: u32 = 8;

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placed {
    pub first: u64,
    pub counter: u64,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NoAddress;

verus! {

// `samples[i]` is the caller's random word for counter value `counter + i`; each sample tried
// consumes one, so a stored counter replays the same placements.
#[verifier::loop_isolation(false)]
pub fn place<T: Copy>(
    records: &Records<T>,
    samples: &[u64],
    counter: u64,
    range: Extent,
    pages: u64,
    align_log2: u8,
) -> (r: Result<Placed, NoAddress>)
    ensures
        match r {
            Ok(placed) => crate::proof::placed_free(
                records.spec_records(),
                range,
                pages,
                1u64 << align_log2,
                placed.first,
            ) && counter <= placed.counter <= counter + samples.len(),
            Err(_) => true,
        },
{
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
    proof {
        vstd::arithmetic::div_mod::lemma_div_nonincreasing((highest - lowest) as int, align as int);
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod(
            (highest - lowest) as int,
            align as int,
        );
        vstd::arithmetic::div_mod::lemma_mod_bound((highest - lowest) as int, align as int);
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod(lowest as int, align as int);
    }
    let starts = (highest - lowest) / align + 1;
    let mut used = counter;
    let mut sample = 0;
    while sample < samples.len() && used < u64::MAX
        invariant
            sample <= samples.len(),
            used == counter + sample,
        decreases samples.len() - sample,
    {
        used += 1;
        let pick = samples[sample] % starts;
        proof {
            let size = align as int;
            let gap = (highest - lowest) as int;
            let span = gap / size;
            vstd::arithmetic::div_mod::lemma_mod_bound(
                samples@[sample as int] as int,
                starts as int,
            );
            vstd::arithmetic::mul::lemma_mul_inequality(pick as int, span, size);
            assert(span * size <= gap) by (nonlinear_arith)
                requires
                    gap == size * span + gap % size,
                    0 <= gap % size,
            ;
            let base = lowest as int / size;
            assert(lowest as int + pick as int * size == (base + pick as int) * size)
                by (nonlinear_arith)
                requires
                    lowest as int == size * base + lowest as int % size,
                    lowest as int % size == 0,
            ;
            vstd::arithmetic::div_mod::lemma_mod_multiples_basic(base + pick as int, size);
        }
        let first = lowest + pick * align;
        let Some(extent) = Extent::new(first, pages) else {
            proof {
                assert(false);
            }
            return Err(NoAddress);
        };
        if records.is_free(extent) {
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

} // verus!
