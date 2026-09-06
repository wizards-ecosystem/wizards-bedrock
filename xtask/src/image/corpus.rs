//! Deterministic FAT32 corpus shared by machine and host checks.
//!
//! The exact bytes embedded in the NVMe namespace are read independently by
//! `fatfs` and `libs/fat32`; the pinned foreign corpus separately checks the
//! format assumptions (D-170, D-207).

#[cfg(test)]
use std::io::Cursor;
use std::io::{Read, Seek, Write};

use anyhow::{Context, Result};
use fatfs::{FatType, FileSystem, FsOptions};

use super::esp;

/// Bytes of the volume.
///
/// FAT32 needs at least 65525 clusters to be FAT32 at all, so a volume much
/// smaller than this is silently formatted as FAT16 — which this reader
/// refuses, correctly, and which would look like a bug in the reader.
pub const VOLUME_BYTES: usize = 64 * 1024 * 1024;

/// A file small enough to sit in one cluster.
pub const SMALL_NAME: &str = "HELLO.TXT";

/// A file long enough to need a chain of them.
///
/// Forty kilobytes: more than one cluster at every cluster size `fatfs` will
/// choose here, so reading it walks the table rather than staying in the first
/// cluster. A file that fitted would leave the chain walk unexercised.
pub const LARGE_NAME: &str = "BIG.BIN";

/// Bytes of [`LARGE_NAME`].
pub const LARGE_BYTES: usize = 40 * 1024;

/// A file that is created and then deleted.
///
/// It leaves a slot beginning `0xE5` **before** live entries, which is the
/// only way to reach the difference between "deleted" and "the directory ends
/// here" on a real volume. A reader that confused the two would lose every
/// file after it, and against a corpus with no deletions nothing would say so
/// (D-170).
pub const DELETED_NAME: &str = "GONE.TXT";

/// A directory, so the walk has somewhere to descend.
pub const DIRECTORY_NAME: &str = "DATA";

/// A file inside it.
pub const NESTED_NAME: &str = "NESTED.TXT";

/// What [`SMALL_NAME`] holds.
pub const SMALL_CONTENT: &[u8] = b"the wizard's os reads a disk\n";

/// What [`NESTED_NAME`] holds.
pub const NESTED_CONTENT: &[u8] = b"one directory down\n";

/// Byte `index` of [`LARGE_NAME`].
///
/// **Position-dependent on purpose.** A file of one repeated byte cannot tell a
/// read at the right offset from a read at the wrong one, and a reader that
/// walked the chain incorrectly would return plausible bytes from the wrong
/// cluster and pass. This is D-155's rule applied to a corpus (D-170).
pub const fn large_byte(index: usize) -> u8 {
    // A cheap mixing of the index so neighbouring bytes differ and so do
    // bytes a cluster apart.
    ((index as u32).wrapping_mul(2_654_435_761) >> 13) as u8
}

/// Build the volume.
///
/// # Errors
///
/// If `fatfs` refuses to format or to write, which on an in-memory volume of
/// this size means a version change rather than an environment problem.
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
    ///
    /// What `012` §1 measured and what the whole arrangement rests on: with
    /// chrono off, `fatfs` pins the volume id and the timestamps, so a volume
    /// built now and one built in a second are identical. A corpus that
    /// changed under the checks would make every disagreement below suspect.
    #[test]
    fn the_corpus_is_the_same_bytes_every_time() {
        let first = build().expect("a volume");
        let second = build().expect("a volume");
        assert_eq!(first.len(), second.len());
        assert!(first == second, "two builds of the corpus differ");
    }

    /// It really is FAT32, and not FAT16 wearing the name.
    ///
    /// Below 65525 clusters `fatfs` formats FAT16 whatever it is asked for,
    /// and `libs/fat32` refuses that — correctly, and in a way that would look
    /// like a bug in the reader rather than a volume too small.
    #[test]
    fn the_corpus_really_is_fat32() {
        let bytes = build().expect("a volume");
        let bpb = fat32::Bpb::parse(bytes[..fat32::SECTOR_BYTES].try_into().expect("a sector"))
            .expect("a FAT32 boot sector");
        assert!(bpb.clusters() >= 65_525, "{} clusters", bpb.clusters());
        assert!(bpb.root_cluster() >= fat32::FIRST_DATA_CLUSTER);
    }

    /// `libs/fat32` reads what `fatfs` wrote, byte for byte.
    ///
    /// The oracle. Two independent implementations of one format over one
    /// corpus, and the disagreement is the failure — which is a better
    /// arrangement than depending on the mature one, because a dependency
    /// cannot disagree with itself (`012` §1, D-170).
    #[test]
    fn the_reader_reads_what_the_writer_wrote() {
        let bytes = build().expect("a volume");
        let mut volume = fat32::Volume::mount(Bytes(&bytes), 0).expect("mounting the corpus");

        // The small file, whole.
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

        // The large one, whole, which walks the chain.
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

        // And a directory below the root.
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
    ///
    /// The property a chain walk breaks silently: reading from offset zero
    /// works whatever the walk does, because the first cluster is the entry's
    /// own. Only a read that has to *skip* clusters exercises it, and only a
    /// file whose bytes differ by position can tell a skip of three from a
    /// skip of four.
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

    /// A read stops at the end of the file rather than at the end of a cluster.
    ///
    /// The last cluster's tail holds whatever its previous occupant left, and a
    /// reader bounded by the cluster rather than by the size hands that back.
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

        // Past the end is nothing at all, rather than the next file.
        let past = volume
            .read_file(&small, small.size, &mut greedy)
            .expect("readable");
        assert_eq!(past, 0);
    }

    /// Every file the corpus holds is listed, and nothing else is.
    ///
    /// The directory walk seen from the other side: a reader that stopped at
    /// the first deleted entry, or that returned long-name fragments, would
    /// produce a different set and every lookup above would still pass.
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
