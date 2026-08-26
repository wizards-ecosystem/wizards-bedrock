//! The filesystem, such as it is: a FAT32 volume on the one controller.
//!
//! `libs/fat32` is the reader and it is generic over a source of sectors. This
//! is that source, backed by the NVMe driver — and the *same* reader runs on
//! the host over a byte slice, which is what makes the oracle possible (D-170).
//!
//! **Read-only, and the phase is honest about it** (`014`): the format was
//! chosen partly because a third-party volume carries zero on-disk-write risk,
//! and nothing here writes.
//!
//! **Sector by sector, with no cache.** `014` carries "no page cache" as a
//! deliberate hole: reading a directory means reading the same sectors
//! repeatedly, and whether that is affordable is a measurement this phase
//! should take rather than assume. The measurement does not exist yet, so
//! neither does the cache.

use fat32::{FatError, Sectors, Volume};
use wost::mm::Segment;

use crate::nvme::Controller;

/// Where the volume begins on the namespace, in sectors.
///
/// A megabyte in, which is where a partition table would put it. Transcribed
/// from `xtask`, which writes the volume there, and a host test compares the
/// two (D-158).
pub(crate) const VOLUME_SECTOR: u64 = 2048;

/// A source of sectors backed by the controller.
///
/// Holds the driver and its bounce buffer by reference, because both live in
/// the `static` the driver is kept in and neither can be moved out.
pub(crate) struct Blocks<'a> {
    controller: &'a mut Controller,
    buffer: &'a mut Segment,
}

impl Sectors for Blocks<'_> {
    type Error = ();

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; fat32::SECTOR_BYTES],
    ) -> Result<(), Self::Error> {
        // One logical block per sector, which holds because the reader refuses
        // any volume whose sector size is not 512 and the namespace reports
        // 512. A controller formatted at 4096 would be refused at the mount
        // rather than misread here.
        if self.controller.block_bytes as usize != fat32::SECTOR_BYTES {
            return Err(());
        }
        self.controller
            .read_blocks(sector, 1, self.buffer)
            .map_err(|_| ())?;
        if !self.buffer.read_at(0, into) {
            return Err(());
        }
        Ok(())
    }
}

/// Mount the volume and run `f` against it.
///
/// Mounted per call rather than kept, which is the same decision as "no cache"
/// and for the same reason: a boot sector is one sector, the volume is not
/// going anywhere, and holding a mounted volume in a `static` would mean
/// holding the controller's borrow for the life of the boot.
///
/// `None` when there is no controller. `Some(Err(..))` when there is one and
/// the volume will not mount, which is a different thing and worth telling
/// apart.
pub(crate) fn with_volume<T>(
    f: impl FnOnce(&mut Volume<Blocks<'_>>) -> Result<T, FatError>,
) -> Option<Result<T, FatError>> {
    crate::nvme::with_controller(|controller, buffer| {
        let source = Blocks { controller, buffer };
        let mut volume = Volume::mount(source, VOLUME_SECTOR)?;
        f(&mut volume)
    })
}
