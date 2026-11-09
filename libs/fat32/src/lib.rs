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

/// The most entries one FAT directory may hold.
///
/// The specification caps a directory at 65,536 entries. Past the clusters
/// that hold them, a chain that has not ended is malformed, so this bounds a
/// directory walk independently of the volume's size (D-352).
const MAX_DIRECTORY_ENTRIES: u32 = 65_536;
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
#[non_exhaustive]
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

impl core::fmt::Display for FatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Unreadable => core::fmt::Write::write_str(f, "the sector source refused a read"),
            Self::NotABootSector => {
                core::fmt::Write::write_str(f, "the boot sector signature is missing")
            }
            Self::SectorSize(bytes) => write!(f, "sector size {bytes} is unsupported"),
            Self::ClusterSize(sectors) => write!(f, "cluster size {sectors} is invalid"),
            Self::Malformed => core::fmt::Write::write_str(f, "the FAT32 layout is malformed"),
            Self::BadCluster(cluster) => {
                write!(f, "cluster {cluster} is outside the data area")
            }
            Self::Unbounded => {
                core::fmt::Write::write_str(f, "the cluster chain did not terminate")
            }
            Self::Truncated => core::fmt::Write::write_str(
                f,
                "the cluster chain ended before the advertised file size",
            ),
            Self::NotFound => core::fmt::Write::write_str(f, "the directory entry was not found"),
            Self::WrongKind => {
                core::fmt::Write::write_str(f, "the directory entry has the wrong kind")
            }
        }
    }
}

impl core::error::Error for FatError {}

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

    /// The most clusters a directory chain may span before it must have ended.
    ///
    /// A directory holds at most [`MAX_DIRECTORY_ENTRIES`] entries, so it needs
    /// at most that many entry slots' worth of clusters. This does not grow
    /// with the volume, so a cyclic or oversized directory on a large volume
    /// cannot drive an unbounded read; the chain length is the tighter of this
    /// and the data area (D-352).
    fn directory_cluster_limit(self) -> u32 {
        let per_cluster = self.cluster_bytes() / ENTRY_BYTES as u32;
        let by_entries = if per_cluster == 0 {
            MAX_DIRECTORY_ENTRIES
        } else {
            MAX_DIRECTORY_ENTRIES.div_ceil(per_cluster)
        };
        by_entries.min(self.clusters())
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
    hare: Option<u32>,
    done: bool,
}

impl DirectoryCursor {
    /// Start reading the directory whose first cluster is `cluster`.
    pub const fn new(cluster: u32) -> Self {
        Self {
            cluster,
            sector: 0,
            visited: 0,
            hare: Some(cluster),
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
        let at = self.base.checked_add(sector).ok_or(FatError::Malformed)?;
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
                Some(current) => match self.next_cluster(current) {
                    Ok(Next::Cluster(next)) => Some(next),
                    Ok(Next::End | Next::Bad | Next::Free) | Err(FatError::BadCluster(_)) => None,
                    Err(error) => return Err(error),
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
                    if visited >= self.bpb.directory_cluster_limit() {
                        return Err(FatError::Unbounded);
                    }
                    // A tortoise/hare check catches a cycle in the length of
                    // the cycle, before the entry cap is reached (D-352).
                    let hare = self.advance_cycle_hare(cursor.hare)?;
                    if hare == Some(next) {
                        return Err(FatError::Unbounded);
                    }
                    cursor.cluster = next;
                    cursor.sector = 0;
                    cursor.visited = visited;
                    cursor.hare = hare;
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
mod tests;
