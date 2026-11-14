use super::*;

fn boot_sector() -> [u8; SECTOR_BYTES] {
    let mut sector = [0u8; SECTOR_BYTES];
    sector[11..13].copy_from_slice(&512u16.to_le_bytes());
    sector[13] = 8;
    sector[14..16].copy_from_slice(&32u16.to_le_bytes());
    sector[16] = 2;
    sector[32..36].copy_from_slice(&1_048_576u32.to_le_bytes());
    sector[36..40].copy_from_slice(&1_024u32.to_le_bytes());
    sector[44..48].copy_from_slice(&2u32.to_le_bytes());
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector
}

#[test]
fn impossible_volume_geometry_is_refused_before_arithmetic() {
    let mut outside = boot_sector();
    outside[32..36].copy_from_slice(&2_079u32.to_le_bytes());
    assert_eq!(Bpb::parse(&outside), Err(FatError::Malformed));

    outside[32..36].copy_from_slice(&2_080u32.to_le_bytes());
    assert_eq!(Bpb::parse(&outside), Err(FatError::Malformed));
}

#[test]
fn a_reserved_link_beyond_the_last_cluster_does_not_hide_it() {
    let mut volume = image_volume(1, 4);
    set_link(&mut volume, FIRST_DATA_CLUSTER, FIRST_DATA_CLUSTER + 1);
    set_link(&mut volume, FIRST_DATA_CLUSTER + 1, FIRST_RESERVED_CLUSTER);
    let last = volume
        .bpb()
        .sector_of(FIRST_DATA_CLUSTER + 1)
        .expect("cluster 3 is on the volume") as usize;
    volume.source.sectors[last].fill(0xA5);

    let entry = Entry::for_contents(FIRST_DATA_CLUSTER, SECTOR_BYTES as u32 + 1);
    let mut buf = [0u8; SECTOR_BYTES + 1];
    assert_eq!(
        volume.read_file(&entry, 0, &mut buf),
        Ok(buf.len()),
        "the hare treated the unused successor as a failed read"
    );
    assert_eq!(buf[SECTOR_BYTES], 0xA5);
}

#[test]
fn a_partition_base_that_wraps_is_malformed() {
    let mut volume = image_volume(1, 4);
    volume.base = u64::MAX;
    assert_eq!(
        volume.next_cluster(FIRST_DATA_CLUSTER),
        Err(FatError::Malformed)
    );
}

#[derive(Debug)]
struct SectorImage {
    sectors: Vec<[u8; SECTOR_BYTES]>,
    reads: Vec<u64>,
    read_limit: usize,
}

impl Sectors for SectorImage {
    type Error = ();

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        self.reads.push(sector);
        if self.reads.len() > self.read_limit {
            return Err(());
        }
        let index = usize::try_from(sector).map_err(|_| ())?;
        *into = *self.sectors.get(index).ok_or(())?;
        Ok(())
    }
}

fn image_volume(sectors_per_cluster: u32, clusters: u32) -> Volume<SectorImage> {
    let total_sectors = 2 + sectors_per_cluster * clusters;
    Volume {
        source: SectorImage {
            sectors: vec![[0; SECTOR_BYTES]; total_sectors as usize],
            reads: Vec::new(),
            read_limit: 32,
        },
        base: 0,
        bpb: Bpb {
            sectors_per_cluster,
            reserved_sectors: 1,
            fats: 1,
            sectors_per_fat: 1,
            active_fat: 0,
            root_cluster: FIRST_DATA_CLUSTER,
            total_sectors,
        },
    }
}

fn set_link(volume: &mut Volume<SectorImage>, cluster: u32, next: u32) {
    let offset = cluster as usize * size_of::<u32>();
    volume.source.sectors[1][offset..offset + 4].copy_from_slice(&next.to_le_bytes());
}

fn directory_entries<S: Sectors>(
    volume: &mut Volume<S>,
    cluster: u32,
) -> Result<Vec<Entry>, FatError> {
    let mut cursor = DirectoryCursor::new(cluster);
    let mut entries = Vec::new();
    loop {
        let chunk = volume.directory_chunk(&mut cursor)?;
        entries.extend_from_slice(chunk.entries());
        if cursor.is_done() {
            return Ok(entries);
        }
    }
}

#[test]
fn a_huge_acyclic_directory_stops_at_the_entry_cap() {
    let bpb = Bpb {
        sectors_per_cluster: 32,
        reserved_sectors: 1,
        fats: 1,
        sectors_per_fat: 64,
        active_fat: 0,
        root_cluster: FIRST_DATA_CLUSTER,
        total_sectors: 100_000,
    };
    let cap = bpb.directory_cluster_limit();
    assert_eq!(cap, 128, "512 entries per cluster caps 65,536 at 128");
    assert!(bpb.clusters() > cap);
    let mut volume = Volume {
        source: CountedLine,
        base: 0,
        bpb,
    };
    let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);

    let mut chunks = 0u32;
    let error = loop {
        match volume.directory_chunk(&mut cursor) {
            Ok(_) => {
                chunks += 1;
                assert!(chunks < cap * 40, "the walk ran past the entry cap");
            }
            Err(error) => break error,
        }
    };
    assert_eq!(error, FatError::Unbounded);
    assert_eq!(cursor.visited, cap - 1);
}

#[derive(Debug)]
struct CountedCycle {
    reads: u32,
    ceiling: u32,
}

impl Sectors for CountedCycle {
    type Error = ();

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        self.reads += 1;
        if self.reads > self.ceiling {
            return Err(());
        }
        if sector == 1 {
            into.fill(0);
            into[8..12].copy_from_slice(&3u32.to_le_bytes());
            into[12..16].copy_from_slice(&2u32.to_le_bytes());
        } else {
            into.fill(0xE5);
        }
        Ok(())
    }
}

/// A FAT whose every cluster links to the next: a never-ending chain with no cycle.
struct CountedLine;

impl Sectors for CountedLine {
    type Error = ();

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        if (1..=64).contains(&sector) {
            let base = ((sector - 1) * 128) as u32;
            for (index, slot) in into.chunks_exact_mut(4).enumerate() {
                slot.copy_from_slice(&(base + index as u32 + 1).to_le_bytes());
            }
        } else {
            into.fill(0xE5);
        }
        Ok(())
    }
}

fn tiny(clusters: u32) -> Bpb {
    Bpb {
        sectors_per_cluster: 1,
        reserved_sectors: 1,
        fats: 1,
        sectors_per_fat: 1,
        active_fat: 0,
        root_cluster: FIRST_DATA_CLUSTER,
        total_sectors: clusters + 2,
    }
}

#[test]
fn a_cyclic_chain_is_refused_in_both_the_directory_and_file_paths() {
    let bpb = tiny(62);
    assert_eq!(bpb.clusters(), 62);
    let ceiling = bpb.clusters() * 4;

    let mut volume = Volume {
        source: CountedCycle { reads: 0, ceiling },
        base: 0,
        bpb,
    };
    assert_eq!(
        directory_entries(&mut volume, FIRST_DATA_CLUSTER),
        Err(FatError::Unbounded),
        "the directory walk did not refuse the cyclic chain"
    );

    let mut volume = Volume {
        source: CountedCycle { reads: 0, ceiling },
        base: 0,
        bpb,
    };
    let entry = Entry::for_contents(FIRST_DATA_CLUSTER, u32::MAX);
    let mut byte = [0u8; 1];
    assert_eq!(
        volume.read_file(&entry, u32::MAX - 1, &mut byte),
        Err(FatError::Unbounded),
        "the file skip did not refuse the cyclic chain"
    );
}
