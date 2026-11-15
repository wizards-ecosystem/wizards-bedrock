use super::*;

#[test]
fn overlapping_regions_answer_the_same_in_either_order() {
    let usable = MemoryRegion {
        base: 0,
        len: PAGE_SIZE * 3,
        kind: MemoryKind::Usable,
    };
    let reserved = MemoryRegion {
        base: 0,
        len: PAGE_SIZE * 2,
        kind: MemoryKind::Reserved,
    };
    let forward = MemoryMap::try_from_regions([reserved, usable]).expect("map");
    let reversed = MemoryMap::try_from_regions([usable, reserved]).expect("map");

    for (start, end) in [(0, 1), (0, PAGE_SIZE), (PAGE_SIZE, PAGE_SIZE * 3)] {
        assert_eq!(
            forward.covers(start, end),
            reversed.covers(start, end),
            "covers({start:#x}, {end:#x}) depends on region order"
        );
        assert_eq!(
            forward.covers_memory(start, end),
            reversed.covers_memory(start, end),
            "covers_memory({start:#x}, {end:#x}) depends on region order"
        );
    }
    assert!(!forward.covers_memory(0, 1));
    assert!(forward.covers(0, 1));

    for map in [forward, reversed] {
        let runs: Vec<_> = map
            .allocatable_runs()
            .map(|run| (run.base(), run.bytes()))
            .collect();
        assert_eq!(runs, [(PAGE_SIZE * 2, PAGE_SIZE)]);
    }
}

#[test]
fn a_byte_granular_region_ending_mid_page_is_direct_mapped_to_its_last_page() {
    const FRAMEBUFFER: u64 = 0x100_0000;
    const FRAMEBUFFER_BYTES: u64 = 3200 * 600;
    const SHARED_PAGE: u64 = 0x200_0000;
    const LONE_TABLE: u64 = 0x300_0000;
    let region = |base, len, kind| MemoryRegion { base, len, kind };
    let map = MemoryMap::try_from_regions([
        region(0, PAGE_SIZE * 16, MemoryKind::Usable),
        region(FRAMEBUFFER, FRAMEBUFFER_BYTES, MemoryKind::Framebuffer),
        region(SHARED_PAGE, 100, MemoryKind::AcpiReclaimable),
        region(SHARED_PAGE + 100, 100, MemoryKind::Reserved),
        region(LONE_TABLE, 100, MemoryKind::AcpiNvs),
    ])
    .expect("map");
    let framebuffer_end = (FRAMEBUFFER + FRAMEBUFFER_BYTES).next_multiple_of(PAGE_SIZE);

    let runs: Vec<_> = map
        .page_runs()
        .map(|run| (run.base(), run.bytes(), run.cache()))
        .collect();
    assert_eq!(
        runs,
        [
            (0, PAGE_SIZE * 16, DirectMapCache::Normal),
            (
                FRAMEBUFFER,
                framebuffer_end - FRAMEBUFFER,
                DirectMapCache::Framebuffer
            ),
            (LONE_TABLE, PAGE_SIZE, DirectMapCache::Normal),
        ]
    );
    assert_eq!(
        map.direct_map_cache(framebuffer_end - PAGE_SIZE, framebuffer_end),
        Some(DirectMapCache::Framebuffer)
    );
    assert!(!map.covers(FRAMEBUFFER + FRAMEBUFFER_BYTES, framebuffer_end));
}
