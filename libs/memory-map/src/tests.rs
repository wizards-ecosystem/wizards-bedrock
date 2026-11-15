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
