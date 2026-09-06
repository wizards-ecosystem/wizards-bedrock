//! Property tests over the map's stated contracts.
//!
//! Unit tests pin hand-checkable examples; these properties quantify the
//! documented contracts over arbitrary, overlapping region sets (D-241).
//!
//! Every property here cites the sentence it was transcribed from, which is
//! D-242's admissibility rule: a check whose text could be recovered by reading
//! the function body would be a restatement of the implementation.
//!
//! Limine promises a narrower non-overlapping input, but this tier checks the
//! crate's public claims rather than one caller's stronger precondition.

use memory_map::{DirectMapCache, MAX_REGIONS, MemoryKind, MemoryMap, MemoryRegion};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRunner};

/// Bytes in one page, as `first_memory_run` aligns to.
const PAGE: u64 = 4096;

fn kind() -> impl Strategy<Value = MemoryKind> {
    prop_oneof![
        Just(MemoryKind::Usable),
        Just(MemoryKind::BootloaderReclaimable),
        Just(MemoryKind::KernelAndModules),
        Just(MemoryKind::Framebuffer),
        Just(MemoryKind::AcpiReclaimable),
        Just(MemoryKind::AcpiNvs),
        Just(MemoryKind::Reserved),
        Just(MemoryKind::BadMemory),
    ]
}

/// Small bases and lengths, so generated regions overlap one another. Spread
/// over the whole address space they would never share a cursor, and the
/// contradictions D-241 repaired live exactly where they do.
fn region() -> impl Strategy<Value = MemoryRegion> {
    (0u64..0x8000, 0u64..0x8000, kind()).prop_map(|(base, len, kind)| MemoryRegion {
        base,
        len,
        kind,
    })
}

fn regions() -> impl Strategy<Value = Vec<MemoryRegion>> {
    prop::collection::vec(region(), 1..24)
}

fn every_byte_agrees(
    regions: &[MemoryRegion],
    start: u64,
    end: u64,
    allowed: impl Fn(MemoryKind) -> bool,
) -> bool {
    (start..end).all(|byte| {
        let claims = regions.iter().filter(|region| {
            let region_end = region
                .base
                .checked_add(region.len)
                .expect("the map accepted this region");
            region.base <= byte && byte < region_end
        });
        let mut described = false;
        for region in claims {
            described = true;
            if !allowed(region.kind) {
                return false;
            }
        }
        described
    })
}

/// A bytewise oracle cannot skip a claim nested between interval endpoints.
#[test]
fn coverage_matches_every_byte_claim_in_a_bounded_space() {
    const LIMIT: u64 = 6;
    let kinds = [MemoryKind::Usable, MemoryKind::Reserved];

    for first_base in 0..=LIMIT {
        for first_end in first_base..=LIMIT {
            for first_kind in kinds {
                let first = MemoryRegion {
                    base: first_base,
                    len: first_end - first_base,
                    kind: first_kind,
                };
                for second_base in 0..=LIMIT {
                    for second_end in second_base..=LIMIT {
                        for second_kind in kinds {
                            let second = MemoryRegion {
                                base: second_base,
                                len: second_end - second_base,
                                kind: second_kind,
                            };
                            let regions = [first, second];
                            let map = MemoryMap::try_from_regions(regions).expect("two regions");

                            for start in 0..=LIMIT {
                                for end in start..=LIMIT {
                                    assert_eq!(
                                        map.covers(start, end),
                                        every_byte_agrees(&regions, start, end, |_| true),
                                        "covers({start}, {end}) disagrees for {regions:?}"
                                    );
                                    assert_eq!(
                                        map.covers_memory(start, end),
                                        every_byte_agrees(&regions, start, end, |kind| {
                                            kind == MemoryKind::Usable
                                        }),
                                        "covers_memory({start}, {end}) disagrees for {regions:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn first_run_oracle(regions: &[MemoryRegion], pages: u64, limit: u64) -> Option<u64> {
    (1..limit).find_map(|candidate| {
        let end = candidate.checked_add(pages)?;
        if end > limit {
            return None;
        }
        (candidate..end)
            .all(|page| {
                let byte = page * PAGE;
                let mut described = false;
                for region in regions {
                    let region_end = region.base + region.len;
                    if region.base <= byte && byte < region_end {
                        described = true;
                        if !region.kind.is_direct_mapped() {
                            return false;
                        }
                    }
                }
                described
            })
            .then_some(candidate * PAGE)
    })
}

fn frame_is_exclusively_allocatable(regions: &[MemoryRegion], start: u64) -> bool {
    let end = start + PAGE;
    let mut boundaries = vec![start, end];
    for region in regions {
        let region_end = region.base + region.len;
        if start < region_end && region.base < end {
            boundaries.push(region.base.clamp(start, end));
            boundaries.push(region_end.clamp(start, end));
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    boundaries.windows(2).all(|window| {
        let sample = window[0];
        if sample == window[1] {
            return true;
        }
        let mut described = false;
        for region in regions {
            let region_end = region.base + region.len;
            if region.base <= sample && sample < region_end {
                described = true;
                if !region.kind.is_allocatable() {
                    return false;
                }
            }
        }
        described
    })
}

fn direct_page_policy(regions: &[MemoryRegion], start: u64) -> Option<(DirectMapCache, bool)> {
    let end = start + PAGE;
    let mut boundaries = vec![start, end];
    let mut framebuffer = false;
    for region in regions {
        let region_end = region.base + region.len;
        if region.len != 0 && start < region_end && region.base < end {
            boundaries.push(region.base.clamp(start, end));
            boundaries.push(region_end.clamp(start, end));
            framebuffer |= region.kind == MemoryKind::Framebuffer;
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    for window in boundaries.windows(2) {
        let sample = window[0];
        if sample == window[1] {
            continue;
        }
        let mut described = false;
        for region in regions {
            let region_end = region.base + region.len;
            if region.base <= sample && sample < region_end {
                described = true;
                if !region.kind.is_direct_mapped() {
                    return None;
                }
            }
        }
        if !described {
            return None;
        }
    }

    Some((
        if framebuffer {
            DirectMapCache::Framebuffer
        } else {
            DirectMapCache::Normal
        },
        frame_is_exclusively_allocatable(regions, start),
    ))
}

#[test]
fn every_small_page_run_is_complete_disjoint_and_order_independent() {
    const HALF: u64 = PAGE / 2;
    const ENDPOINTS: u64 = 6;
    const FRAMES: u64 = ENDPOINTS / 2;
    let kinds = [
        MemoryKind::Usable,
        MemoryKind::Framebuffer,
        MemoryKind::Reserved,
    ];

    for first_base in 0..=ENDPOINTS {
        for first_end in first_base..=ENDPOINTS {
            for first_kind in kinds {
                let first = MemoryRegion {
                    base: first_base * HALF,
                    len: (first_end - first_base) * HALF,
                    kind: first_kind,
                };
                for second_base in 0..=ENDPOINTS {
                    for second_end in second_base..=ENDPOINTS {
                        for second_kind in kinds {
                            let second = MemoryRegion {
                                base: second_base * HALF,
                                len: (second_end - second_base) * HALF,
                                kind: second_kind,
                            };
                            for regions in [[first, second], [second, first]] {
                                let map = MemoryMap::try_from_regions(regions)
                                    .expect("two bounded regions");
                                let runs = map.page_runs().collect::<Vec<_>>();
                                for pair in runs.windows(2) {
                                    assert!(
                                        pair[0].base() + pair[0].bytes() <= pair[1].base(),
                                        "normalized runs overlap for {regions:?}"
                                    );
                                    assert!(
                                        pair[0].base() + pair[0].bytes() < pair[1].base()
                                            || pair[0].cache() != pair[1].cache()
                                            || pair[0].is_allocatable() != pair[1].is_allocatable(),
                                        "adjacent equal-policy runs were not maximal for {regions:?}"
                                    );
                                }
                                for frame in 0..FRAMES {
                                    let start = frame * PAGE;
                                    let emitted = runs.iter().find_map(|run| {
                                        (run.base() <= start
                                            && start + PAGE <= run.base() + run.bytes())
                                        .then_some((run.cache(), run.is_allocatable()))
                                    });
                                    assert_eq!(
                                        emitted,
                                        direct_page_policy(&regions, start),
                                        "direct policy disagrees for frame {frame}, {regions:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn every_small_allocatable_run_arbitrates_overlap_and_partial_frames() {
    const HALF: u64 = PAGE / 2;
    const ENDPOINTS: u64 = 8;
    const FRAMES: u64 = ENDPOINTS / 2;
    let kinds = [MemoryKind::Usable, MemoryKind::Reserved];

    for first_base in 0..=ENDPOINTS {
        for first_end in first_base..=ENDPOINTS {
            for first_kind in kinds {
                let first = MemoryRegion {
                    base: first_base * HALF,
                    len: (first_end - first_base) * HALF,
                    kind: first_kind,
                };
                for second_base in 0..=ENDPOINTS {
                    for second_end in second_base..=ENDPOINTS {
                        for second_kind in kinds {
                            let second = MemoryRegion {
                                base: second_base * HALF,
                                len: (second_end - second_base) * HALF,
                                kind: second_kind,
                            };
                            for regions in [[first, second], [second, first]] {
                                let map = MemoryMap::try_from_regions(regions)
                                    .expect("two bounded regions");
                                let runs = map.allocatable_runs().collect::<Vec<_>>();
                                for frame in 0..FRAMES {
                                    let start = frame * PAGE;
                                    let emitted = runs.iter().any(|run| {
                                        run.base() <= start
                                            && start + PAGE <= run.base() + run.bytes()
                                    });
                                    assert_eq!(
                                        emitted,
                                        frame_is_exclusively_allocatable(&regions, start),
                                        "allocator policy disagrees for frame {frame}, {regions:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn every_small_first_memory_run_matches_an_independent_page_oracle() {
    const LIMIT: u64 = 6;
    let kinds = [MemoryKind::Usable, MemoryKind::Reserved];

    for first_base in 0..=LIMIT {
        for first_end in first_base..=LIMIT {
            for first_kind in kinds {
                let first = MemoryRegion {
                    base: first_base * PAGE,
                    len: (first_end - first_base) * PAGE,
                    kind: first_kind,
                };
                for second_base in 0..=LIMIT {
                    for second_end in second_base..=LIMIT {
                        for second_kind in kinds {
                            let second = MemoryRegion {
                                base: second_base * PAGE,
                                len: (second_end - second_base) * PAGE,
                                kind: second_kind,
                            };
                            let regions = [first, second];
                            let map = MemoryMap::try_from_regions(regions).expect("two regions");
                            for pages in 1..=3 {
                                assert_eq!(
                                    map.first_memory_run(pages * PAGE),
                                    first_run_oracle(&regions, pages, LIMIT),
                                    "first run disagrees for {regions:?}, pages={pages}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

proptest! {
    /// A run `first_memory_run` names is page-aligned, nonzero, and memory by
    /// the map's own other answer.
    ///
    /// Transcribed from "Find a page-aligned, nonzero memory run of at least
    /// `len` bytes". Two contracts about one address must agree even when a
    /// reserved region overlaps a direct-mapped one (D-241).
    #[test]
    fn a_found_run_is_aligned_nonzero_and_agrees_with_covers_memory(
        regions in regions(),
        len in 1u64..0x4000,
    ) {
        let Ok(map) = MemoryMap::try_from_regions(regions) else {
            return Ok(());
        };
        let Some(base) = map.first_memory_run(len) else {
            return Ok(());
        };
        prop_assert_eq!(base % PAGE, 0, "the run is not page-aligned");
        prop_assert_ne!(base, 0, "the run is the zero page");
        prop_assert!(
            map.covers_memory(base, base + len),
            "first_memory_run gave {:#x} for {:#x} bytes, which covers_memory denies",
            base,
            len
        );
    }

    /// Covering does not depend on the order the regions arrived in.
    ///
    /// Transcribed from "Input order is not assumed, so each bounded step
    /// searches the complete map". A walk that took the first region holding
    /// the cursor made this false whenever two regions shared a base, and the
    /// stride depended on the order too.
    #[test]
    fn covering_does_not_depend_on_the_order_regions_arrived(
        regions in regions(),
        start in 0u64..0x8000,
        span in 0u64..0x2000,
    ) {
        let mut reversed = regions.clone();
        reversed.reverse();
        let (Ok(forward), Ok(backward)) = (
            MemoryMap::try_from_regions(regions),
            MemoryMap::try_from_regions(reversed),
        ) else {
            return Ok(());
        };
        let end = start.saturating_add(span);
        prop_assert_eq!(
            forward.covers(start, end),
            backward.covers(start, end),
            "covers({:#x}, {:#x}) changed when the regions were reversed",
            start,
            end
        );
        prop_assert_eq!(
            forward.covers_memory(start, end),
            backward.covers_memory(start, end),
            "covers_memory({:#x}, {:#x}) changed when reversed",
            start,
            end
        );
    }

    #[test]
    fn a_region_is_accepted_exactly_when_its_end_is_representable(
        base in any::<u64>(),
        len in any::<u64>(),
    ) {
        let region = MemoryRegion { base, len, kind: MemoryKind::Reserved };
        prop_assert_eq!(
            MemoryMap::try_from_regions([region]).is_ok(),
            base.checked_add(len).is_some()
        );
    }

    /// A handoff is taken whole or refused whole, never as a prefix.
    ///
    /// Transcribed from the module doc: "A fixed bound is safe only when
    /// overflow refuses the whole handoff rather than returning a useful-looking
    /// prefix." D-224 established it on three hand-written cases; this asks it
    /// of every region count either side of the bound.
    #[test]
    fn a_handoff_is_taken_whole_or_refused_whole(
        regions in prop::collection::vec(region(), 1..200),
    ) {
        let count = regions.len();
        match MemoryMap::try_from_regions(regions) {
            Ok(map) => prop_assert_eq!(map.regions().count(), count, "a prefix was kept"),
            Err(_) => prop_assert!(count > MAX_REGIONS, "a map that fits was refused"),
        }
    }
}

/// The properties above skip a refused map, so this states how often they do
/// not — D-242 requires the rate rather than an assumption about it.
///
/// A suite whose guards almost always fire passes vacuously and is
/// indistinguishable from one that works, which is the failure this measures.
#[test]
fn the_properties_reach_a_constructed_map_and_a_found_run() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut constructed = 0usize;
    let mut found_a_run = 0usize;

    for _ in 0..SAMPLES {
        let value = regions()
            .new_tree(&mut runner)
            .expect("a region set")
            .current();
        let Ok(map) = MemoryMap::try_from_regions(value) else {
            continue;
        };
        constructed += 1;
        if map.first_memory_run(PAGE).is_some() {
            found_a_run += 1;
        }
    }

    println!("memory-map: {constructed}/{SAMPLES} constructed, {found_a_run} yielded a run");
    assert!(
        constructed > SAMPLES / 2,
        "only {constructed} of {SAMPLES} region sets built a map; the properties are near-vacuous"
    );
    assert!(
        found_a_run > SAMPLES / 8,
        "only {found_a_run} of {SAMPLES} maps yielded a run; the run property is near-vacuous"
    );
}
