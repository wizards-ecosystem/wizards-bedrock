//! Heap and frame-pool ownership accounting.

/// Counts that determine whether an allocator preserved ownership.
///
/// The receiver in [`is_preserved_since`](Self::is_preserved_since) is the
/// later snapshot. Live allocations must return exactly to their earlier
/// count. Slab frames may increase because the heap retains empty slabs; large
/// frames may move when a still-live allocation changes capacity. Every heap
/// frame must leave the free pool so the total physical account is conserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocatorOwnership {
    free_frames: usize,
    slab_frames: usize,
    large_frames: usize,
    live_allocations: usize,
}

impl AllocatorOwnership {
    /// Records the free-pool and heap ownership counts at one instant.
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

    /// Whether this later snapshot preserves the ownership in `before`.
    pub fn is_preserved_since(self, before: Self) -> bool {
        self.live_allocations == before.live_allocations
            && self.slab_frames >= before.slab_frames
            && self
                .accounted_physical_frames()
                .is_some_and(|accounted| before.accounted_physical_frames() == Some(accounted))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ownership(
        free_frames: usize,
        slab_frames: usize,
        large_frames: usize,
        live_allocations: usize,
    ) -> AllocatorOwnership {
        AllocatorOwnership::new(free_frames, slab_frames, large_frames, live_allocations)
    }

    #[test]
    fn every_small_free_and_slab_account_obeys_conservation() {
        const BOUND: usize = 4;

        for before_free in 0..=BOUND {
            for before_slabs in 0..=BOUND {
                let before = ownership(before_free, before_slabs, 2, 3);
                for after_free in 0..=BOUND {
                    for after_slabs in 0..=BOUND {
                        let after = ownership(after_free, after_slabs, 2, 3);
                        let conserved = before_free + before_slabs == after_free + after_slabs;
                        let expected = conserved && after_slabs >= before_slabs;
                        assert_eq!(
                            after.is_preserved_since(before),
                            expected,
                            "free {before_free}->{after_free}, slabs {before_slabs}->{after_slabs}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_boundary_account_fails_closed_on_overflow() {
        const COUNTS: [usize; 5] = [0, 1, 2, usize::MAX - 1, usize::MAX];

        for before_free in COUNTS {
            for before_slabs in COUNTS {
                for before_large in COUNTS {
                    let before = ownership(before_free, before_slabs, before_large, 0);
                    let before_mathematical =
                        before_free as u128 + before_slabs as u128 + before_large as u128;
                    let before_account = usize::try_from(before_mathematical).ok();
                    assert_eq!(
                        before.accounted_physical_frames(),
                        before_account,
                        "free {before_free}, slabs {before_slabs}, large {before_large}"
                    );
                    assert_eq!(
                        before.is_preserved_since(before),
                        before_account.is_some(),
                        "overflowing snapshot accepted itself"
                    );

                    for after_free in COUNTS {
                        for after_slabs in COUNTS {
                            for after_large in COUNTS {
                                let after = ownership(after_free, after_slabs, after_large, 0);
                                let after_mathematical =
                                    after_free as u128 + after_slabs as u128 + after_large as u128;
                                let expected = before_account.is_some()
                                    && before_mathematical == after_mathematical
                                    && after_slabs >= before_slabs;
                                assert_eq!(
                                    after.is_preserved_since(before),
                                    expected,
                                    "free {before_free}->{after_free}, slabs {before_slabs}->{after_slabs}, large {before_large}->{after_large}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn live_count_is_exact_while_large_frame_ownership_may_move() {
        const BOUND: usize = 4;
        const PHYSICAL_BUDGET: usize = 12;

        for before_live in 0..=BOUND {
            for after_live in 0..=BOUND {
                for before_large in 0..=BOUND {
                    for after_large in 0..=BOUND {
                        let before =
                            ownership(PHYSICAL_BUDGET - before_large, 0, before_large, before_live);
                        let after =
                            ownership(PHYSICAL_BUDGET - after_large, 0, after_large, after_live);
                        assert_eq!(
                            after.is_preserved_since(before),
                            before_live == after_live,
                            "live {before_live}->{after_live}, large {before_large}->{after_large}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_conserved_slab_transition_is_directionally_monotonic() {
        const BOUND: usize = 8;
        const PHYSICAL_BUDGET: usize = 16;

        for before_slabs in 0..=BOUND {
            for after_slabs in 0..=BOUND {
                let before = ownership(PHYSICAL_BUDGET - before_slabs, before_slabs, 0, 1);
                let after = ownership(PHYSICAL_BUDGET - after_slabs, after_slabs, 0, 1);
                assert_eq!(
                    after.is_preserved_since(before),
                    after_slabs >= before_slabs,
                    "slabs {before_slabs}->{after_slabs}"
                );
            }
        }
    }
}
