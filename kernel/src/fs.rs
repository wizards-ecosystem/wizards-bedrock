use fat32::{DirectoryChunk, DirectoryCursor, Entry, FatError, Sectors, Volume};

// `xtask` writes the volume a megabyte in, where a partition table would put it.
pub(crate) const VOLUME_SECTOR: u64 = 2048;

#[derive(Debug)]
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

#[derive(Debug)]
pub(crate) struct Mounted {
    volume: Volume<Blocks>,
}

impl Mounted {
    pub(crate) const fn shape(&self) -> (u32, u32) {
        (
            self.volume.bpb().clusters(),
            self.volume.bpb().cluster_bytes(),
        )
    }

    pub(crate) const fn root_cluster(&self) -> u32 {
        self.volume.bpb().root_cluster()
    }

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

    pub(crate) fn directory_chunk(
        &mut self,
        cursor: &mut DirectoryCursor,
    ) -> Result<DirectoryChunk, FatError> {
        self.volume.directory_chunk(cursor)
    }
}

pub(crate) fn mount() -> Option<Result<Mounted, FatError>> {
    let blocks = crate::nvme::namespace_blocks()?;
    Some(
        Volume::mount(Blocks, VOLUME_SECTOR, blocks.saturating_sub(VOLUME_SECTOR))
            .map(|volume| Mounted { volume }),
    )
}
