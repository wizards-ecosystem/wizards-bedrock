use super::*;

#[test]
fn every_fat_error_describes_its_cause() {
    fn assert_error<T: core::error::Error>() {}

    assert_error::<FatError>();
    for (error, expected) in [
        (FatError::Unreadable, "the sector source refused a read"),
        (
            FatError::NotABootSector,
            "the boot sector signature is missing",
        ),
        (
            FatError::SectorSize(4096),
            "sector size 4096 is unsupported",
        ),
        (FatError::ClusterSize(3), "cluster size 3 is invalid"),
        (FatError::Malformed, "the FAT32 layout is malformed"),
        (
            FatError::BadCluster(17),
            "cluster 17 is outside the data area",
        ),
        (FatError::Unbounded, "the cluster chain did not terminate"),
        (
            FatError::Truncated,
            "the cluster chain ended before the advertised file size",
        ),
        (FatError::NotFound, "the directory entry was not found"),
        (
            FatError::WrongKind,
            "the directory entry has the wrong kind",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }
}

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

#[derive(Debug)]
struct TracedSectors {
    boot: [u8; SECTOR_BYTES],
    base: u64,
    reads: [u64; 2],
    read_count: usize,
}

impl TracedSectors {
    const fn new(boot: [u8; SECTOR_BYTES], base: u64) -> Self {
        Self {
            boot,
            base,
            reads: [0; 2],
            read_count: 0,
        }
    }
}

impl Sectors for TracedSectors {
    type Error = core::convert::Infallible;

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        self.reads[self.read_count] = sector;
        self.read_count += 1;
        into.fill(0);
        if sector == self.base {
            *into = self.boot;
        } else {
            into[8..12].copy_from_slice(&END_OF_CHAIN.to_le_bytes());
        }
        Ok(())
    }
}

fn table_reads(extended_flags: u16) -> [u64; 2] {
    const BASE: u64 = 19;

    let mut boot = boot_sector();
    boot[40..42].copy_from_slice(&extended_flags.to_le_bytes());
    let mut volume = Volume::mount(TracedSectors::new(boot, BASE), BASE).expect("valid volume");

    assert_eq!(volume.next_cluster(FIRST_DATA_CLUSTER), Ok(Next::End));
    volume.source.reads
}

#[test]
fn the_boot_sector_is_read_where_the_layout_puts_its_fields() {
    let bpb = Bpb::parse(&boot_sector()).expect("a valid boot sector");

    assert_eq!(bpb.sectors_per_cluster(), 8);
    assert_eq!(bpb.cluster_bytes(), 4096);
    assert_eq!(bpb.root_cluster(), 2);
    assert_eq!(bpb.first_data_sector(), 32 + 2 * 1024);
    assert_eq!(bpb.clusters(), (1_048_576 - 2080) / 8);
    assert_eq!(bpb.table_position(128), (33, 0));
}

#[test]
fn fat32_rejects_every_legacy_or_unsupported_bpb_field() {
    for (offset, value) in [(17usize, 1u16), (19, 1), (22, 1), (42, 1)] {
        let mut boot = boot_sector();
        boot[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        assert_eq!(
            Bpb::parse(&boot),
            Err(FatError::Malformed),
            "field {offset}"
        );
    }
}

#[test]
fn a_boot_sector_that_is_not_one_is_refused() {
    let mut blank = [0u8; SECTOR_BYTES];
    assert_eq!(Bpb::parse(&blank), Err(FatError::NotABootSector));

    let mut half_signature = boot_sector();
    half_signature[510] = 0;
    assert_eq!(Bpb::parse(&half_signature), Err(FatError::NotABootSector));
    half_signature = boot_sector();
    half_signature[511] = 0;
    assert_eq!(Bpb::parse(&half_signature), Err(FatError::NotABootSector));

    blank[510] = 0x55;
    blank[511] = 0xAA;
    assert_eq!(Bpb::parse(&blank), Err(FatError::SectorSize(0)));

    let mut wide = boot_sector();
    wide[11..13].copy_from_slice(&4096u16.to_le_bytes());
    assert_eq!(Bpb::parse(&wide), Err(FatError::SectorSize(4096)));

    let mut lumpy = boot_sector();
    lumpy[13] = 3;
    assert_eq!(Bpb::parse(&lumpy), Err(FatError::ClusterSize(3)));
    lumpy[13] = 0;
    assert_eq!(Bpb::parse(&lumpy), Err(FatError::ClusterSize(0)));

    let mut fat16 = boot_sector();
    fat16[36..40].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(Bpb::parse(&fat16), Err(FatError::Malformed));

    let mut no_reserved_area = boot_sector();
    no_reserved_area[14..16].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(Bpb::parse(&no_reserved_area), Err(FatError::Malformed));

    let mut no_fat = boot_sector();
    no_fat[16] = 0;
    assert_eq!(Bpb::parse(&no_fat), Err(FatError::Malformed));

    let mut rooted = boot_sector();
    rooted[44..48].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(Bpb::parse(&rooted), Err(FatError::Malformed));
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
fn the_fat_indexes_every_advertised_data_cluster() {
    let mut exact = boot_sector();
    exact[13] = 1;
    exact[14..16].copy_from_slice(&1u16.to_le_bytes());
    exact[16] = 1;
    exact[32..36].copy_from_slice(&66_176u32.to_le_bytes());
    exact[36..40].copy_from_slice(&513u32.to_le_bytes());

    assert_eq!(Bpb::parse(&exact).map(Bpb::clusters), Ok(65_662));

    exact[32..36].copy_from_slice(&66_177u32.to_le_bytes());
    assert_eq!(Bpb::parse(&exact), Err(FatError::Malformed));
}

#[test]
fn a_fat32_volume_has_at_least_the_fat32_cluster_count() {
    let mut edge = boot_sector();
    edge[13] = 1;
    edge[14..16].copy_from_slice(&1u16.to_le_bytes());
    edge[16] = 1;
    edge[32..36].copy_from_slice(&66_038u32.to_le_bytes());
    edge[36..40].copy_from_slice(&512u32.to_le_bytes());

    assert_eq!(Bpb::parse(&edge).map(Bpb::clusters), Ok(65_525));

    edge[32..36].copy_from_slice(&66_037u32.to_le_bytes());
    assert_eq!(Bpb::parse(&edge), Err(FatError::Malformed));
}

#[test]
fn a_fat32_volume_stops_before_reserved_cluster_numbers() {
    let mut edge = boot_sector();
    edge[13] = 1;
    edge[14..16].copy_from_slice(&1u16.to_le_bytes());
    edge[16] = 1;
    edge[32..36].copy_from_slice(&270_532_591u32.to_le_bytes());
    edge[36..40].copy_from_slice(&2_097_152u32.to_le_bytes());

    let bpb = Bpb::parse(&edge).expect("the largest FAT32 data domain");
    assert_eq!(bpb.clusters(), 0x0FFF_FFEE);
    assert!(bpb.holds(0x0FFF_FFEF));
    for reserved in FIRST_RESERVED_CLUSTER..=BAD_CLUSTER {
        assert!(!bpb.holds(reserved));
    }

    edge[32..36].copy_from_slice(&270_532_592u32.to_le_bytes());
    assert_eq!(Bpb::parse(&edge), Err(FatError::Malformed));
}

#[test]
fn extended_flags_choose_the_fat_that_is_read() {
    const BASE: u64 = 19;
    const RESERVED: u64 = 32;
    const SECTORS_PER_FAT: u64 = 1_024;

    assert_eq!(table_reads(0), [BASE, BASE + RESERVED]);
    assert_eq!(
        table_reads(MIRRORING_DISABLED | 1),
        [BASE, BASE + RESERVED + SECTORS_PER_FAT]
    );
}

#[test]
fn an_active_fat_must_name_an_existing_copy() {
    let mut boot = boot_sector();
    boot[40..42].copy_from_slice(&(MIRRORING_DISABLED | 2).to_le_bytes());

    assert_eq!(Bpb::parse(&boot), Err(FatError::Malformed));
}

#[test]
fn extended_flags_reject_reserved_bits_and_meaningless_selectors() {
    for flags in [1, 1 << 4, 1 << 15, MIRRORING_DISABLED | (1 << 4)] {
        let mut boot = boot_sector();
        boot[40..42].copy_from_slice(&flags.to_le_bytes());
        assert_eq!(Bpb::parse(&boot), Err(FatError::Malformed), "{flags:#06x}");
    }
}

#[test]
fn the_root_cluster_must_lie_inside_the_data_area() {
    let mut outside = boot_sector();
    let clusters = (1_048_576 - 2_080) / 8;
    outside[44..48].copy_from_slice(&(FIRST_DATA_CLUSTER + clusters).to_le_bytes());

    assert_eq!(Bpb::parse(&outside), Err(FatError::Malformed));

    outside[44..48].copy_from_slice(&(FIRST_DATA_CLUSTER + clusters - 1).to_le_bytes());
    assert!(Bpb::parse(&outside).is_ok());
}

#[test]
fn a_clusters_sector_is_biased_by_two() {
    let bpb = Bpb::parse(&boot_sector()).expect("valid");
    let data = bpb.first_data_sector();

    assert_eq!(bpb.sector_of(2), Ok(data));
    assert_eq!(bpb.sector_of(3), Ok(data + 8));
    assert_eq!(bpb.sector_of(10), Ok(data + 64));

    assert_eq!(bpb.sector_of(0), Err(FatError::BadCluster(0)));
    assert_eq!(bpb.sector_of(1), Err(FatError::BadCluster(1)));
    let past = FIRST_DATA_CLUSTER + bpb.clusters();
    assert_eq!(bpb.sector_of(past), Err(FatError::BadCluster(past)));
    assert!(bpb.holds(past - 1));
}

#[test]
fn a_table_entry_is_twenty_eight_bits() {
    assert_eq!(decode_entry(0), Next::Free);
    assert_eq!(decode_entry(3), Next::Cluster(3));
    assert_eq!(decode_entry(0x0FFF_FFF6), Next::Cluster(0x0FFF_FFF6));
    assert_eq!(decode_entry(BAD_CLUSTER), Next::Bad);
    assert_eq!(decode_entry(END_OF_CHAIN), Next::End);
    assert_eq!(decode_entry(0x0FFF_FFFF), Next::End);

    assert_eq!(decode_entry(0xF000_0000), Next::Free);
    assert_eq!(decode_entry(0xF000_0003), Next::Cluster(3));
    assert_eq!(decode_entry(0xFFFF_FFFF), Next::End);
    assert_eq!(decode_entry(0xFFFF_FFF7), Next::Bad);
}

#[derive(Debug)]
struct LinkedTo(u32);

impl Sectors for LinkedTo {
    type Error = core::convert::Infallible;

    fn read_sector(
        &mut self,
        _sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        into.fill(0);
        into[8..12].copy_from_slice(&self.0.to_le_bytes());
        Ok(())
    }
}

#[test]
fn a_chain_link_must_name_a_cluster_in_this_volume() {
    let bpb = tiny(62);
    for invalid in [1, FIRST_DATA_CLUSTER + bpb.clusters()] {
        let mut volume = Volume {
            source: LinkedTo(invalid),
            base: 0,
            bpb,
        };
        assert_eq!(
            volume.next_cluster(FIRST_DATA_CLUSTER),
            Err(FatError::BadCluster(invalid))
        );
    }
    for reserved in FIRST_RESERVED_CLUSTER..BAD_CLUSTER {
        let mut volume = Volume {
            source: LinkedTo(reserved),
            base: 0,
            bpb,
        };
        assert_eq!(
            volume.next_cluster(FIRST_DATA_CLUSTER),
            Err(FatError::BadCluster(reserved))
        );
    }
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

fn short_entry(name: &[u8; 11], attributes: u8, cluster: u32, size: u32) -> [u8; ENTRY_BYTES] {
    let mut entry = [0u8; ENTRY_BYTES];
    entry[..11].copy_from_slice(name);
    entry[11] = attributes;
    entry[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    entry[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    entry
}

fn put_entry(
    volume: &mut Volume<SectorImage>,
    sector: usize,
    slot: usize,
    entry: [u8; ENTRY_BYTES],
) {
    let offset = slot * ENTRY_BYTES;
    volume.source.sectors[sector][offset..offset + ENTRY_BYTES].copy_from_slice(&entry);
}

fn directory_volume() -> Volume<SectorImage> {
    let mut volume = image_volume(2, 2);
    for sector in &mut volume.source.sectors[2..] {
        sector.fill(0xE5);
    }
    set_link(&mut volume, 2, 3);
    set_link(&mut volume, 3, END_OF_CHAIN);
    put_entry(
        &mut volume,
        3,
        15,
        short_entry(b"ALPHA   TXT", attribute::ARCHIVE, 2, 1),
    );
    put_entry(
        &mut volume,
        5,
        0,
        short_entry(b"OMEGA   TXT", attribute::ARCHIVE, 3, 1),
    );
    volume
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
fn a_directory_walk_uses_every_sector_and_its_exact_chain_bound() {
    let mut volume = directory_volume();
    let names: Vec<_> = directory_entries(&mut volume, FIRST_DATA_CLUSTER)
        .expect("the exact two-cluster chain ends")
        .iter()
        .map(|entry| entry.name().to_vec())
        .collect();
    assert_eq!(names, [b"ALPHA.TXT".to_vec(), b"OMEGA.TXT".to_vec()]);

    let mut volume = directory_volume();
    let found = volume
        .lookup(FIRST_DATA_CLUSTER, b"ALPHA.TXT")
        .expect("present");
    assert_eq!(found.name(), b"ALPHA.TXT");
    assert_eq!(
        volume.source.reads,
        [2, 3],
        "a false callback did not stop the walk immediately"
    );
}

#[test]
fn directory_chunks_advance_one_sector_and_finish_idempotently() {
    let mut volume = directory_volume();
    let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);

    let first = volume.directory_chunk(&mut cursor).expect("first sector");
    assert!(first.entries().is_empty());
    assert_eq!((cursor.cluster, cursor.sector, cursor.visited), (2, 1, 0));
    assert_eq!(volume.source.reads, [2]);

    let second = volume.directory_chunk(&mut cursor).expect("second sector");
    assert_eq!(second.entries().len(), 1);
    assert_eq!(second.entries()[0].name(), b"ALPHA.TXT");
    assert_eq!((cursor.cluster, cursor.sector, cursor.visited), (2, 2, 0));
    assert_eq!(volume.source.reads, [2, 3]);

    let third = volume.directory_chunk(&mut cursor).expect("next cluster");
    assert!(third.entries().is_empty());
    assert_eq!((cursor.cluster, cursor.sector, cursor.visited), (3, 1, 1));
    assert_eq!(volume.source.reads, [2, 3, 1, 1, 1, 4]);

    let fourth = volume.directory_chunk(&mut cursor).expect("last sector");
    assert_eq!(fourth.entries().len(), 1);
    assert_eq!(fourth.entries()[0].name(), b"OMEGA.TXT");
    assert_eq!((cursor.cluster, cursor.sector, cursor.visited), (3, 2, 1));

    let end = volume.directory_chunk(&mut cursor).expect("end of chain");
    assert!(end.entries().is_empty());
    assert!(cursor.is_done());
    assert_eq!(volume.source.reads, [2, 3, 1, 1, 1, 4, 5, 1]);

    let reads = volume.source.reads.clone();
    assert!(
        volume
            .directory_chunk(&mut cursor)
            .expect("terminal cursor")
            .entries()
            .is_empty()
    );
    assert_eq!(volume.source.reads, reads);
}

#[test]
fn a_directory_chunk_skips_dead_slots_and_stops_at_the_first_end_marker() {
    let mut volume = image_volume(1, 1);
    volume.source.sectors[2].fill(0xE5);

    let mut long_name = [0u8; ENTRY_BYTES];
    long_name[0] = 1;
    long_name[11] = attribute::LONG_NAME;
    put_entry(&mut volume, 2, 1, long_name);
    put_entry(
        &mut volume,
        2,
        2,
        short_entry(b"KEPT    TXT", attribute::ARCHIVE, 2, 7),
    );
    put_entry(&mut volume, 2, 3, [0; ENTRY_BYTES]);
    put_entry(
        &mut volume,
        2,
        4,
        short_entry(b"HIDDEN  TXT", attribute::ARCHIVE, 2, 9),
    );

    let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);
    let chunk = volume.directory_chunk(&mut cursor).expect("one sector");
    assert_eq!(chunk.entries().len(), 1);
    assert_eq!(chunk.entries()[0].name(), b"KEPT.TXT");
    assert!(cursor.is_done());
    assert_eq!(volume.source.reads, [2]);
}

#[test]
fn malformed_short_names_and_dot_entries_cannot_shadow_namespace_names() {
    let mut volume = image_volume(1, 4);
    set_link(&mut volume, FIRST_DATA_CLUSTER, END_OF_CHAIN);
    put_entry(
        &mut volume,
        2,
        0,
        short_entry(b"FI LE   TXT", attribute::ARCHIVE, 3, 1),
    );
    put_entry(
        &mut volume,
        2,
        1,
        short_entry(b".          ", attribute::DIRECTORY, 2, 0),
    );
    put_entry(
        &mut volume,
        2,
        2,
        short_entry(b"..         ", attribute::DIRECTORY, 2, 0),
    );
    put_entry(
        &mut volume,
        2,
        3,
        short_entry(b"FI/LE   TXT", attribute::ARCHIVE, 3, 1),
    );
    put_entry(
        &mut volume,
        2,
        4,
        short_entry(b"FI\0LE   TXT", attribute::ARCHIVE, 3, 1),
    );
    put_entry(
        &mut volume,
        2,
        5,
        short_entry(b"FILE    TXT", attribute::ARCHIVE, 4, 1),
    );

    let found = volume
        .lookup(FIRST_DATA_CLUSTER, b"FILE.TXT")
        .expect("the canonical entry after malformed aliases");
    assert_eq!(found.cluster, 4);

    let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);
    let chunk = volume
        .directory_chunk(&mut cursor)
        .expect("directory chunk");
    assert_eq!(
        chunk.entries().iter().map(Entry::name).collect::<Vec<_>>(),
        [b"FILE.TXT".as_slice()]
    );
}

fn canonical_entry_after(alias: [u8; ENTRY_BYTES]) -> Volume<SectorImage> {
    let mut volume = image_volume(1, 4);
    set_link(&mut volume, FIRST_DATA_CLUSTER, END_OF_CHAIN);
    put_entry(&mut volume, 2, 0, alias);
    put_entry(
        &mut volume,
        2,
        1,
        short_entry(b"FILE    TXT", attribute::ARCHIVE, 4, 1),
    );
    put_entry(&mut volume, 2, 2, [0; ENTRY_BYTES]);
    volume
}

#[test]
fn every_forbidden_short_name_byte_is_skipped_in_both_components() {
    const PUNCTUATION: [u8; 16] = *b"\"*+,./:;<=>?[\\]|";
    for byte in (0u8..=0x1F).chain([0x7F]).chain(PUNCTUATION) {
        for position in [1usize, 8] {
            let mut alias = short_entry(b"FAKE    BIN", attribute::ARCHIVE, 3, 1);
            alias[position] = byte;
            let mut volume = canonical_entry_after(alias);
            let found = volume
                .lookup(FIRST_DATA_CLUSTER, b"FILE.TXT")
                .expect("a malformed alias cannot shadow the canonical entry");
            assert_eq!(found.cluster, 4, "byte {byte:#04x} at {position}");

            let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);
            let chunk = volume.directory_chunk(&mut cursor).expect("one chunk");
            assert_eq!(
                chunk.entries().iter().map(Entry::name).collect::<Vec<_>>(),
                [b"FILE.TXT".as_slice()],
                "byte {byte:#04x} at {position}"
            );
        }
    }
}

#[test]
fn reserved_attributes_cannot_create_a_namespace_entry() {
    for reserved in [0x40, 0x80, 0xC0] {
        let alias = short_entry(b"FILE    TXT", attribute::ARCHIVE | reserved, 3, 1);
        let mut volume = canonical_entry_after(alias);
        assert_eq!(
            volume
                .lookup(FIRST_DATA_CLUSTER, b"FILE.TXT")
                .map(|entry| entry.cluster),
            Ok(4),
            "attributes {reserved:#04x}"
        );
    }
}

#[test]
fn mixed_case_disk_aliases_are_listed_and_resolved_canonically() {
    let alias = short_entry(b"MiXeD   TxT", attribute::ARCHIVE, 3, 1);
    let mut volume = canonical_entry_after(alias);
    assert_eq!(
        volume
            .lookup(FIRST_DATA_CLUSTER, b"mixed.txt")
            .map(|entry| entry.name().to_vec()),
        Ok(b"MIXED.TXT".to_vec())
    );

    let mut lower = short_entry(b"LOWER   TXT", attribute::ARCHIVE, 3, 1);
    lower[12] = CASE_FLAGS;
    let mut volume = canonical_entry_after(lower);
    assert_eq!(
        volume
            .lookup(FIRST_DATA_CLUSTER, b"LOWER.TXT")
            .map(|entry| entry.name().to_vec()),
        Ok(b"lower.txt".to_vec())
    );

    let mut reserved = short_entry(b"FILE    TXT", attribute::ARCHIVE, 3, 1);
    reserved[12] = 1;
    let mut volume = canonical_entry_after(reserved);
    assert_eq!(
        volume
            .lookup(FIRST_DATA_CLUSTER, b"FILE.TXT")
            .map(|entry| entry.cluster),
        Ok(4)
    );
}

#[test]
fn reserved_first_cluster_bits_cannot_create_an_entry() {
    let alias = short_entry(b"FILE    TXT", attribute::ARCHIVE, 0xF000_0003, 1);
    let mut volume = canonical_entry_after(alias);
    assert_eq!(
        volume
            .lookup(FIRST_DATA_CLUSTER, b"FILE.TXT")
            .map(|entry| entry.cluster),
        Ok(4)
    );
}

#[test]
fn an_escaped_deleted_marker_names_a_live_entry() {
    let mut volume = image_volume(1, 4);
    set_link(&mut volume, FIRST_DATA_CLUSTER, END_OF_CHAIN);
    put_entry(
        &mut volume,
        2,
        0,
        short_entry(b"\x05SCAPE  TXT", attribute::ARCHIVE, 3, 1),
    );
    put_entry(&mut volume, 2, 1, [0; ENTRY_BYTES]);

    let found = volume
        .lookup(FIRST_DATA_CLUSTER, b"\xE5SCAPE.TXT")
        .expect("0x05 escapes a live first 0xE5 byte");
    assert_eq!(found.name(), b"\xE5SCAPE.TXT");
    assert_eq!(found.cluster, 3);
}

#[test]
fn a_cyclic_directory_chain_is_caught_by_the_hare_not_the_volume() {
    let bpb = tiny(1_000_000);
    assert!(bpb.clusters() > bpb.directory_cluster_limit());
    let mut volume = Volume {
        source: CountedCycle {
            reads: 0,
            ceiling: 64,
        },
        base: 0,
        bpb,
    };
    let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);

    let mut chunks = 0u32;
    let error = loop {
        match volume.directory_chunk(&mut cursor) {
            Ok(chunk) => {
                assert!(chunk.entries().is_empty());
                chunks += 1;
                assert!(chunks < 8, "the hare did not catch the two-cycle quickly");
            }
            Err(error) => break error,
        }
    };
    assert_eq!(error, FatError::Unbounded);
    assert!(
        cursor.visited < 4,
        "the cycle was caught within its own length"
    );
    let before = cursor;
    assert_eq!(
        volume.directory_chunk(&mut cursor),
        Err(FatError::Unbounded)
    );
    assert_eq!(cursor, before, "a refused transition changed the cursor");
}

// 512-byte clusters hold 16 entries, so 65,536 entries need 4,096 clusters.
#[test]
fn the_directory_cluster_limit_is_the_tighter_of_entries_and_volume() {
    assert_eq!(tiny(1_000_000).directory_cluster_limit(), 4096);
    assert_eq!(tiny(100).directory_cluster_limit(), 100);
    let mut wide = tiny(1_000_000);
    wide.sectors_per_cluster = 8;
    assert_eq!(wide.directory_cluster_limit(), 512);
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

#[test]
fn an_owned_directory_chunk_survives_a_later_chain_error() {
    let mut volume = directory_volume();
    volume.source.read_limit = 2;
    let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);

    let first = volume.directory_chunk(&mut cursor).expect("first sector");
    assert!(first.entries().is_empty());
    let owned = volume.directory_chunk(&mut cursor).expect("second sector");
    assert_eq!(owned.entries()[0].name(), b"ALPHA.TXT");
    assert_eq!(
        volume.directory_chunk(&mut cursor),
        Err(FatError::Unreadable)
    );
    assert_eq!(owned.entries()[0].name(), b"ALPHA.TXT");
}

fn file_volume() -> (Volume<SectorImage>, Vec<u8>, Entry) {
    let mut volume = image_volume(2, 2);
    set_link(&mut volume, 2, 3);
    set_link(&mut volume, 3, END_OF_CHAIN);

    let mut contents = Vec::with_capacity(4 * SECTOR_BYTES);
    for sector in 2usize..6 {
        for byte in 0..SECTOR_BYTES {
            volume.source.sectors[sector][byte] = ((sector * 37 + byte) % 251) as u8;
        }
        contents.extend_from_slice(&volume.source.sectors[sector]);
    }
    let entry = Entry::for_contents(FIRST_DATA_CLUSTER, contents.len() as u32);
    (volume, contents, entry)
}

#[test]
fn file_reads_match_the_concatenated_sector_bytes_at_every_boundary() {
    for (offset, requested) in [
        (0usize, 2_048usize),
        (1, 1),
        (511, 3),
        (700, 900),
        (1_024, 64),
        (1_535, 600),
        (2_048, 9),
        (1, 0),
    ] {
        let (mut volume, contents, entry) = file_volume();
        let mut observed = vec![0xA5; requested];
        let read = volume
            .read_file(&entry, offset as u32, &mut observed)
            .expect("a bounded valid read");
        let expected = if offset >= contents.len() {
            &[][..]
        } else {
            let end = contents.len().min(offset + requested);
            &contents[offset..end]
        };
        assert_eq!(
            read,
            expected.len(),
            "offset={offset}, requested={requested}"
        );
        assert_eq!(
            &observed[..read],
            expected,
            "offset={offset}, requested={requested}"
        );
        assert!(
            observed[read..].iter().all(|byte| *byte == 0xA5),
            "the read wrote past its reported length"
        );
    }

    let (mut volume, _, mut entry) = file_volume();
    entry.size = 2_049;
    let mut byte = [0u8; 1];
    assert_eq!(
        volume.read_file(&entry, 2_048, &mut byte),
        Err(FatError::Unbounded),
        "skipping every possible cluster cannot leave a current cluster"
    );

    let mut volume = image_volume(1, 4);
    set_link(&mut volume, 2, 3);
    set_link(&mut volume, 3, 2);
    volume.source.sectors[2].fill(0x12);
    volume.source.sectors[3].fill(0x34);
    let entry = Entry::for_contents(2, 1_025);
    let mut bytes = [0u8; 1_025];
    assert_eq!(
        volume.read_file(&entry, 0, &mut bytes),
        Err(FatError::Unbounded),
        "a cycle cannot erase the volume-derived visit bound"
    );
}

fn resolve_volume() -> Volume<SectorImage> {
    let mut volume = image_volume(1, 3);
    put_entry(
        &mut volume,
        2,
        0,
        short_entry(b"DIR        ", attribute::DIRECTORY, 3, 0),
    );
    put_entry(
        &mut volume,
        2,
        1,
        short_entry(b"FILE    TXT", attribute::ARCHIVE, 4, 1),
    );
    put_entry(
        &mut volume,
        3,
        0,
        short_entry(b"INNER   TXT", attribute::ARCHIVE, 4, 1),
    );
    volume
}

#[test]
fn path_resolution_descends_only_through_directory_components() {
    let mut volume = resolve_volume();
    assert_eq!(
        volume
            .resolve(&[b"DIR".as_slice(), b"INNER.TXT".as_slice()])
            .map(|entry| entry.name().to_vec()),
        Ok(b"INNER.TXT".to_vec())
    );

    let mut volume = resolve_volume();
    assert_eq!(
        volume.resolve(&[b"FILE.TXT".as_slice(), b"INNER.TXT".as_slice()]),
        Err(FatError::WrongKind)
    );

    let mut volume = resolve_volume();
    assert_eq!(
        volume
            .resolve(&[b"FILE.TXT".as_slice()])
            .map(|entry| entry.name().to_vec()),
        Ok(b"FILE.TXT".to_vec())
    );
    assert_eq!(volume.resolve(&[]), Err(FatError::NotFound));
}

#[test]
fn the_two_directory_markers_are_not_the_same_marker() {
    let mut slot = [0u8; ENTRY_BYTES];
    assert_eq!(decode_slot(&slot), Slot::End);

    slot[0] = 0xE5;
    assert_eq!(decode_slot(&slot), Slot::Deleted);
}

#[test]
fn an_eight_three_name_is_unpacked() {
    let mut slot = [0u8; ENTRY_BYTES];
    slot[0..11].copy_from_slice(b"HELLO   TXT");
    slot[11] = attribute::ARCHIVE;
    slot[20..22].copy_from_slice(&0x0001u16.to_le_bytes());
    slot[26..28].copy_from_slice(&0x0234u16.to_le_bytes());
    slot[28..32].copy_from_slice(&4_096u32.to_le_bytes());

    let Slot::Entry(entry) = decode_slot(&slot) else {
        panic!("a usable entry");
    };
    assert_eq!(entry.name(), b"HELLO.TXT");
    assert_eq!(entry.size, 4_096);
    assert_eq!(entry.cluster, 0x0001_0234);
    assert!(!entry.is_directory());

    let mut bare = [0u8; ENTRY_BYTES];
    bare[0..11].copy_from_slice(b"DATA       ");
    bare[11] = attribute::DIRECTORY;
    let Slot::Entry(directory) = decode_slot(&bare) else {
        panic!("a usable entry");
    };
    assert_eq!(directory.name(), b"DATA");
    assert!(directory.is_directory());

    let mut sparse_extension = [0u8; ENTRY_BYTES];
    sparse_extension[0..11].copy_from_slice(b"EDGE      X");
    sparse_extension[11] = attribute::READ_ONLY;
    assert_eq!(decode_slot(&sparse_extension), Slot::Skip);

    let mut leading_extension = [0u8; ENTRY_BYTES];
    leading_extension[0..11].copy_from_slice(b"LEAD    X  ");
    leading_extension[11] = attribute::READ_ONLY;
    let Slot::Entry(leading_extension) = decode_slot(&leading_extension) else {
        panic!("a usable entry");
    };
    assert_eq!(leading_extension.name(), b"LEAD.X");
}

#[test]
fn a_full_eight_three_name_unpacks_under_any_attribute_byte() {
    let mut slot = [0u8; ENTRY_BYTES];
    slot[0..11].copy_from_slice(b"FILENAMEEXT");
    slot[11] = attribute::READ_ONLY;
    let Slot::Entry(entry) = decode_slot(&slot) else {
        panic!("a usable entry");
    };
    assert_eq!(entry.name(), b"FILENAME.EXT");

    let mut bad_extension = [0u8; ENTRY_BYTES];
    bad_extension[0..11].copy_from_slice(b"FILE    T*X");
    bad_extension[11] = attribute::READ_ONLY;
    assert_eq!(decode_slot(&bad_extension), Slot::Skip);
}

#[test]
fn dot_entries_are_skipped_as_invalid_short_names() {
    for name in [b".          ", b"..         "] {
        let mut slot = [0u8; ENTRY_BYTES];
        slot[0..11].copy_from_slice(name);
        slot[11] = attribute::DIRECTORY;
        assert_eq!(decode_slot(&slot), Slot::Skip);
    }
}

#[test]
fn long_name_fragments_and_the_volume_label_are_skipped() {
    assert_eq!(attribute::LONG_NAME, 0x0F);

    let mut fragment = [0u8; ENTRY_BYTES];
    fragment[0] = 0x41;
    fragment[11] = 0x0F;
    assert_eq!(decode_slot(&fragment), Slot::Skip);

    let mut label = [0u8; ENTRY_BYTES];
    label[0..11].copy_from_slice(b"WIZARD     ");
    label[11] = attribute::VOLUME_LABEL;
    assert_eq!(decode_slot(&label), Slot::Skip);

    let mut plain = [0u8; ENTRY_BYTES];
    plain[0..11].copy_from_slice(b"PLAIN      ");
    assert!(matches!(decode_slot(&plain), Slot::Entry(_)));
}

#[test]
fn a_name_is_matched_without_regard_to_case() {
    let mut slot = [0u8; ENTRY_BYTES];
    slot[0..11].copy_from_slice(b"HELLO   TXT");
    slot[11] = attribute::ARCHIVE;
    let Slot::Entry(entry) = decode_slot(&slot) else {
        panic!("a usable entry");
    };

    assert!(entry.is_named(b"HELLO.TXT"));
    assert!(entry.is_named(b"hello.txt"));
    assert!(entry.is_named(b"HeLLo.TxT"));
    assert!(!entry.is_named(b"JELLO.TXT"));
    assert!(!entry.is_named(b"HELLO.TX"), "a prefix is not the name");
    assert!(!entry.is_named(b"HELLO.TXTX"));
    assert!(!entry.is_named(b"GOODBYE.TXT"));
}

#[derive(Debug)]
struct LinearFat {
    reads: u32,
}

impl Sectors for LinearFat {
    type Error = ();

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        self.reads += 1;
        if self.reads > 300_000 {
            return Err(());
        }
        into.fill(0);
        if sector >= 1 {
            let first = u32::try_from((sector - 1) * 128).map_err(|_| ())?;
            for slot in 0..128usize {
                let cluster = first.checked_add(slot as u32).ok_or(())?;
                let next = cluster.checked_add(1).ok_or(())?;
                let at = slot * core::mem::size_of::<u32>();
                into[at..at + 4].copy_from_slice(&next.to_le_bytes());
            }
        }
        Ok(())
    }
}

#[test]
fn a_maximum_file_offset_does_not_overflow_chain_progress() {
    let bpb = Bpb {
        sectors_per_cluster: 128,
        reserved_sectors: 1,
        fats: 1,
        sectors_per_fat: 1,
        active_fat: 0,
        root_cluster: FIRST_DATA_CLUSTER,
        total_sectors: u32::MAX,
    };
    let mut volume = Volume {
        source: LinearFat { reads: 0 },
        base: 0,
        bpb,
    };
    let entry = Entry::for_contents(FIRST_DATA_CLUSTER, u32::MAX);
    let mut byte = [0u8; 1];

    assert_eq!(volume.read_file(&entry, u32::MAX - 1, &mut byte), Ok(1));
}

#[test]
fn a_file_skip_past_the_volume_is_refused_before_walking() {
    let bpb = tiny(62);
    assert_eq!(bpb.clusters(), 62);
    let mut volume = Volume {
        source: LinearFat { reads: 0 },
        base: 0,
        bpb,
    };
    let entry = Entry::for_contents(FIRST_DATA_CLUSTER, u32::MAX);
    let mut byte = [0u8; 1];
    assert_eq!(
        volume.read_file(&entry, 100 * SECTOR_BYTES as u32, &mut byte),
        Err(FatError::Unbounded)
    );
    assert_eq!(
        volume.source.reads, 0,
        "the skip was refused before any read"
    );
}

#[test]
fn an_early_end_of_chain_is_a_truncated_file_not_eof() {
    let mut skipped = image_volume(1, 4);
    set_link(&mut skipped, FIRST_DATA_CLUSTER, END_OF_CHAIN);
    let entry = Entry::for_contents(FIRST_DATA_CLUSTER, (SECTOR_BYTES * 2) as u32);
    let mut byte = [0u8; 1];
    assert_eq!(
        skipped.read_file(&entry, SECTOR_BYTES as u32, &mut byte),
        Err(FatError::Truncated)
    );

    let mut crossed = image_volume(1, 4);
    set_link(&mut crossed, FIRST_DATA_CLUSTER, END_OF_CHAIN);
    let mut two_sectors = [0u8; SECTOR_BYTES * 2];
    assert_eq!(
        crossed.read_file(&entry, 0, &mut two_sectors),
        Err(FatError::Truncated)
    );
}

#[test]
fn skipped_clusters_consume_the_same_cycle_budget_as_copied_clusters() {
    let mut volume = image_volume(1, 4);
    set_link(&mut volume, 2, 3);
    set_link(&mut volume, 3, 2);
    let entry = Entry::for_contents(2, (SECTOR_BYTES * 3) as u32);
    let mut tail = [0u8; SECTOR_BYTES * 2];

    assert_eq!(
        volume.read_file(&entry, SECTOR_BYTES as u32, &mut tail),
        Err(FatError::Unbounded)
    );
    assert_eq!(
        volume.source.reads.len(),
        7,
        "the copy phase received a fresh chain budget"
    );
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
