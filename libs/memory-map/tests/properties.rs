//! Property tests over the map's documented contracts, each citing the sentence it transcribes.

use memory_map::{MAX_REGIONS, MemoryKind, MemoryMap, MemoryRegion};
use proptest::prelude::*;

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

/// Small bases and lengths, so generated regions overlap and share cursors.
fn region() -> impl Strategy<Value = MemoryRegion> {
    (0u64..0x8000, 0u64..0x8000, kind()).prop_map(|(base, len, kind)| MemoryRegion {
        base,
        len,
        kind,
    })
}

proptest! {

    /// Transcribed from "Copy a complete region stream or refuse it whole".
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
