#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt;

pub const SECTOR_BYTES: usize = 512;

const ENTRY_BYTES: usize = 32;

pub const FIRST_DATA_CLUSTER: u32 = 2;

const ENTRY_MASK: u32 = 0x0FFF_FFFF;

const END_OF_CHAIN: u32 = 0x0FFF_FFF8;

const BAD_CLUSTER: u32 = 0x0FFF_FFF7;

const MIN_FAT32_CLUSTERS: u64 = 65_525;
const FIRST_RESERVED_CLUSTER: u32 = 0x0FFF_FFF0;
const MAX_FAT32_CLUSTERS: u64 = FIRST_RESERVED_CLUSTER as u64 - FIRST_DATA_CLUSTER as u64;

const MAX_DIRECTORY_ENTRIES: u32 = 65_536;
const MIRRORING_DISABLED: u16 = 1 << 7;
const ACTIVE_FAT_MASK: u16 = 0x000F;
const EXTENDED_FLAGS_MASK: u16 = MIRRORING_DISABLED | ACTIVE_FAT_MASK;
const LOWERCASE_BASE: u8 = 1 << 3;
const LOWERCASE_EXTENSION: u8 = 1 << 4;
const CASE_FLAGS: u8 = LOWERCASE_BASE | LOWERCASE_EXTENSION;

mod attribute {
    // A long-name fragment carries this bit too, so one test skips both.
    pub(super) const VOLUME_LABEL: u8 = 0x08;
    pub(super) const DIRECTORY: u8 = 0x10;
    pub(super) const ARCHIVE: u8 = 0x20;
}

pub trait Sectors {
    type Error;

    fn read_sector(
        &mut self,
        sector: u64,
        into: &mut [u8; SECTOR_BYTES],
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FatError {
    Unreadable,
    NotABootSector,
    SectorSize(u16),
    ClusterSize(u8),
    Malformed,
    BadCluster(u32),
    Unbounded,
    Truncated,
    NotFound,
    WrongKind,
}

impl fmt::Display for FatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Unreadable => formatter.write_str("the sector source refused a read"),
            Self::NotABootSector => formatter.write_str("the boot sector signature is missing"),
            Self::SectorSize(bytes) => write!(formatter, "sector size {bytes} is unsupported"),
            Self::ClusterSize(sectors) => write!(formatter, "cluster size {sectors} is invalid"),
            Self::Malformed => formatter.write_str("the FAT32 layout is malformed"),
            Self::BadCluster(cluster) => {
                write!(formatter, "cluster {cluster} is outside the data area")
            }
            Self::Unbounded => formatter.write_str("the cluster chain did not terminate"),
            Self::Truncated => {
                formatter.write_str("the cluster chain ended before the advertised file size")
            }
            Self::NotFound => formatter.write_str("the directory entry was not found"),
            Self::WrongKind => formatter.write_str("the directory entry has the wrong kind"),
        }
    }
}

impl core::error::Error for FatError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
        let fat_entries = sectors_per_fat as u64 * SECTOR_BYTES as u64 / size_of::<u32>() as u64;
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

    const fn sectors_per_cluster(self) -> u32 {
        self.sectors_per_cluster
    }

    pub const fn cluster_bytes(self) -> u32 {
        self.sectors_per_cluster * SECTOR_BYTES as u32
    }

    pub const fn root_cluster(self) -> u32 {
        self.root_cluster
    }

    pub const fn first_data_sector(self) -> u64 {
        self.reserved_sectors as u64 + self.fats as u64 * self.sectors_per_fat as u64
    }

    pub const fn clusters(self) -> u32 {
        let data = self.total_sectors as u64 - self.first_data_sector();
        (data / self.sectors_per_cluster as u64) as u32
    }

    fn directory_cluster_limit(self) -> u32 {
        let per_cluster = self.cluster_bytes() / ENTRY_BYTES as u32;
        MAX_DIRECTORY_ENTRIES
            .div_ceil(per_cluster)
            .min(self.clusters())
    }

    const fn holds(self, cluster: u32) -> bool {
        cluster >= FIRST_DATA_CLUSTER && cluster < FIRST_DATA_CLUSTER + self.clusters()
    }

    const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {
        if !self.holds(cluster) {
            return Err(FatError::BadCluster(cluster));
        }
        let index = (cluster - FIRST_DATA_CLUSTER) as u64;
        Ok(self.first_data_sector() + index * self.sectors_per_cluster as u64)
    }

    const fn table_position(self, cluster: u32) -> (u64, usize) {
        let byte = cluster as u64 * size_of::<u32>() as u64;
        (
            self.reserved_sectors as u64
                + self.active_fat as u64 * self.sectors_per_fat as u64
                + byte / SECTOR_BYTES as u64,
            (byte % SECTOR_BYTES as u64) as usize,
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Next {
    Cluster(u32),
    End,
    Bad,
    Free,
}

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    name: [u8; 12],
    name_length: usize,
    pub attributes: u8,
    pub cluster: u32,
    pub size: u32,
}

impl Entry {
    const EMPTY: Self = Self {
        name: [0; 12],
        name_length: 0,
        attributes: 0,
        cluster: 0,
        size: 0,
    };

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_length]
    }

    pub const fn for_contents(cluster: u32, size: u32) -> Self {
        Self {
            name: [0u8; 12],
            name_length: 0,
            attributes: attribute::ARCHIVE,
            cluster,
            size,
        }
    }

    pub const fn is_directory(&self) -> bool {
        self.attributes & attribute::DIRECTORY != 0
    }

    fn is_named(&self, wanted: &[u8]) -> bool {
        if wanted.len() != self.name_length {
            return false;
        }
        wanted
            .iter()
            .zip(self.name())
            .all(|(wanted, observed)| upper(*wanted) == upper(*observed))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectoryCursor {
    cluster: u32,
    sector: u32,
    visited: u32,
    hare: Option<u32>,
    done: bool,
}

impl DirectoryCursor {
    pub const fn new(cluster: u32) -> Self {
        Self {
            cluster,
            sector: 0,
            visited: 0,
            hare: Some(cluster),
            done: false,
        }
    }

    pub const fn is_done(&self) -> bool {
        self.done
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectoryChunk {
    entries: [Entry; SECTOR_BYTES / ENTRY_BYTES],
    count: usize,
}

impl DirectoryChunk {
    const EMPTY: Self = Self {
        entries: [Entry::EMPTY; SECTOR_BYTES / ENTRY_BYTES],
        count: 0,
    };

    pub fn entries(&self) -> &[Entry] {
        &self.entries[..self.count]
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Entry(Entry),
    Skip,
    End,
}

const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {
    if slot[0] == 0x00 {
        return Slot::End;
    }
    if slot[0] == 0xE5 {
        return Slot::Skip;
    }

    let attributes = slot[11];
    if attributes & 0xC0 != 0 {
        return Slot::Skip;
    }
    let case = slot[12];
    if case & !CASE_FLAGS != 0 {
        return Slot::Skip;
    }
    if attributes & attribute::VOLUME_LABEL != 0 {
        return Slot::Skip;
    }

    let Some((name, name_length)) = short_name(slot, case) else {
        return Slot::Skip;
    };
    let Some(cluster) = first_cluster(slot) else {
        return Slot::Skip;
    };
    Slot::Entry(Entry {
        name,
        name_length,
        attributes,
        cluster,
        size: u32::from_le_bytes([slot[28], slot[29], slot[30], slot[31]]),
    })
}

const fn short_name(slot: &[u8; ENTRY_BYTES], case: u8) -> Option<([u8; 12], usize)> {
    let Some(base_len) = padded_len(slot, 0, 8) else {
        return None;
    };
    if base_len == 0 {
        return None;
    }
    let Some(extension_len) = padded_len(slot, 8, 3) else {
        return None;
    };

    let mut name = [0u8; 12];
    let mut length = 0;
    let mut index = 0;
    while index < base_len {
        let byte = if index == 0 && slot[index] == 0x05 {
            0xE5
        } else {
            slot[index]
        };
        if !valid_short_name_byte(byte) {
            return None;
        }
        name[length] = presented(byte, case & LOWERCASE_BASE);
        length += 1;
        index += 1;
    }
    if extension_len != 0 {
        name[length] = b'.';
        length += 1;
        index = 0;
        while index < extension_len {
            if !valid_short_name_byte(slot[8 + index]) {
                return None;
            }
            name[length] = presented(slot[8 + index], case & LOWERCASE_EXTENSION);
            length += 1;
            index += 1;
        }
    }
    Some((name, length))
}

const fn padded_len(slot: &[u8; ENTRY_BYTES], start: usize, width: usize) -> Option<usize> {
    let mut length = 0;
    while length < width && slot[start + length] != b' ' {
        length += 1;
    }
    let mut index = length;
    while index < width {
        if slot[start + index] != b' ' {
            return None;
        }
        index += 1;
    }
    Some(length)
}

const fn presented(byte: u8, lowercase: u8) -> u8 {
    let canonical = upper(byte);
    if lowercase != 0 {
        lower(canonical)
    } else {
        canonical
    }
}

const fn first_cluster(slot: &[u8; ENTRY_BYTES]) -> Option<u32> {
    let high = u16::from_le_bytes([slot[20], slot[21]]) as u32;
    if high & 0xF000 != 0 {
        return None;
    }
    let low = u16::from_le_bytes([slot[26], slot[27]]) as u32;
    Some((high << 16) | low)
}

#[derive(Debug)]
pub struct Volume<S: Sectors> {
    source: S,
    base: u64,
    bpb: Bpb,
}

impl<S: Sectors> Volume<S> {
    // `sectors` is the extent the device holds from `base`; the volume must fit inside it.
    pub fn mount(mut source: S, base: u64, sectors: u64) -> Result<Self, FatError> {
        if sectors == 0 {
            return Err(FatError::Malformed);
        }
        let mut sector = [0u8; SECTOR_BYTES];
        source
            .read_sector(base, &mut sector)
            .map_err(|_| FatError::Unreadable)?;
        let bpb = Bpb::parse(&sector)?;
        if u64::from(bpb.total_sectors) > sectors {
            return Err(FatError::Malformed);
        }
        Ok(Self { source, base, bpb })
    }

    pub const fn bpb(&self) -> Bpb {
        self.bpb
    }

    fn sector(&mut self, sector: u64, into: &mut [u8; SECTOR_BYTES]) -> Result<(), FatError> {
        if sector >= u64::from(self.bpb.total_sectors) {
            return Err(FatError::Malformed);
        }
        let at = self.base.checked_add(sector).ok_or(FatError::Malformed)?;
        self.source
            .read_sector(at, into)
            .map_err(|_| FatError::Unreadable)
    }

    const fn chain_limit(&self) -> u32 {
        self.bpb.clusters()
    }

    fn next_cluster(&mut self, cluster: u32) -> Result<Next, FatError> {
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
                    Err(reason) => return Err(reason),
                },
                None => None,
            };
        }
        Ok(cluster)
    }

    // The entries own their names, so a caller may release the device lock before reading them.
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
                Slot::Skip => {}
                Slot::Entry(entry) => {
                    chunk.entries[chunk.count] = entry;
                    chunk.count += 1;
                }
            }
        }
        Ok(chunk)
    }

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

    // Never past the size: the cluster tail holds another file's data.
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
}

#[cfg(test)]
mod tests;
