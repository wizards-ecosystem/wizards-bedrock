//! The filesystem, such as it is: a FAT32 volume on the one controller.
//!
//! `libs/fat32` is the reader and it is generic over a source of sectors. This
//! is that source, backed by the NVMe driver — and the *same* reader runs on
//! the host over a byte slice, which is what makes the oracle possible (D-170).
//!
//! This path is read-only: the third-party volume therefore carries no
//! on-disk-write risk, and nothing here writes (`014`).

use fat32::{DirectoryChunk, DirectoryCursor, Entry, FatError, Sectors, Volume};

/// Where the volume begins on the namespace, in sectors.
///
/// A megabyte in, which is where a partition table would put it. Transcribed
/// from `xtask`, which writes the volume there, and a host test compares the
/// two (D-158).
pub(crate) const VOLUME_SECTOR: u64 = 2048;

struct Blocks;

impl Sectors for Blocks {
    type Error = ();

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; fat32::SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        crate::nvme::read_sector(sector, into)
    }
}

/// One mounted read-only volume without controller ownership.
pub(crate) struct Mounted {
    volume: Volume<Blocks>,
}

impl Mounted {
    /// Cluster count and bytes per cluster reported by the mounted layout.
    pub(crate) const fn shape(&self) -> (u32, u32) {
        (
            self.volume.bpb().clusters(),
            self.volume.bpb().cluster_bytes(),
        )
    }

    /// Whether the mounted layout retains the FAT32 bounds proved at parse time.
    #[cfg(feature = "selftest")]
    pub(crate) const fn layout_is_fat32(&self) -> bool {
        let bpb = self.volume.bpb();
        bpb.root_cluster() >= fat32::FIRST_DATA_CLUSTER
            && bpb.clusters() >= 65_525
            && bpb.cluster_bytes() >= fat32::SECTOR_BYTES as u32
            && bpb.first_data_sector() > 0
    }

    /// Root directory cluster.
    pub(crate) const fn root_cluster(&self) -> u32 {
        self.volume.bpb().root_cluster()
    }

    /// Resolve every component within this one mounted layout.
    pub(crate) fn resolve(&mut self, from: u32, components: &[&[u8]]) -> Result<Entry, FatError> {
        let mut directory = from;
        for (index, name) in components.iter().enumerate() {
            let entry = self.volume.lookup(directory, name)?;
            if index + 1 == components.len() {
                return Ok(entry);
            }
            if !entry.is_directory() {
                return Err(FatError::WrongKind);
            }
            directory = entry.cluster;
        }
        Err(FatError::NotFound)
    }

    /// Read one file range.
    pub(crate) fn read_file(
        &mut self,
        cluster: u32,
        size: u32,
        offset: u32,
        into: &mut [u8],
    ) -> Result<usize, FatError> {
        self.volume
            .read_file(&Entry::for_contents(cluster, size), offset, into)
    }

    /// Copy the next bounded directory chunk.
    pub(crate) fn directory_chunk(
        &mut self,
        cursor: &mut DirectoryCursor,
    ) -> Result<DirectoryChunk, FatError> {
        self.volume.directory_chunk(cursor)
    }
}

/// Mount the retained controller's volume.
///
/// Mounted per call rather than kept, which is the same decision as "no cache"
/// and for the same reason: a boot sector is one sector, the volume is not
/// going anywhere, and holding a mounted volume in a `static` would mean
/// holding the controller's borrow for the life of the boot.
///
/// `None` means no controller; `Some(Err(..))` means a malformed volume.
pub(crate) fn mount() -> Option<Result<Mounted, FatError>> {
    if !crate::nvme::has_controller() {
        return None;
    }
    Some(Volume::mount(Blocks, VOLUME_SECTOR).map(|volume| Mounted { volume }))
}
