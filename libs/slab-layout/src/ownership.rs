//! Heap and frame-pool ownership accounting.

/// Free-pool and heap ownership counts at one instant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AllocatorOwnership {
    free_frames: usize,
    slab_frames: usize,
    large_frames: usize,
    live_allocations: usize,
}

impl AllocatorOwnership {
    pub const fn new(
        free_frames: usize,
        slab_frames: usize,
        large_frames: usize,
        live_allocations: usize,
    ) -> Self {
        Self {
            free_frames,
            slab_frames,
            large_frames,
            live_allocations,
        }
    }

    /// Free and heap-owned physical frames, or `None` if the sum overflows.
    pub fn accounted_physical_frames(self) -> Option<usize> {
        self.free_frames
            .checked_add(self.slab_frames)?
            .checked_add(self.large_frames)
    }

    /// Whether this later snapshot keeps `before`'s live count and physical total; slabs may grow.
    pub fn is_preserved_since(self, before: Self) -> bool {
        self.live_allocations == before.live_allocations
            && self.slab_frames >= before.slab_frames
            && self
                .accounted_physical_frames()
                .is_some_and(|accounted| before.accounted_physical_frames() == Some(accounted))
    }
}

#[cfg(test)]
mod tests;
