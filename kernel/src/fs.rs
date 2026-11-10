//! A FAT32 volume on the one controller, read-only: nothing here writes (`014`).
//!
//! The NVMe sector source for `libs/fat32`, whose same reader runs on the host as the oracle.

use fat32::{DirectoryChunk, DirectoryCursor, Entry, FatError, Sectors, Volume};

/// Where the volume begins, in sectors: a megabyte in, where a partition table would put it.
/// `xtask` writes the volume there, and a host test compares the two.
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

/// Mount the retained controller's volume: `None` without a controller, `Some(Err(..))` if it is
/// malformed. Mounted per call, since a kept volume would hold the controller's borrow all boot.
pub(crate) fn mount() -> Option<Result<Mounted, FatError>> {
    if !crate::nvme::has_controller() {
        return None;
    }
    Some(Volume::mount(Blocks, VOLUME_SECTOR).map(|volume| Mounted { volume }))
}
