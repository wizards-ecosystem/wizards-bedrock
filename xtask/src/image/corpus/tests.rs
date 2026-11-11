use super::*;

#[derive(Debug)]
struct Bytes<'bytes>(&'bytes [u8]);

impl fat32::Sectors for Bytes<'_> {
    type Error = ();

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; fat32::SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        let at = usize::try_from(sector).map_err(|_| ())? * fat32::SECTOR_BYTES;
        let end = at.checked_add(fat32::SECTOR_BYTES).ok_or(())?;
        let slice = self.0.get(at..end).ok_or(())?;
        into.copy_from_slice(slice);
        Ok(())
    }
}

/// With chrono off `fatfs` pins the volume id and timestamps; every check below rests on this.
#[test]
fn the_corpus_is_the_same_bytes_every_time() {
    let first = build().expect("a volume");
    let second = build().expect("a volume");
    assert_eq!(first.len(), second.len());
    assert!(first == second, "two builds of the corpus differ");
}

#[test]
fn the_corpus_really_is_fat32() {
    let bytes = build().expect("a volume");
    let bpb = fat32::Bpb::parse(bytes[..fat32::SECTOR_BYTES].try_into().expect("a sector"))
        .expect("a FAT32 boot sector");
    assert!(bpb.clusters() >= 65_525, "{} clusters", bpb.clusters());
    assert!(bpb.root_cluster() >= fat32::FIRST_DATA_CLUSTER);
}

/// The oracle: two independent readers of one corpus, as a dependency cannot disagree with itself.
#[test]
fn the_reader_reads_what_the_writer_wrote() {
    let bytes = build().expect("a volume");
    let mut volume = fat32::Volume::mount(Bytes(&bytes), 0).expect("mounting the corpus");

    let root = volume.bpb().root_cluster();
    let small = volume
        .lookup(root, SMALL_NAME.as_bytes())
        .expect("HELLO.TXT is there");
    assert_eq!(small.size as usize, SMALL_CONTENT.len());
    let mut read = vec![0u8; SMALL_CONTENT.len()];
    assert_eq!(
        volume.read_file(&small, 0, &mut read).expect("readable"),
        SMALL_CONTENT.len()
    );
    assert_eq!(read, SMALL_CONTENT);

    let large = volume
        .lookup(root, LARGE_NAME.as_bytes())
        .expect("BIG.BIN is there");
    assert_eq!(large.size as usize, LARGE_BYTES);
    let mut whole = vec![0u8; LARGE_BYTES];
    assert_eq!(
        volume.read_file(&large, 0, &mut whole).expect("readable"),
        LARGE_BYTES
    );
    for (index, byte) in whole.iter().enumerate() {
        assert_eq!(*byte, large_byte(index), "byte {index}");
    }

    let directory = volume
        .lookup(root, DIRECTORY_NAME.as_bytes())
        .expect("DATA is there");
    assert!(directory.is_directory());
    let nested = volume
        .lookup(directory.cluster, NESTED_NAME.as_bytes())
        .expect("NESTED.TXT is there");
    let mut inside = vec![0u8; NESTED_CONTENT.len()];
    assert_eq!(
        volume.read_file(&nested, 0, &mut inside).expect("readable"),
        NESTED_CONTENT.len()
    );
    assert_eq!(inside, NESTED_CONTENT);
}

/// Offset zero reads the entry's own cluster whatever the walk does; only skipped clusters test it.
#[test]
fn a_read_from_the_middle_starts_where_it_was_told() {
    let bytes = build().expect("a volume");
    let mut volume = fat32::Volume::mount(Bytes(&bytes), 0).expect("mounting");
    let root = volume.bpb().root_cluster();
    let large = volume
        .lookup(root, LARGE_NAME.as_bytes())
        .expect("BIG.BIN is there");

    let cluster = volume.bpb().cluster_bytes() as usize;
    for offset in [
        1usize,
        511,
        512,
        cluster - 1,
        cluster,
        cluster + 1,
        cluster * 3 + 7,
    ] {
        let mut window = [0u8; 64];
        let got = volume
            .read_file(&large, offset as u32, &mut window)
            .expect("readable");
        assert_eq!(got, window.len(), "offset {offset}");
        for (index, byte) in window.iter().enumerate() {
            assert_eq!(
                *byte,
                large_byte(offset + index),
                "offset {offset} + {index}"
            );
        }
    }
}

/// The last cluster's tail holds stale bytes that a reader bounded by the cluster hands back.
#[test]
fn a_read_stops_at_the_end_of_the_file() {
    let bytes = build().expect("a volume");
    let mut volume = fat32::Volume::mount(Bytes(&bytes), 0).expect("mounting");
    let root = volume.bpb().root_cluster();
    let small = volume
        .lookup(root, SMALL_NAME.as_bytes())
        .expect("HELLO.TXT is there");

    let mut greedy = [0u8; 4096];
    let got = volume.read_file(&small, 0, &mut greedy).expect("readable");
    assert_eq!(
        got,
        SMALL_CONTENT.len(),
        "asked for a cluster, given a file"
    );
    assert_eq!(&greedy[..got], SMALL_CONTENT);

    let past = volume
        .read_file(&small, small.size, &mut greedy)
        .expect("readable");
    assert_eq!(past, 0);
}

/// A reader stopping at the deleted entry or returning long-name fragments lists another set.
/// Sorted, so walk order is not the claim: `GONE.TXT` is absent and every later entry present.
#[test]
fn the_root_lists_exactly_what_was_written() {
    let bytes = build().expect("a volume");
    let mut volume = fat32::Volume::mount(Bytes(&bytes), 0).expect("mounting");
    let root = volume.bpb().root_cluster();

    let mut names: Vec<String> = Vec::new();
    let mut cursor = fat32::DirectoryCursor::new(root);
    loop {
        let chunk = volume.directory_chunk(&mut cursor).expect("walkable");
        for entry in chunk.entries() {
            names.push(String::from_utf8_lossy(entry.name()).into_owned());
        }
        if cursor.is_done() {
            break;
        }
    }
    names.sort();

    assert_eq!(names, vec![LARGE_NAME, DIRECTORY_NAME, SMALL_NAME]);
}

#[test]
fn a_path_is_resolved_component_by_component() {
    let bytes = build().expect("a volume");
    let mut volume = fat32::Volume::mount(Bytes(&bytes), 0).expect("mounting");

    let nested = volume
        .resolve(&[DIRECTORY_NAME.as_bytes(), NESTED_NAME.as_bytes()])
        .expect("DATA/NESTED.TXT");
    assert_eq!(nested.size as usize, NESTED_CONTENT.len());

    assert_eq!(
        volume.resolve(&[SMALL_NAME.as_bytes(), NESTED_NAME.as_bytes()]),
        Err(fat32::FatError::WrongKind)
    );
    assert_eq!(
        volume.resolve(&[b"NOPE.TXT"]),
        Err(fat32::FatError::NotFound)
    );
}
