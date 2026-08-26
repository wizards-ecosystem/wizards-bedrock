//! A read-only FAT32 reader over any source of sectors.
//!
//! **The format was chosen on the corpus, not on the code** (`012` §1).
//! Measured against GRUB's own two readers — one project, one style, both
//! read-only — FAT is 673 lines to ext2's ~772. Thirteen per cent. Nobody
//! should start here believing FAT is dramatically simpler, because it is not,
//! and its traps are differently shaped rather than fewer.
//!
//! What decided it is that `xtask` already builds a FAT32 volume through
//! `fatfs`, and two volumes built seconds apart hash identically. Every
//! pure-Rust ext2 image builder fails to run on this machine, and the reference
//! read-only ext2 crate generates its corpus by shelling out to `mkfs.ext2`,
//! which D-16 forbids. So there is **an oracle**: two independent
//! implementations of one format, one of them mature, and the disagreement
//! between them is the test.
//!
//! **The source of sectors is a trait, and that is the whole of the block
//! layer** (D-170). Not a speculative kernel-side abstraction — the parameter
//! this reader needs, which happens to be exactly what makes it testable on the
//! host against a file while the kernel backs it with a controller.
//!
//! **No allocation, and no panic on hostile input.** Every field that could be
//! a length is checked against what remains, every cluster number against the
//! chain's bounds, and a walk that would not terminate is bounded. The bytes
//! come off a disk, and a disk is not trusted.
//!
//! Three traps worth naming before the code:
//!
//! - **A cluster number is biased by two.** Clusters 0 and 1 are the media
//!   descriptor and the end-of-chain marker; data starts at cluster 2, so the
//!   sector of a cluster is `first_data + (cluster - 2) * per_cluster`. Forget
//!   the bias and every file is read two clusters late.
//! - **A FAT32 entry is twenty-eight bits.** The top four are reserved and a
//!   reader that compares all thirty-two against the end-of-chain marker
//!   follows a chain that has ended.
//! - **A directory ends at the first entry whose name begins with zero**, and
//!   an entry beginning `0xE5` is deleted but not the end. Confusing the two
//!   either stops early or resurrects deleted files.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

/// Bytes in the only sector size this reader supports.
///
/// Five hundred and twelve, and a volume that says otherwise is refused rather
/// than misread. It is what lets every buffer here be a fixed array on the
/// stack, which is what "no allocation" costs: a reader that supported 4096
/// would need either a caller-supplied buffer everywhere or an allocator, and
/// no volume this project builds or boots uses one.
pub const SECTOR_BYTES: usize = 512;

/// Bytes in one directory entry.
pub const ENTRY_BYTES: usize = 32;

/// The lowest cluster number that names data.
///
/// Zero is the media descriptor and one is the end-of-chain marker, so the
/// data area begins at two and every cluster is biased by it.
pub const FIRST_DATA_CLUSTER: u32 = 2;

/// A FAT32 entry is twenty-eight bits; the top four are reserved.
pub const ENTRY_MASK: u32 = 0x0FFF_FFFF;

/// At or above this, the chain has ended.
pub const END_OF_CHAIN: u32 = 0x0FFF_FFF8;

/// A cluster the volume has marked unusable.
pub const BAD_CLUSTER: u32 = 0x0FFF_FFF7;

/// Directory entry attributes.
pub mod attribute {
    /// Read only.
    pub const READ_ONLY: u8 = 0x01;
    /// Hidden.
    pub const HIDDEN: u8 = 0x02;
    /// A system file.
    pub const SYSTEM: u8 = 0x04;
    /// The volume label, which lives in the root directory and is not a file.
    pub const VOLUME_LABEL: u8 = 0x08;
    /// A directory.
    pub const DIRECTORY: u8 = 0x10;
    /// Archive.
    pub const ARCHIVE: u8 = 0x20;
    /// The combination that marks a long-name fragment rather than an entry.
    ///
    /// All four of read-only, hidden, system and volume-label at once, which
    /// no real entry ever is. A reader that did not skip these would return
    /// fragments of UCS-2 as though they were 8.3 names.
    pub const LONG_NAME: u8 = READ_ONLY | HIDDEN | SYSTEM | VOLUME_LABEL;
}

/// Where sectors come from.
///
/// The block layer, and it is a trait because that is what this reader needs
/// rather than because a kernel might one day want one (D-41, D-170). The host
/// backs it with a file and the kernel with a controller, and the *same*
/// reader runs over both — which is what makes an oracle possible at all.
pub trait Sectors {
    /// What can go wrong down there.
    type Error;

    /// Read one sector into `into`.
    ///
    /// # Errors
    ///
    /// Whatever the source reports. A reader above this treats any error as
    /// "the volume is unreadable" and stops.
    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error>;
}

/// Why a volume could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatError {
    /// The source refused a read.
    Unreadable,
    /// The boot sector does not end `0x55 0xAA`.
    NotABootSector,
    /// The sector size is not [`SECTOR_BYTES`].
    SectorSize(u16),
    /// Sectors per cluster is zero or not a power of two.
    ClusterSize(u8),
    /// A field that must not be zero is.
    Malformed,
    /// A cluster number outside the data area.
    BadCluster(u32),
    /// A chain that did not end, or a directory that did not.
    Unbounded,
    /// The name asked for is not in that directory.
    NotFound,
    /// The entry found is not the kind asked for.
    WrongKind,
}

/// The BIOS parameter block, which is the only thing that says where anything
/// is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bpb {
    sectors_per_cluster: u32,
    reserved_sectors: u32,
    fats: u32,
    sectors_per_fat: u32,
    root_cluster: u32,
    total_sectors: u32,
}

impl Bpb {
    /// Parse a boot sector.
    ///
    /// # Errors
    ///
    /// [`FatError::NotABootSector`] without the signature,
    /// [`FatError::SectorSize`] for anything but 512,
    /// [`FatError::ClusterSize`] for a cluster that is zero or not a power of
    /// two, and [`FatError::Malformed`] for a zero where the layout needs a
    /// count.
    pub const fn parse(sector: &[u8; SECTOR_BYTES]) -> Result<Self, FatError> {
        if sector[510] != 0x55 || sector[511] != 0xAA {
            return Err(FatError::NotABootSector);
        }

        let bytes_per_sector = u16::from_le_bytes([sector[11], sector[12]]);
        if bytes_per_sector as usize != SECTOR_BYTES {
            return Err(FatError::SectorSize(bytes_per_sector));
        }

        let sectors_per_cluster = sector[13];
        if sectors_per_cluster == 0 || !sectors_per_cluster.is_power_of_two() {
            return Err(FatError::ClusterSize(sectors_per_cluster));
        }

        let reserved_sectors = u16::from_le_bytes([sector[14], sector[15]]) as u32;
        let fats = sector[16] as u32;
        // The sixteen-bit count at 22 is zero on FAT32; the real one is the
        // thirty-two-bit field at 36. A reader that took the small one divides
        // by zero or lands the data area on top of the tables.
        let sectors_per_fat = u32::from_le_bytes([sector[36], sector[37], sector[38], sector[39]]);
        let root_cluster = u32::from_le_bytes([sector[44], sector[45], sector[46], sector[47]]);
        let small_total = u16::from_le_bytes([sector[19], sector[20]]) as u32;
        let large_total = u32::from_le_bytes([sector[32], sector[33], sector[34], sector[35]]);
        let total_sectors = if small_total == 0 {
            large_total
        } else {
            small_total
        };

        if reserved_sectors == 0
            || fats == 0
            || sectors_per_fat == 0
            || root_cluster < FIRST_DATA_CLUSTER
            || total_sectors == 0
        {
            return Err(FatError::Malformed);
        }

        Ok(Self {
            sectors_per_cluster: sectors_per_cluster as u32,
            reserved_sectors,
            fats,
            sectors_per_fat,
            root_cluster,
            total_sectors,
        })
    }

    /// Sectors in one cluster.
    pub const fn sectors_per_cluster(self) -> u32 {
        self.sectors_per_cluster
    }

    /// Bytes in one cluster.
    pub const fn cluster_bytes(self) -> u32 {
        self.sectors_per_cluster * SECTOR_BYTES as u32
    }

    /// The cluster the root directory begins at.
    pub const fn root_cluster(self) -> u32 {
        self.root_cluster
    }

    /// The first sector of the data area.
    ///
    /// Past the reserved sectors and every copy of the table.
    pub const fn first_data_sector(self) -> u64 {
        self.reserved_sectors as u64 + self.fats as u64 * self.sectors_per_fat as u64
    }

    /// Clusters the data area holds.
    pub const fn clusters(self) -> u32 {
        let data = self.total_sectors as u64 - self.first_data_sector();
        (data / self.sectors_per_cluster as u64) as u32
    }

    /// Whether `cluster` names data in this volume.
    ///
    /// Both ends: below two is a marker rather than data, and past the count
    /// is off the volume. A chain that walks off the end is what a corrupt or
    /// hostile table produces, and following it reads whatever is there.
    pub const fn holds(self, cluster: u32) -> bool {
        cluster >= FIRST_DATA_CLUSTER && cluster < FIRST_DATA_CLUSTER + self.clusters()
    }

    /// The first sector of `cluster`.
    ///
    /// # Errors
    ///
    /// [`FatError::BadCluster`] for a cluster this volume does not hold.
    pub const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {
        if !self.holds(cluster) {
            return Err(FatError::BadCluster(cluster));
        }
        // **Biased by two.** Clusters zero and one are markers, so the data
        // area's first cluster is two — and a reader that forgets it reads
        // every file two clusters late.
        let index = (cluster - FIRST_DATA_CLUSTER) as u64;
        Ok(self.first_data_sector() + index * self.sectors_per_cluster as u64)
    }

    /// Where `cluster`'s entry lives in the table: which sector, and where in
    /// it.
    pub const fn table_position(self, cluster: u32) -> (u64, usize) {
        let byte = cluster as u64 * 4;
        (
            self.reserved_sectors as u64 + byte / SECTOR_BYTES as u64,
            (byte % SECTOR_BYTES as u64) as usize,
        )
    }
}

/// What the table says follows `cluster`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Another cluster.
    Cluster(u32),
    /// The chain ends here.
    End,
    /// The volume marked this cluster unusable.
    Bad,
    /// The entry is free, which inside a chain means the volume is corrupt.
    Free,
}

/// Decode one table entry.
///
/// **Twenty-eight bits.** The top four are reserved, and a reader that
/// compared all thirty-two against the end-of-chain marker would follow a
/// chain that has ended — into whatever those bits happened to be.
pub const fn decode_entry(raw: u32) -> Next {
    let value = raw & ENTRY_MASK;
    if value == 0 {
        Next::Free
    } else if value == BAD_CLUSTER {
        Next::Bad
    } else if value >= END_OF_CHAIN {
        Next::End
    } else {
        Next::Cluster(value)
    }
}

/// One directory entry, with its 8.3 name unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The name, without padding, upper case, with the dot restored.
    name: [u8; 12],
    /// How many of those bytes are the name.
    name_len: usize,
    /// The attribute byte.
    pub attributes: u8,
    /// The first cluster of its contents.
    pub cluster: u32,
    /// Bytes in the file. Zero for a directory, which FAT does not size.
    pub size: u32,
}

impl Entry {
    /// The name, as bytes.
    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len]
    }

    /// An entry that names contents and nothing else.
    ///
    /// For a caller that kept a cluster and a size rather than a whole
    /// directory entry — a VFS node is not a copy of the entry it came from,
    /// and carrying the name would make it one. The name is empty, which is
    /// the only honest thing to put there.
    pub const fn for_contents(cluster: u32, size: u32) -> Self {
        Self {
            name: [0u8; 12],
            name_len: 0,
            attributes: attribute::ARCHIVE,
            cluster,
            size,
        }
    }

    /// Whether this entry is a directory.
    pub const fn is_directory(&self) -> bool {
        self.attributes & attribute::DIRECTORY != 0
    }

    /// Whether the name matches, ignoring case.
    ///
    /// FAT stores 8.3 names upper-cased, so a caller that asked for `hello.txt`
    /// and compared byte for byte would never find `HELLO.TXT`.
    pub fn is_named(&self, wanted: &[u8]) -> bool {
        if wanted.len() != self.name_len {
            return false;
        }
        let mut index = 0;
        while index < wanted.len() {
            if upper(wanted[index]) != self.name[index] {
                return false;
            }
            index += 1;
        }
        true
    }
}

/// Upper-case one ASCII byte.
const fn upper(byte: u8) -> u8 {
    if byte.is_ascii_lowercase() {
        byte - 32
    } else {
        byte
    }
}

/// What a thirty-two byte slot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// A usable entry.
    Entry(Entry),
    /// Deleted, and the directory continues past it.
    Deleted,
    /// A long-name fragment, or the volume label: skip it.
    Skip,
    /// The directory ends here and nothing after it is an entry.
    End,
}

/// Decode one directory slot.
///
/// The two markers are different things and confusing them is the classic
/// mistake: **zero ends the directory**, and `0xE5` is one deleted entry with
/// live entries possibly after it. A reader that stopped on `0xE5` loses files;
/// one that continued past zero reads whatever the rest of the cluster holds.
pub const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {
    if slot[0] == 0x00 {
        return Slot::End;
    }
    if slot[0] == 0xE5 {
        return Slot::Deleted;
    }

    let attributes = slot[11];
    // The long-name combination is checked whole. Testing only the volume-label
    // bit would skip a fragment *and* the volume label, which is right by
    // accident; testing only `LONG_NAME` misses the label, which is not a file
    // either.
    if attributes & attribute::LONG_NAME == attribute::LONG_NAME
        || attributes & attribute::VOLUME_LABEL != 0
    {
        return Slot::Skip;
    }

    let mut name = [0u8; 12];
    let mut len = 0;

    // The stem: eight bytes, space-padded on the right.
    let mut index = 0;
    while index < 8 {
        if slot[index] != b' ' {
            name[len] = slot[index];
            len += 1;
        }
        index += 1;
    }

    // The extension: three more, and the dot is *not* stored.
    let mut extension = 0;
    let mut has_extension = false;
    while extension < 3 {
        if slot[8 + extension] != b' ' {
            has_extension = true;
        }
        extension += 1;
    }
    if has_extension {
        name[len] = b'.';
        len += 1;
        extension = 0;
        while extension < 3 {
            if slot[8 + extension] != b' ' {
                name[len] = slot[8 + extension];
                len += 1;
            }
            extension += 1;
        }
    }

    let high = u16::from_le_bytes([slot[20], slot[21]]) as u32;
    let low = u16::from_le_bytes([slot[26], slot[27]]) as u32;
    Slot::Entry(Entry {
        name,
        name_len: len,
        attributes,
        // **Two halves, sixteen bytes apart.** The high word is at 20 and the
        // low at 26, which is the layout's oddest corner and the one a reader
        // gets wrong by assuming they are adjacent.
        cluster: (high << 16) | low,
        size: u32::from_le_bytes([slot[28], slot[29], slot[30], slot[31]]),
    })
}

/// A mounted volume.
#[derive(Debug)]
pub struct Volume<S: Sectors> {
    source: S,
    /// Where the volume begins on the source, in sectors.
    ///
    /// A partition offset, because a volume is rarely at sector zero and a
    /// reader that assumed it was would be one a partition table could not be
    /// put in front of.
    base: u64,
    bpb: Bpb,
}

/// How many clusters a walk will follow before calling the chain a loop.
///
/// A FAT is a linked list a hostile volume can make circular, and following one
/// is a hang rather than a fault. The bound is the cluster count, because a
/// chain longer than the volume has visited one twice.
const MAX_CHAIN: u32 = 1 << 24;

impl<S: Sectors> Volume<S> {
    /// Read the boot sector at `base` and mount what it describes.
    ///
    /// # Errors
    ///
    /// See [`FatError`].
    pub fn mount(mut source: S, base: u64) -> Result<Self, FatError> {
        let mut sector = [0u8; SECTOR_BYTES];
        source
            .read_sector(base, &mut sector)
            .map_err(|_| FatError::Unreadable)?;
        let bpb = Bpb::parse(&sector)?;
        Ok(Self { source, base, bpb })
    }

    /// What the boot sector said.
    pub const fn bpb(&self) -> Bpb {
        self.bpb
    }

    /// Read one of the volume's sectors.
    fn sector(&mut self, sector: u64, into: &mut [u8; SECTOR_BYTES]) -> Result<(), FatError> {
        let at = self
            .base
            .checked_add(sector)
            .ok_or(FatError::BadCluster(0))?;
        self.source
            .read_sector(at, into)
            .map_err(|_| FatError::Unreadable)
    }

    /// What follows `cluster`.
    ///
    /// # Errors
    ///
    /// [`FatError::BadCluster`] for a cluster outside the data area, and
    /// [`FatError::Unreadable`] if the table cannot be read.
    pub fn next_cluster(&mut self, cluster: u32) -> Result<Next, FatError> {
        if !self.bpb.holds(cluster) {
            return Err(FatError::BadCluster(cluster));
        }
        let (sector, offset) = self.bpb.table_position(cluster);
        let mut buffer = [0u8; SECTOR_BYTES];
        self.sector(sector, &mut buffer)?;
        let raw = u32::from_le_bytes([
            buffer[offset],
            buffer[offset + 1],
            buffer[offset + 2],
            buffer[offset + 3],
        ]);
        Ok(decode_entry(raw))
    }

    /// Call `found` for every entry of the directory beginning at `cluster`,
    /// stopping when it returns `false`.
    ///
    /// # Errors
    ///
    /// [`FatError::Unbounded`] if the chain does not end, and whatever a read
    /// refused with.
    pub fn walk_directory(
        &mut self,
        cluster: u32,
        mut found: impl FnMut(&Entry) -> bool,
    ) -> Result<(), FatError> {
        let mut current = cluster;
        let mut visited = 0u32;

        loop {
            let first = self.bpb.sector_of(current)?;
            for index in 0..u64::from(self.bpb.sectors_per_cluster()) {
                let mut buffer = [0u8; SECTOR_BYTES];
                self.sector(first + index, &mut buffer)?;

                for slot in 0..SECTOR_BYTES / ENTRY_BYTES {
                    let mut raw = [0u8; ENTRY_BYTES];
                    raw.copy_from_slice(&buffer[slot * ENTRY_BYTES..(slot + 1) * ENTRY_BYTES]);
                    match decode_slot(&raw) {
                        Slot::End => return Ok(()),
                        Slot::Deleted | Slot::Skip => {}
                        Slot::Entry(entry) => {
                            if !found(&entry) {
                                return Ok(());
                            }
                        }
                    }
                }
            }

            visited += 1;
            if visited > MAX_CHAIN {
                return Err(FatError::Unbounded);
            }
            match self.next_cluster(current)? {
                Next::Cluster(next) => current = next,
                Next::End => return Ok(()),
                Next::Bad | Next::Free => return Err(FatError::BadCluster(current)),
            }
        }
    }

    /// Find `name` in the directory beginning at `cluster`.
    ///
    /// # Errors
    ///
    /// [`FatError::NotFound`] if it is not there.
    pub fn lookup(&mut self, cluster: u32, name: &[u8]) -> Result<Entry, FatError> {
        let mut found = None;
        self.walk_directory(cluster, |entry| {
            if entry.is_named(name) {
                found = Some(*entry);
                return false;
            }
            true
        })?;
        found.ok_or(FatError::NotFound)
    }

    /// Read up to `into.len()` bytes of `entry`, starting `offset` bytes in.
    ///
    /// Returns how many bytes were read, which is fewer than asked for at the
    /// end of the file and zero past it.
    ///
    /// **Offsets are per call**, which is D-152's rule arriving here: there is
    /// no cursor on anything, and a caller that wants one keeps it.
    ///
    /// # Errors
    ///
    /// [`FatError::WrongKind`] for a directory, and whatever the walk refused
    /// with.
    pub fn read_file(
        &mut self,
        entry: &Entry,
        offset: u32,
        into: &mut [u8],
    ) -> Result<usize, FatError> {
        if entry.is_directory() {
            return Err(FatError::WrongKind);
        }
        if offset >= entry.size || into.is_empty() {
            return Ok(0);
        }

        // Bounded by the file's own size: a read that ran past it would return
        // whatever the last cluster's tail holds, which is the previous
        // occupant's data.
        let wanted = into.len().min((entry.size - offset) as usize);
        let cluster_bytes = self.bpb.cluster_bytes();

        // Walk to the cluster the offset lands in rather than reading from the
        // first: a chain is a linked list and there is no other way to index
        // it.
        let mut current = entry.cluster;
        let mut skipped = 0;
        while skipped + cluster_bytes <= offset {
            match self.next_cluster(current)? {
                Next::Cluster(next) => current = next,
                Next::End => return Ok(0),
                Next::Bad | Next::Free => return Err(FatError::BadCluster(current)),
            }
            skipped += cluster_bytes;
            if skipped > entry.size {
                return Ok(0);
            }
        }

        let mut done = 0;
        let mut within = (offset - skipped) as usize;
        let mut visited = 0u32;

        while done < wanted {
            let first = self.bpb.sector_of(current)?;
            let mut sector_index = within / SECTOR_BYTES;
            let mut byte = within % SECTOR_BYTES;

            while done < wanted && sector_index < self.bpb.sectors_per_cluster() as usize {
                let mut buffer = [0u8; SECTOR_BYTES];
                self.sector(first + sector_index as u64, &mut buffer)?;
                let take = (SECTOR_BYTES - byte).min(wanted - done);
                into[done..done + take].copy_from_slice(&buffer[byte..byte + take]);
                done += take;
                byte = 0;
                sector_index += 1;
            }
            within = 0;

            if done == wanted {
                break;
            }
            visited += 1;
            if visited > MAX_CHAIN {
                return Err(FatError::Unbounded);
            }
            match self.next_cluster(current)? {
                Next::Cluster(next) => current = next,
                Next::End => break,
                Next::Bad | Next::Free => return Err(FatError::BadCluster(current)),
            }
        }

        Ok(done)
    }

    /// Resolve a path of components from the root, and return what it names.
    ///
    /// Every component but the last must be a directory. There is **no
    /// leading separator and no `..`** — this takes the components already
    /// split, and the resolver above it is what refuses those (`012` §5, and
    /// slice 7).
    ///
    /// # Errors
    ///
    /// [`FatError::NotFound`], or [`FatError::WrongKind`] if a component that
    /// must be a directory is not.
    pub fn resolve(&mut self, components: &[&[u8]]) -> Result<Entry, FatError> {
        let mut cluster = self.bpb.root_cluster();
        let mut last = None;

        for (index, component) in components.iter().enumerate() {
            let entry = self.lookup(cluster, component)?;
            if index + 1 < components.len() {
                if !entry.is_directory() {
                    return Err(FatError::WrongKind);
                }
                cluster = entry.cluster;
            }
            last = Some(entry);
        }

        last.ok_or(FatError::NotFound)
    }

    /// Give the source back.
    pub fn into_source(self) -> S {
        self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A boot sector with the fields this reader needs.
    fn boot_sector() -> [u8; SECTOR_BYTES] {
        let mut sector = [0u8; SECTOR_BYTES];
        sector[11..13].copy_from_slice(&512u16.to_le_bytes());
        sector[13] = 8; // sectors per cluster
        sector[14..16].copy_from_slice(&32u16.to_le_bytes()); // reserved
        sector[16] = 2; // fats
        sector[32..36].copy_from_slice(&131_072u32.to_le_bytes()); // total
        sector[36..40].copy_from_slice(&1_024u32.to_le_bytes()); // sectors per fat
        sector[44..48].copy_from_slice(&2u32.to_le_bytes()); // root cluster
        sector[510] = 0x55;
        sector[511] = 0xAA;
        sector
    }

    /// The boot sector's fields are read from where the layout puts them.
    ///
    /// Pairwise distinct values on purpose (D-136): eight, thirty-two, two,
    /// 1024 and 131072 cannot be swapped for one another with this still
    /// passing.
    #[test]
    fn the_boot_sector_is_read_where_the_layout_puts_its_fields() {
        let bpb = Bpb::parse(&boot_sector()).expect("a valid boot sector");

        assert_eq!(bpb.sectors_per_cluster(), 8);
        assert_eq!(bpb.cluster_bytes(), 4096);
        assert_eq!(bpb.root_cluster(), 2);
        // Reserved, then two copies of the table.
        assert_eq!(bpb.first_data_sector(), 32 + 2 * 1024);
        assert_eq!(bpb.clusters(), (131_072 - 2080) / 8);
    }

    /// A boot sector that is not one is refused rather than read.
    ///
    /// Each of these is a real way to arrive at the wrong sector: an unwritten
    /// disk, a partition offset off by one, a FAT16 volume, and a volume with
    /// a sector size this reader's fixed buffers cannot hold.
    #[test]
    fn a_boot_sector_that_is_not_one_is_refused() {
        let mut blank = [0u8; SECTOR_BYTES];
        assert_eq!(Bpb::parse(&blank), Err(FatError::NotABootSector));

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

        // A FAT16 volume: its thirty-two-bit sectors-per-FAT is zero, and a
        // reader that took it would divide the data area by nothing.
        let mut fat16 = boot_sector();
        fat16[36..40].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(Bpb::parse(&fat16), Err(FatError::Malformed));

        // A root cluster below two names a marker, not data.
        let mut rooted = boot_sector();
        rooted[44..48].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(Bpb::parse(&rooted), Err(FatError::Malformed));
    }

    /// A cluster's sector is biased by two, and out-of-range clusters are
    /// refused.
    ///
    /// The bias is the trap: forget it and every file is read two clusters
    /// late, which on a volume whose first file starts at cluster two means
    /// reading the *second* file's data with no error anywhere.
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

    /// A table entry is twenty-eight bits, and the top four are ignored.
    ///
    /// A reader comparing all thirty-two against the end-of-chain marker
    /// follows a chain that has ended — into whatever those reserved bits
    /// happened to be, which is a cluster number and reads as data.
    #[test]
    fn a_table_entry_is_twenty_eight_bits() {
        assert_eq!(decode_entry(0), Next::Free);
        assert_eq!(decode_entry(3), Next::Cluster(3));
        assert_eq!(decode_entry(BAD_CLUSTER), Next::Bad);
        assert_eq!(decode_entry(END_OF_CHAIN), Next::End);
        assert_eq!(decode_entry(0x0FFF_FFFF), Next::End);

        // The reserved bits set, over each of the cases: the answer must not
        // change.
        assert_eq!(decode_entry(0xF000_0000), Next::Free);
        assert_eq!(decode_entry(0xF000_0003), Next::Cluster(3));
        assert_eq!(decode_entry(0xFFFF_FFFF), Next::End);
        assert_eq!(decode_entry(0xFFFF_FFF7), Next::Bad);
    }

    /// A directory slot's two markers mean different things.
    ///
    /// Zero ends the directory; `0xE5` is one deleted entry with live entries
    /// possibly after it. Stopping on `0xE5` loses files and continuing past
    /// zero reads whatever the rest of the cluster holds.
    #[test]
    fn the_two_directory_markers_are_not_the_same_marker() {
        let mut slot = [0u8; ENTRY_BYTES];
        assert_eq!(decode_slot(&slot), Slot::End);

        slot[0] = 0xE5;
        assert_eq!(decode_slot(&slot), Slot::Deleted);
    }

    /// An 8.3 name is unpacked with its padding gone and its dot restored.
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
        // **The two halves are sixteen bytes apart**, high at 20 and low at 26,
        // which is the layout's oddest corner.
        assert_eq!(entry.cluster, 0x0001_0234);
        assert!(!entry.is_directory());

        // A name with no extension keeps no dot.
        let mut bare = [0u8; ENTRY_BYTES];
        bare[0..11].copy_from_slice(b"DATA       ");
        bare[11] = attribute::DIRECTORY;
        let Slot::Entry(directory) = decode_slot(&bare) else {
            panic!("a usable entry");
        };
        assert_eq!(directory.name(), b"DATA");
        assert!(directory.is_directory());
    }

    /// Long-name fragments and the volume label are skipped.
    ///
    /// A fragment is UCS-2 with the attribute bits of four things at once, and
    /// a reader that did not skip it would return half a name as though it
    /// were an 8.3 one.
    #[test]
    fn long_name_fragments_and_the_volume_label_are_skipped() {
        let mut fragment = [0u8; ENTRY_BYTES];
        fragment[0] = 0x41;
        fragment[11] = attribute::LONG_NAME;
        assert_eq!(decode_slot(&fragment), Slot::Skip);

        let mut label = [0u8; ENTRY_BYTES];
        label[0..11].copy_from_slice(b"WIZARD     ");
        label[11] = attribute::VOLUME_LABEL;
        assert_eq!(decode_slot(&label), Slot::Skip);
    }

    /// A name is matched without regard to case.
    ///
    /// FAT stores 8.3 names upper-cased, so a caller asking for `hello.txt`
    /// and comparing byte for byte never finds `HELLO.TXT`.
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
        assert!(!entry.is_named(b"HELLO.TX"), "a prefix is not the name");
        assert!(!entry.is_named(b"HELLO.TXTX"));
        assert!(!entry.is_named(b"GOODBYE.TXT"));
    }
}
