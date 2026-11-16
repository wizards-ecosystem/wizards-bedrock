use memory_object::{Extent, PLACEMENT_SAMPLES, Records, place};
use x86_paging::{PAGE_SIZE, USER_END};

// Kernel placements stay above the first 4 GiB, where position-dependent images link.
const PLACEMENT_START: u64 = 0x0000_0001_0000_0000;

// Seeded addresses for the pages the kernel maps into a space; each placement is recorded so no
// two overlap, and the counter is stored so the same seed places the same way again.
#[derive(Debug)]
pub(crate) struct Placer {
    seed: [u8; 32],
    counter: u64,
    placed: Records<()>,
}

impl Placer {
    pub(crate) const fn new(seed: [u8; 32]) -> Self {
        Self {
            seed,
            counter: 0,
            placed: Records::EMPTY,
        }
    }

    // The address of the first of `pages` free pages, the first `guard` of them left unmapped.
    pub(crate) fn place(&mut self, pages: u64, guard: u64) -> Option<u64> {
        let range = Extent::new(
            PLACEMENT_START / PAGE_SIZE,
            (USER_END - PLACEMENT_START) / PAGE_SIZE,
        )?;
        let total = pages.checked_add(guard)?;
        let mut samples = [0u64; PLACEMENT_SAMPLES as usize];
        for (index, sample) in (0u64..).zip(samples.iter_mut()) {
            let index = self.counter.wrapping_add(index);
            let [a, b, c, d, e, f, g, h, ..] =
                entropy::derive(&self.seed, b"placement", index, wost::mm::wipe);
            *sample = u64::from_le_bytes([a, b, c, d, e, f, g, h]);
        }
        let placed = place(&self.placed, &samples, self.counter, range, total, 0).ok()?;
        self.placed
            .insert(Extent::new(placed.first, total)?, ())
            .ok()?;
        self.counter = placed.counter;
        Some((placed.first + guard) * PAGE_SIZE)
    }
}
