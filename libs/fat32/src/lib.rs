//! A read-only FAT32 reader over any source of sectors.
//!
//! The sector source is a trait so the same parser runs over a host corpus and
//! a kernel controller (D-170). The reader allocates nothing and treats disk
//! bytes as hostile: layouts and cluster numbers are checked, reserved FAT bits
//! are masked, and every chain walk is bounded.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

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

const MIN_FAT32_CLUSTERS: u64 = 65_525;
const FIRST_RESERVED_CLUSTER: u32 = 0x0FFF_FFF0;
const MAX_FAT32_CLUSTERS: u64 = FIRST_RESERVED_CLUSTER as u64 - FIRST_DATA_CLUSTER as u64;
const MIRRORING_DISABLED: u16 = 1 << 7;
const ACTIVE_FAT_MASK: u16 = 0x000F;
const EXTENDED_FLAGS_MASK: u16 = MIRRORING_DISABLED | ACTIVE_FAT_MASK;
const LOWERCASE_BASE: u8 = 1 << 3;
const LOWERCASE_EXTENSION: u8 = 1 << 4;
const CASE_FLAGS: u8 = LOWERCASE_BASE | LOWERCASE_EXTENSION;

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
/// Host fixtures and kernel controllers implement the same boundary (D-170).
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
    /// The boot fields describe an impossible layout.
    Malformed,
    /// A cluster number outside the data area.
    BadCluster(u32),
    /// A chain that did not end, or a directory that did not.
    Unbounded,
    /// The cluster chain ended before the file size it advertises.
    Truncated,
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
    active_fat: u32,
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
    /// two, and [`FatError::Malformed`] when the fields cannot describe a
    /// complete FAT32 layout.
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
        let root_entries = u16::from_le_bytes([sector[17], sector[18]]);
        // The sixteen-bit count at 22 is zero on FAT32; the real one is at 36.
        // Taking the small one divides by zero or overlays the tables.
        let sectors_per_fat = u32::from_le_bytes([sector[36], sector[37], sector[38], sector[39]]);
        let extended_flags = u16::from_le_bytes([sector[40], sector[41]]);
        let selector = extended_flags & ACTIVE_FAT_MASK;
        if extended_flags & !EXTENDED_FLAGS_MASK != 0
            || (extended_flags & MIRRORING_DISABLED == 0 && selector != 0)
        {
            return Err(FatError::Malformed);
        }
        let active_fat = if extended_flags & MIRRORING_DISABLED == 0 {
            0
        } else {
            selector as u32
        };
        let root_cluster = u32::from_le_bytes([sector[44], sector[45], sector[46], sector[47]]);
        let small_total = u16::from_le_bytes([sector[19], sector[20]]) as u32;
        let small_fat = u16::from_le_bytes([sector[22], sector[23]]);
        let large_total = u32::from_le_bytes([sector[32], sector[33], sector[34], sector[35]]);
        let version = u16::from_le_bytes([sector[42], sector[43]]);
        let total_sectors = large_total;

        if reserved_sectors == 0
            || fats == 0
            || root_entries != 0
            || small_total != 0
            || small_fat != 0
            || version != 0
            || active_fat >= fats
            || root_cluster < FIRST_DATA_CLUSTER
            || total_sectors == 0
        {
            return Err(FatError::Malformed);
        }

        let first_data_sector = reserved_sectors as u64 + fats as u64 * sectors_per_fat as u64;
        if first_data_sector >= total_sectors as u64 {
            return Err(FatError::Malformed);
        }
        let data_clusters = (total_sectors as u64 - first_data_sector) / sectors_per_cluster as u64;
        if data_clusters < MIN_FAT32_CLUSTERS || data_clusters > MAX_FAT32_CLUSTERS {
            return Err(FatError::Malformed);
        }
        let fat_entries =
            sectors_per_fat as u64 * SECTOR_BYTES as u64 / core::mem::size_of::<u32>() as u64;
        if data_clusters + FIRST_DATA_CLUSTER as u64 > fat_entries {
            return Err(FatError::Malformed);
        }
        if (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {
            return Err(FatError::Malformed);
        }

        Ok(Self {
            sectors_per_cluster: sectors_per_cluster as u32,
            reserved_sectors,
            fats,
            sectors_per_fat,
            active_fat,
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
    const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {
        if !self.holds(cluster) {
            return Err(FatError::BadCluster(cluster));
        }
        let index = (cluster - FIRST_DATA_CLUSTER) as u64;
        Ok(self.first_data_sector() + index * self.sectors_per_cluster as u64)
    }

    const fn table_position(self, cluster: u32) -> (u64, usize) {
        let byte = cluster as u64 * core::mem::size_of::<u32>() as u64;
        (
            self.reserved_sectors as u64
                + self.active_fat as u64 * self.sectors_per_fat as u64
                + byte / SECTOR_BYTES as u64,
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
///
const fn decode_entry(raw: u32) -> Next {
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
    const EMPTY: Self = Self {
        name: [0; 12],
        name_len: 0,
        attributes: 0,
        cluster: 0,
        size: 0,
    };

    /// The name, as bytes.
    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_len]
    }

    /// An entry that names contents and nothing else.
    ///
    /// Its name is empty because the caller retained only cluster and size.
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
        wanted
            .iter()
            .zip(self.name())
            .all(|(wanted, actual)| upper(*wanted) == upper(*actual))
    }
}

/// Progress through a directory chain, retained between bounded reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryCursor {
    cluster: u32,
    sector: u32,
    visited: u32,
    done: bool,
}

impl DirectoryCursor {
    /// Start reading the directory whose first cluster is `cluster`.
    pub const fn new(cluster: u32) -> Self {
        Self {
            cluster,
            sector: 0,
            visited: 0,
            done: false,
        }
    }

    /// Whether the directory has reached an end marker or end of chain.
    pub const fn is_done(&self) -> bool {
        self.done
    }
}

/// Entries copied from at most one directory sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryChunk {
    entries: [Entry; SECTOR_BYTES / ENTRY_BYTES],
    len: usize,
}

impl DirectoryChunk {
    const EMPTY: Self = Self {
        entries: [Entry::EMPTY; SECTOR_BYTES / ENTRY_BYTES],
        len: 0,
    };

    /// The live short entries in their on-disk order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries[..self.len]
    }
}

const fn upper(byte: u8) -> u8 {
    if byte.is_ascii_lowercase() {
        byte - 32
    } else {
        byte
    }
}

const fn lower(byte: u8) -> u8 {
    if byte.is_ascii_uppercase() {
        byte + 32
    } else {
        byte
    }
}

const fn valid_short_name_byte(byte: u8) -> bool {
    if byte >= 0x80 {
        return true;
    }
    if byte < 0x20 || byte == 0x7F {
        return false;
    }
    !matches!(
        byte,
        b'"' | b'*'
            | b'+'
            | b','
            | b'.'
            | b'/'
            | b':'
            | b';'
            | b'<'
            | b'='
            | b'>'
            | b'?'
            | b'['
            | b'\\'
            | b']'
            | b'|'
    )
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
const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {
    if slot[0] == 0x00 {
        return Slot::End;
    }
    if slot[0] == 0xE5 {
        return Slot::Deleted;
    }

    let attributes = slot[11];
    if attributes & 0xC0 != 0 {
        return Slot::Skip;
    }
    let case = slot[12];
    if case & !CASE_FLAGS != 0 {
        return Slot::Skip;
    }
    // Long-name fragments carry the volume-label bit, so this excludes both.
    if attributes & attribute::VOLUME_LABEL != 0 {
        return Slot::Skip;
    }

    let mut base_len = 0;
    while base_len < 8 && slot[base_len] != b' ' {
        base_len += 1;
    }
    let mut index = base_len;
    while index < 8 {
        if slot[index] != b' ' {
            return Slot::Skip;
        }
        index += 1;
    }
    if base_len == 0 {
        return Slot::Skip;
    }

    let mut extension_len = 0;
    while extension_len < 3 && slot[8 + extension_len] != b' ' {
        extension_len += 1;
    }
    index = extension_len;
    while index < 3 {
        if slot[8 + index] != b' ' {
            return Slot::Skip;
        }
        index += 1;
    }

    let mut name = [0u8; 12];
    let mut len = 0;
    index = 0;
    while index < base_len {
        let byte = if index == 0 && slot[index] == 0x05 {
            0xE5
        } else {
            slot[index]
        };
        if !valid_short_name_byte(byte) {
            return Slot::Skip;
        }
        // NTRes controls presentation; matching remains case-insensitive so
        // the spelling enumeration returns is always traversable.
        let canonical = upper(byte);
        name[len] = if case & LOWERCASE_BASE != 0 {
            lower(canonical)
        } else {
            canonical
        };
        len += 1;
        index += 1;
    }
    if extension_len != 0 {
        name[len] = b'.';
        len += 1;
        index = 0;
        while index < extension_len {
            if !valid_short_name_byte(slot[8 + index]) {
                return Slot::Skip;
            }
            let canonical = upper(slot[8 + index]);
            name[len] = if case & LOWERCASE_EXTENSION != 0 {
                lower(canonical)
            } else {
                canonical
            };
            len += 1;
            index += 1;
        }
    }
    let high = u16::from_le_bytes([slot[20], slot[21]]) as u32;
    if high & 0xF000 != 0 {
        return Slot::Skip;
    }
    let low = u16::from_le_bytes([slot[26], slot[27]]) as u32;
    Slot::Entry(Entry {
        name,
        name_len: len,
        attributes,
        // Two halves, not adjacent: the high word is at 20 and the low at 26.
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

impl<S: Sectors> Volume<S> {
    /// Read the boot sector at `base` and mount what it describes.
    ///
    /// # Errors
    ///
    /// See [`FatError`].
    pub fn mount(mut source: S, base: u64) -> Result<Self, FatError> {
        let mut sector = [0u8; SECTOR_BYTES];
        <S as Sectors>::read_sector(&mut source, base, &mut sector)
            .map_err(|_| FatError::Unreadable)?;
        let bpb = Bpb::parse(&sector)?;
        Ok(Self { source, base, bpb })
    }

    /// What the boot sector said.
    pub const fn bpb(&self) -> Bpb {
        self.bpb
    }

    fn sector(&mut self, sector: u64, into: &mut [u8; SECTOR_BYTES]) -> Result<(), FatError> {
        let at = self
            .base
            .checked_add(sector)
            .ok_or(FatError::BadCluster(0))?;
        <S as Sectors>::read_sector(&mut self.source, at, into).map_err(|_| FatError::Unreadable)
    }

    const fn chain_limit(&self) -> u32 {
        self.bpb.clusters()
    }

    /// What follows `cluster`.
    ///
    /// # Errors
    ///
    /// [`FatError::BadCluster`] if either this cluster or the link stored in
    /// its entry lies outside the data area, and [`FatError::Unreadable`] if
    /// the table cannot be read.
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
        let next = decode_entry(raw);
        if let Next::Cluster(cluster) = next
            && !self.bpb.holds(cluster)
        {
            return Err(FatError::BadCluster(cluster));
        }
        Ok(next)
    }

    fn advance_cycle_hare(&mut self, mut cluster: Option<u32>) -> Result<Option<u32>, FatError> {
        for _ in 0..2 {
            cluster = match cluster {
                Some(current) => match self.next_cluster(current)? {
                    Next::Cluster(next) => Some(next),
                    Next::End | Next::Bad | Next::Free => None,
                },
                None => None,
            };
        }
        Ok(cluster)
    }

    /// Read at most one sector of directory entries and advance `cursor`.
    ///
    /// The returned entries own their names. No borrow of the sector source
    /// survives this call, so callers may release a device or controller lock
    /// before inspecting them.
    ///
    /// # Errors
    ///
    /// [`FatError::Unbounded`] if the chain does not end, and whatever a read
    /// refused with.
    pub fn directory_chunk(
        &mut self,
        cursor: &mut DirectoryCursor,
    ) -> Result<DirectoryChunk, FatError> {
        if cursor.done {
            return Ok(DirectoryChunk::EMPTY);
        }

        if cursor.sector >= self.bpb.sectors_per_cluster() {
            match self.next_cluster(cursor.cluster)? {
                Next::Cluster(next) => {
                    let Some(visited) = cursor.visited.checked_add(1) else {
                        return Err(FatError::Unbounded);
                    };
                    if visited >= self.chain_limit() {
                        return Err(FatError::Unbounded);
                    }
                    cursor.cluster = next;
                    cursor.sector = 0;
                    cursor.visited = visited;
                }
                Next::End => {
                    cursor.done = true;
                    return Ok(DirectoryChunk::EMPTY);
                }
                Next::Bad | Next::Free => return Err(FatError::BadCluster(cursor.cluster)),
            }
        }

        let first = self.bpb.sector_of(cursor.cluster)?;
        let mut buffer = [0u8; SECTOR_BYTES];
        self.sector(first + u64::from(cursor.sector), &mut buffer)?;
        cursor.sector += 1;

        let mut chunk = DirectoryChunk::EMPTY;
        for raw in buffer.chunks_exact(ENTRY_BYTES) {
            let mut slot = [0u8; ENTRY_BYTES];
            slot.copy_from_slice(raw);
            match decode_slot(&slot) {
                Slot::End => {
                    cursor.done = true;
                    break;
                }
                Slot::Deleted | Slot::Skip => {}
                Slot::Entry(entry) => {
                    chunk.entries[chunk.len] = entry;
                    chunk.len += 1;
                }
            }
        }
        Ok(chunk)
    }

    /// Find `name` in the directory beginning at `cluster`.
    ///
    /// # Errors
    ///
    /// [`FatError::NotFound`] if it is not there.
    pub fn lookup(&mut self, cluster: u32, name: &[u8]) -> Result<Entry, FatError> {
        let mut cursor = DirectoryCursor::new(cluster);
        loop {
            let chunk = self.directory_chunk(&mut cursor)?;
            for entry in chunk.entries() {
                if entry.is_named(name) {
                    return Ok(*entry);
                }
            }
            if cursor.is_done() {
                return Err(FatError::NotFound);
            }
        }
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

        // Bounded by the file's own size: past it, the last cluster's tail
        // holds the previous occupant's data.
        let wanted = into.len().min((entry.size - offset) as usize);
        let cluster_bytes = self.bpb.cluster_bytes();

        let mut current = entry.cluster;
        let mut cycle_hare = Some(current);
        let skip_clusters = offset / cluster_bytes;
        if skip_clusters >= self.chain_limit() {
            return Err(FatError::Unbounded);
        }
        let mut visited = 0u32;
        for _ in 0..skip_clusters {
            visited += 1;
            if visited >= self.chain_limit() {
                return Err(FatError::Unbounded);
            }
            match self.next_cluster(current)? {
                Next::Cluster(next) => {
                    cycle_hare = self.advance_cycle_hare(cycle_hare)?;
                    if cycle_hare == Some(next) {
                        return Err(FatError::Unbounded);
                    }
                    current = next;
                }
                Next::End => return Err(FatError::Truncated),
                Next::Bad | Next::Free => return Err(FatError::BadCluster(current)),
            }
        }

        let mut done = 0;
        let mut within = (offset % cluster_bytes) as usize;

        loop {
            let first = self.bpb.sector_of(current)?;
            let mut sector_index = within / SECTOR_BYTES;
            let mut byte = within % SECTOR_BYTES;

            while sector_index < self.bpb.sectors_per_cluster() as usize {
                let mut buffer = [0u8; SECTOR_BYTES];
                self.sector(first + sector_index as u64, &mut buffer)?;
                let take = (SECTOR_BYTES - byte).min(wanted - done);
                into[done..done + take].copy_from_slice(&buffer[byte..byte + take]);
                done += take;
                if done == wanted {
                    break;
                }
                byte = 0;
                sector_index += 1;
            }
            within = 0;

            if done == wanted {
                break;
            }
            visited += 1;
            if visited >= self.chain_limit() {
                return Err(FatError::Unbounded);
            }
            match self.next_cluster(current)? {
                Next::Cluster(next) => {
                    cycle_hare = self.advance_cycle_hare(cycle_hare)?;
                    if cycle_hare == Some(next) {
                        return Err(FatError::Unbounded);
                    }
                    current = next;
                }
                Next::End => return Err(FatError::Truncated),
                Next::Bad | Next::Free => return Err(FatError::BadCluster(current)),
            }
        }

        Ok(done)
    }

    /// Resolve a path of components from the root, and return what it names.
    ///
    /// Every component but the last must be a directory. There is **no
    /// leading separator and no `..`** — callers provide already-validated
    /// components (`012` §5).
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
}

#[cfg(test)]
mod tests {
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
        // Pairwise-distinct fields expose swaps in the decoder (D-136).
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
        let offset = cluster as usize * core::mem::size_of::<u32>();
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
        assert_eq!(volume.source.reads, [2, 3, 1, 4]);

        let fourth = volume.directory_chunk(&mut cursor).expect("last sector");
        assert_eq!(fourth.entries().len(), 1);
        assert_eq!(fourth.entries()[0].name(), b"OMEGA.TXT");
        assert_eq!((cursor.cluster, cursor.sector, cursor.visited), (3, 2, 1));

        let end = volume.directory_chunk(&mut cursor).expect("end of chain");
        assert!(end.entries().is_empty());
        assert!(cursor.is_done());
        assert_eq!(volume.source.reads, [2, 3, 1, 4, 5, 1]);

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
    fn directory_chunks_retain_the_volume_derived_cycle_bound() {
        let bpb = tiny(5);
        let mut volume = Volume {
            source: CountedCycle {
                reads: 0,
                ceiling: bpb.clusters() * 4,
            },
            base: 0,
            bpb,
        };
        let mut cursor = DirectoryCursor::new(FIRST_DATA_CLUSTER);

        for visited in 0..bpb.clusters() {
            let chunk = volume
                .directory_chunk(&mut cursor)
                .expect("one bounded cluster");
            assert!(chunk.entries().is_empty());
            assert_eq!(cursor.visited, visited);
        }
        let before = cursor;
        assert_eq!(
            volume.directory_chunk(&mut cursor),
            Err(FatError::Unbounded)
        );
        assert_eq!(cursor, before, "a refused transition changed the cursor");
        assert_eq!(cursor.visited, bpb.clusters() - 1);
        assert_eq!(volume.source.reads, bpb.clusters() * 2);
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
            let mut actual = vec![0xA5; requested];
            let read = volume
                .read_file(&entry, offset as u32, &mut actual)
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
                &actual[..read],
                expected,
                "offset={offset}, requested={requested}"
            );
            assert!(
                actual[read..].iter().all(|byte| *byte == 0xA5),
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
                // Every slot deleted: the directory never reaches its end, so
                // only the chain bound can stop the walk.
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
    fn a_cyclic_chain_is_refused_within_the_volume() {
        let bpb = tiny(62);
        assert_eq!(bpb.clusters(), 62);
        // Four reads per cluster the volume holds is far more rope than a
        // bounded walk needs and far less than a fixed cap consumes.
        let ceiling = bpb.clusters() * 4;

        let mut volume = Volume {
            source: CountedCycle { reads: 0, ceiling },
            base: 0,
            bpb,
        };
        assert_eq!(
            directory_entries(&mut volume, FIRST_DATA_CLUSTER),
            Err(FatError::Unbounded),
            "the walk ran past every cluster the volume holds"
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
            "the skip walked a cycle with no bound at all"
        );
    }
}
