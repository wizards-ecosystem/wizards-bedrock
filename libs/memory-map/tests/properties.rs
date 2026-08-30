//! Property tests over the map's stated contracts.
//!
//! The unit tests pin the two answers a reader can check by hand. These ask
//! whether the claims the doc comments make hold over region sets nobody would
//! think to write — which is how D-241's three contradictions were found, all
//! of them on regions sharing a base.
//!
//! Every property here cites the sentence it was transcribed from, which is
//! D-242's admissibility rule: a check whose text could be recovered by reading
//! the function body would be a restatement of the implementation.
//!
//! What is *not* here: anything about whether a real bootloader can produce the
//! region sets these generate. It cannot — the Limine protocol promises usable
//! entries overlap nothing, which is why the contradictions were latent rather
//! than live. This tier checks the crate against its own claims, not against
//! the one caller that happens to satisfy a stronger precondition.

use memory_map::{MAX_REGIONS, MemoryKind, MemoryMap, MemoryRegion};
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

proptest! {
    /// A run `first_memory_run` names is page-aligned, nonzero, and memory by
    /// the map's own other answer.
    ///
    /// Transcribed from "Find a page-aligned, nonzero memory run of at least
    /// `len` bytes". Two contracts about one address must agree; before D-241
    /// they did not, because the filter chose from a direct-mapped region while
    /// the walk landed in a reserved one overlapping it.
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

    /// The allocatable total saturates rather than wrapping.
    ///
    /// Transcribed from "Saturating, because a wrapped total is a wrong number
    /// handed to a frame allocator". A bare sum panics under the debug gate and
    /// wraps under release, and lengths this crate does not bound can reach it.
    #[test]
    fn the_allocatable_total_saturates_rather_than_wrapping(
        lens in prop::collection::vec(0u64..u64::MAX, 1..8),
    ) {
        let built: Vec<_> = lens
            .iter()
            .map(|&len| MemoryRegion { base: 0, len, kind: MemoryKind::Usable })
            .collect();
        let Ok(map) = MemoryMap::try_from_regions(built) else {
            return Ok(());
        };
        let expected = lens.iter().fold(0u64, |total, &len| total.saturating_add(len));
        prop_assert_eq!(
            map.allocatable_bytes(),
            expected,
            "the reported total is not the saturating sum of the region lengths"
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
