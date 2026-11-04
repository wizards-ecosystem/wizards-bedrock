//! Deterministic FAT32 corpus shared by machine and host checks.
//! `fatfs` and `libs/fat32` read the same bytes; a foreign corpus checks the format assumptions.

#[cfg(test)]
use std::io::Cursor;
use std::io::{Read, Seek, Write};

use anyhow::{Context, Result};
use fatfs::{FatType, FileSystem, FsOptions};

use super::esp;

/// Bytes of the volume.
/// Below 65525 clusters `fatfs` silently formats FAT16, which this reader refuses.
pub const VOLUME_BYTES: usize = 64 * 1024 * 1024;

/// A file small enough to sit in one cluster.
pub const SMALL_NAME: &str = "HELLO.TXT";

/// A file long enough to need a cluster chain.
/// Forty kilobytes exceeds one cluster at every size `fatfs` chooses here.
pub const LARGE_NAME: &str = "BIG.BIN";

/// Bytes of [`LARGE_NAME`].
pub const LARGE_BYTES: usize = 40 * 1024;

/// A file created and then deleted, leaving `0xE5` before live entries.
/// That is the only way to tell a deleted slot from the end of the directory.
pub const DELETED_NAME: &str = "GONE.TXT";

/// A directory, so the walk has somewhere to descend.
pub const DIRECTORY_NAME: &str = "DATA";

/// A file inside it.
pub const NESTED_NAME: &str = "NESTED.TXT";

/// What [`SMALL_NAME`] holds.
pub const SMALL_CONTENT: &[u8] = b"the wizard's bedrock reads a disk\n";

/// What [`NESTED_NAME`] holds.
pub const NESTED_CONTENT: &[u8] = b"one directory down\n";

/// Byte `index` of [`LARGE_NAME`], different at every position.
/// A repeated byte cannot tell a right offset or a right cluster from a wrong one.
pub const fn large_byte(index: usize) -> u8 {
    // A cheap mixing of the index so neighbouring bytes differ and so do
    // bytes a cluster apart.
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
}

/// Build the volume.
/// A refusal here is a `fatfs` version change, not an environment problem.
#[cfg(test)]
fn build() -> Result<Vec<u8>> {
    let mut bytes = vec![0u8; VOLUME_BYTES];
    populate(Cursor::new(&mut bytes[..]))?;

    Ok(bytes)
}

pub(crate) fn populate(mut volume: impl Read + Write + Seek) -> Result<()> {
    esp::format(&mut volume, FatType::Fat32).context("formatting the corpus volume")?;
    let filesystem =
        FileSystem::new(volume, FsOptions::new()).context("opening the corpus volume")?;
    {
        let root = filesystem.root_dir();

        esp::write_file(&root, SMALL_NAME, SMALL_CONTENT)?;
        // Removing this only after later live entries exist makes premature
        // end-of-directory handling observable.
        esp::write_file(&root, DELETED_NAME, b"not for long\n")?;

        let large: Vec<u8> = (0..LARGE_BYTES).map(large_byte).collect();
        esp::write_file(&root, LARGE_NAME, &large)?;

        let directory = root
            .create_dir(DIRECTORY_NAME)
            .with_context(|| format!("creating /{DIRECTORY_NAME}"))?;
        esp::write_file(&directory, NESTED_NAME, NESTED_CONTENT)?;

        root.remove(DELETED_NAME)
            .with_context(|| format!("removing /{DELETED_NAME}"))?;
    }
    filesystem.unmount().context("unmounting the corpus")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source of sectors backed by a slice, which is what the host has where
    /// the kernel has a controller.
    struct Bytes<'a>(&'a [u8]);

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

    /// The corpus is the same bytes every time it is built.
    /// With chrono off, `fatfs` pins the volume id and the timestamps.
    #[test]
    fn the_corpus_is_the_same_bytes_every_time() {
        let first = build().expect("a volume");
        let second = build().expect("a volume");
        assert_eq!(first.len(), second.len());
        assert!(first == second, "two builds of the corpus differ");
    }

    /// It really is FAT32, not FAT16 wearing the name.
    /// Below 65525 clusters `fatfs` formats FAT16 whatever it is asked for.
    #[test]
    fn the_corpus_really_is_fat32() {
        let bytes = build().expect("a volume");
        let bpb = fat32::Bpb::parse(bytes[..fat32::SECTOR_BYTES].try_into().expect("a sector"))
            .expect("a FAT32 boot sector");
        assert!(bpb.clusters() >= 65_525, "{} clusters", bpb.clusters());
        assert!(bpb.root_cluster() >= fat32::FIRST_DATA_CLUSTER);
    }

    /// `libs/fat32` reads what `fatfs` wrote, byte for byte.
    /// Two implementations of one format; a dependency cannot disagree with itself.
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

    /// A read from the middle of a file starts where it was told to.
    /// Offset zero succeeds even if the chain walk is wrong; only a skip can show it.
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

    /// A read stops at the end of the file, not at the end of a cluster.
    /// The last cluster's tail is the previous occupant's data.
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

    /// Every file the corpus holds is listed, and nothing else is.
    /// Stopping at the first deleted entry would still leave the lookups above green.
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

        // Sorted, so walk order is not the claim: `GONE.TXT` is absent and
        // everything after its slot is present.
        assert_eq!(names, vec![LARGE_NAME, DIRECTORY_NAME, SMALL_NAME]);
    }

    /// A path is resolved component by component.
    #[test]
    fn a_path_is_resolved_component_by_component() {
        let bytes = build().expect("a volume");
        let mut volume = fat32::Volume::mount(Bytes(&bytes), 0).expect("mounting");

        let nested = volume
            .resolve(&[DIRECTORY_NAME.as_bytes(), NESTED_NAME.as_bytes()])
            .expect("DATA/NESTED.TXT");
        assert_eq!(nested.size as usize, NESTED_CONTENT.len());

        // A component that is not a directory cannot be walked through.
        assert_eq!(
            volume.resolve(&[SMALL_NAME.as_bytes(), NESTED_NAME.as_bytes()]),
            Err(fat32::FatError::WrongKind)
        );
        assert_eq!(
            volume.resolve(&[b"NOPE.TXT"]),
            Err(fat32::FatError::NotFound)
        );
    }
}
