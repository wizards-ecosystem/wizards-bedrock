//! The independently formatted FAT32 proof corpus.

use std::io::{Cursor, Read};

use anyhow::{Context, Result, anyhow, ensure};
use fatfs::{FileSystem, FsOptions};

use crate::sha256;

const ENVELOPE: &str = include_str!("../../../tools/fat32-corpus/foreign-fat32.sparse");
const MAGIC: &str = "wizard-fat32-sparse-v1";
const IMAGE_BYTES: usize = 64 * 1024 * 1024;
const SECTOR_BYTES: usize = 512;
const IMAGE_SHA256: &str = "06a6ad589c3652731fc60bc669e4e20bc4576da7761c18be0dc9c800fa4d1576";
const FRAGMENT_BYTES: usize = 20 * 1024;
const LONG_CONTENT: &[u8] = b"long-name fragments precede this alias\n";
const NESTED_CONTENT: &[u8] = b"nested through a foreign directory\n";

fn header<'a>(lines: &mut impl Iterator<Item = &'a str>, expected: &str) -> Result<&'a str> {
    let line = lines
        .next()
        .with_context(|| format!("missing {expected} header"))?;
    let mut fields = line.split(' ');
    ensure!(
        fields.next() == Some(expected),
        "expected {expected} header"
    );
    let value = fields
        .next()
        .with_context(|| format!("missing {expected} value"))?;
    ensure!(fields.next().is_none(), "extra field in {expected} header");
    Ok(value)
}

const fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn expand(source: &str) -> Result<Vec<u8>> {
    let mut lines = source.lines();
    ensure!(lines.next() == Some(MAGIC), "wrong sparse-envelope version");

    let bytes: usize = header(&mut lines, "bytes")?
        .parse()
        .context("parsing expanded byte length")?;
    let sector_bytes: usize = header(&mut lines, "sector-bytes")?
        .parse()
        .context("parsing sector byte length")?;
    let expected_digest = header(&mut lines, "sha256")?;

    ensure!(sector_bytes == SECTOR_BYTES, "unsupported sector size");
    ensure!(
        bytes != 0 && bytes <= IMAGE_BYTES,
        "expanded image is unbounded"
    );
    ensure!(bytes.is_multiple_of(sector_bytes), "partial final sector");
    ensure!(
        expected_digest.len() == 64
            && expected_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')),
        "malformed expanded digest"
    );

    let sectors = bytes / sector_bytes;
    let mut expanded = vec![0u8; bytes];
    let mut seen = vec![false; sectors];
    for (line_index, line) in lines.enumerate() {
        let line_number = line_index + 5;
        let mut fields = line.split(' ');
        ensure!(
            fields.next() == Some("sector"),
            "line {line_number} is not a sector record"
        );
        let index: usize = fields
            .next()
            .context("missing sector index")?
            .parse()
            .with_context(|| format!("parsing sector index on line {line_number}"))?;
        let encoded = fields.next().context("missing encoded sector")?;
        ensure!(
            fields.next().is_none(),
            "extra field on sector line {line_number}"
        );
        ensure!(index < sectors, "sector {index} is outside the image");
        ensure!(!seen[index], "sector {index} is recorded twice");
        ensure!(
            encoded.len() == sector_bytes * 2,
            "sector {index} is not complete"
        );

        let at = index * sector_bytes;
        for (pair, byte) in encoded
            .as_bytes()
            .chunks_exact(2)
            .zip(&mut expanded[at..at + sector_bytes])
        {
            let high = nibble(pair[0]).context("non-hexadecimal sector byte")?;
            let low = nibble(pair[1]).context("non-hexadecimal sector byte")?;
            *byte = high << 4 | low;
        }
        ensure!(
            expanded[at..at + sector_bytes]
                .iter()
                .any(|byte| *byte != 0),
            "sector {index} redundantly records zeros"
        );
        seen[index] = true;
    }

    let actual = sha256::hex(&sha256::digest(&expanded));
    ensure!(
        actual == expected_digest,
        "expanded digest is {actual}, expected {expected_digest}"
    );
    Ok(expanded)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct InventoryEntry {
    name: String,
    directory: bool,
    bytes: u32,
}

#[derive(Debug, PartialEq, Eq)]
struct View {
    inventory: Vec<InventoryEntry>,
    fragmented: Vec<u8>,
    long: Vec<u8>,
    nested: Vec<u8>,
}

fn oracle_view(image: &mut [u8]) -> Result<View> {
    let volume = Cursor::new(image);
    let filesystem = FileSystem::new(volume, FsOptions::new()).context("mounting with fatfs")?;
    let root = filesystem.root_dir();

    let mut inventory = Vec::new();
    for entry in root.iter() {
        let entry = entry.context("walking the foreign root with fatfs")?;
        inventory.push(InventoryEntry {
            name: entry.short_file_name(),
            directory: entry.is_dir(),
            bytes: u32::try_from(entry.len()).context("narrowing a FAT file size")?,
        });
    }
    inventory.sort();

    let mut fragmented = Vec::new();
    root.open_file("FRAG.BIN")
        .context("opening FRAG.BIN with fatfs")?
        .read_to_end(&mut fragmented)
        .context("reading FRAG.BIN with fatfs")?;

    let mut long = Vec::new();
    root.open_file("A Long Foreign Name.txt")
        .context("opening the long name with fatfs")?
        .read_to_end(&mut long)
        .context("reading the long name with fatfs")?;

    let mut nested = Vec::new();
    root.open_dir("ARCHIVE")
        .context("opening ARCHIVE with fatfs")?
        .open_file("INNER.TXT")
        .context("opening INNER.TXT with fatfs")?
        .read_to_end(&mut nested)
        .context("reading INNER.TXT with fatfs")?;

    Ok(View {
        inventory,
        fragmented,
        long,
        nested,
    })
}

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
        into.copy_from_slice(self.0.get(at..end).ok_or(())?);
        Ok(())
    }
}

fn reader_view(image: &[u8]) -> Result<View> {
    let mut volume = fat32::Volume::mount(Bytes(image), 0)
        .map_err(|error| anyhow!("mounting with libs/fat32: {error:?}"))?;
    let root = volume.bpb().root_cluster();
    let mut inventory = Vec::new();
    volume
        .walk_directory(root, |entry| {
            inventory.push(InventoryEntry {
                name: String::from_utf8_lossy(entry.name()).into_owned(),
                directory: entry.is_directory(),
                bytes: entry.size,
            });
            true
        })
        .map_err(|error| anyhow!("walking the foreign root with libs/fat32: {error:?}"))?;
    inventory.sort();

    let fragmented_entry = volume
        .lookup(root, b"FRAG.BIN")
        .map_err(|error| anyhow!("finding FRAG.BIN with libs/fat32: {error:?}"))?;
    let mut fragmented = vec![0u8; fragmented_entry.size as usize];
    let read = volume
        .read_file(&fragmented_entry, 0, &mut fragmented)
        .map_err(|error| anyhow!("reading FRAG.BIN with libs/fat32: {error:?}"))?;
    ensure!(read == fragmented.len(), "libs/fat32 shortened FRAG.BIN");

    let long_entry = volume
        .lookup(root, b"ALONGF~1.TXT")
        .map_err(|error| anyhow!("finding the long name's short alias: {error:?}"))?;
    let mut long = vec![0u8; long_entry.size as usize];
    let read = volume
        .read_file(&long_entry, 0, &mut long)
        .map_err(|error| anyhow!("reading the long name's short alias: {error:?}"))?;
    ensure!(
        read == long.len(),
        "libs/fat32 shortened the long-name file"
    );

    let nested_entry = volume
        .resolve(&[&b"ARCHIVE"[..], &b"INNER.TXT"[..]])
        .map_err(|error| anyhow!("resolving ARCHIVE/INNER.TXT with libs/fat32: {error:?}"))?;
    let mut nested = vec![0u8; nested_entry.size as usize];
    let read = volume
        .read_file(&nested_entry, 0, &mut nested)
        .map_err(|error| anyhow!("reading ARCHIVE/INNER.TXT with libs/fat32: {error:?}"))?;
    ensure!(read == nested.len(), "libs/fat32 shortened INNER.TXT");

    Ok(View {
        inventory,
        fragmented,
        long,
        nested,
    })
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[derive(Debug)]
struct RawLayout {
    root_chain: Vec<u32>,
    fragment_chain: Vec<u32>,
    deleted_before_live: bool,
    long_name_fragment: bool,
    volume_label: bool,
}

fn raw_chain(image: &[u8], fat_sector: usize, start: u32) -> Result<Vec<u32>> {
    let mut chain = Vec::new();
    let mut current = start;
    loop {
        ensure!(chain.len() < 512, "raw FAT chain did not terminate");
        chain.push(current);
        let at = fat_sector * SECTOR_BYTES + current as usize * 4;
        let next = le32(image, at) & 0x0fff_ffff;
        if next >= 0x0fff_fff8 {
            return Ok(chain);
        }
        ensure!(next >= 2, "raw FAT chain reached a free or reserved entry");
        current = next;
    }
}

fn inspect_layout(image: &[u8]) -> Result<RawLayout> {
    let sectors_per_cluster = usize::from(image[13]);
    let reserved = usize::from(le16(image, 14));
    let fats = usize::from(image[16]);
    let sectors_per_fat = le32(image, 36) as usize;
    let root = le32(image, 44);
    ensure!(
        sectors_per_cluster == 1 && reserved == 64 && fats == 2,
        "foreign geometry changed"
    );
    let data_sector = reserved + fats * sectors_per_fat;
    let root_chain = raw_chain(image, reserved, root)?;

    let mut deleted = false;
    let mut deleted_before_live = false;
    let mut long_name_fragment = false;
    let mut volume_label = false;
    let mut fragment_cluster = None;
    'directory: for cluster in &root_chain {
        let sector = data_sector + (*cluster as usize - 2) * sectors_per_cluster;
        for slot in image[sector * SECTOR_BYTES..(sector + sectors_per_cluster) * SECTOR_BYTES]
            .chunks_exact(32)
        {
            if slot[0] == 0 {
                break 'directory;
            }
            if slot[0] == 0xe5 {
                deleted = true;
                continue;
            }
            let attributes = slot[11];
            long_name_fragment |= attributes == 0x0f;
            volume_label |= attributes & 0x08 != 0 && attributes != 0x0f;
            if attributes != 0x0f && attributes & 0x08 == 0 {
                deleted_before_live |= deleted;
                if &slot[..11] == b"FRAG    BIN" {
                    fragment_cluster =
                        Some((u32::from(le16(slot, 20)) << 16) | u32::from(le16(slot, 26)));
                }
            }
        }
    }

    let fragment_chain = raw_chain(
        image,
        reserved,
        fragment_cluster.context("FRAG.BIN has no raw directory slot")?,
    )?;
    Ok(RawLayout {
        root_chain,
        fragment_chain,
        deleted_before_live,
        long_name_fragment,
        volume_label,
    })
}

fn fragment_byte(index: usize) -> u8 {
    (((index * 37) ^ (index >> 3) ^ 0x5a) & 0xff) as u8
}

#[test]
fn foreign_envelope_is_exact_and_hostile_records_are_refused() {
    let image = expand(ENVELOPE).expect("the pinned foreign image");
    assert_eq!(image.len(), IMAGE_BYTES);
    assert_eq!(sha256::hex(&sha256::digest(&image)), IMAGE_SHA256);

    let one = vec![1u8; SECTOR_BYTES];
    let digest = sha256::hex(&sha256::digest(&one));
    let encoded = "01".repeat(SECTOR_BYTES);
    let prefix =
        format!("{MAGIC}\nbytes {SECTOR_BYTES}\nsector-bytes {SECTOR_BYTES}\nsha256 {digest}\n");

    let duplicate = format!("{prefix}sector 0 {encoded}\nsector 0 {encoded}\n");
    let error = expand(&duplicate).expect_err("a duplicate sector must be refused");
    assert!(error.to_string().contains("recorded twice"));

    let partial = format!("{prefix}sector 0 01\n");
    let error = expand(&partial).expect_err("a partial sector must be refused");
    assert!(error.to_string().contains("not complete"));

    let outside = format!("{prefix}sector 1 {encoded}\n");
    let error = expand(&outside).expect_err("an out-of-range sector must be refused");
    assert!(error.to_string().contains("outside the image"));

    let zero = "00".repeat(SECTOR_BYTES);
    let error = expand(&format!("{prefix}sector 0 {zero}\n"))
        .expect_err("an explicit zero sector must be refused");
    assert!(error.to_string().contains("redundantly records zeros"));

    let wrong_digest = prefix.replace(&digest, &"0".repeat(64));
    let error = expand(&format!("{wrong_digest}sector 0 {encoded}\n"))
        .expect_err("the expanded digest must be checked");
    assert!(error.to_string().contains("expanded digest"));
}

#[test]
fn both_readers_agree_on_the_adversarial_foreign_layout() {
    let mut image = expand(ENVELOPE).expect("the pinned foreign image");
    let layout = inspect_layout(&image).expect("the recorded adversarial layout");
    assert!(
        layout.root_chain.len() >= 3,
        "root chain: {:?}",
        layout.root_chain
    );
    assert!(layout.deleted_before_live);
    assert!(layout.long_name_fragment);
    assert!(layout.volume_label);
    assert_eq!(layout.fragment_chain.len(), FRAGMENT_BYTES / SECTOR_BYTES);
    assert!(
        layout
            .fragment_chain
            .windows(2)
            .any(|pair| pair[1] != pair[0] + 1),
        "FRAG.BIN is contiguous: {:?}",
        layout.fragment_chain
    );

    let oracle = oracle_view(&mut image).expect("fatfs reads the foreign image");
    let reader = reader_view(&image).expect("libs/fat32 reads the foreign image");
    assert_eq!(reader.inventory, oracle.inventory);
    assert_eq!(reader.inventory.len(), 44);
    assert!(reader.inventory.iter().any(|entry| entry.name == "R39.TXT"));
    assert!(!reader.inventory.iter().any(|entry| entry.name == "R05.TXT"));
    assert!(
        reader
            .inventory
            .iter()
            .any(|entry| entry.name == "ALONGF~1.TXT")
    );

    assert_eq!(reader.fragmented, oracle.fragmented);
    assert_eq!(reader.fragmented.len(), FRAGMENT_BYTES);
    for (index, byte) in reader.fragmented.iter().enumerate() {
        assert_eq!(*byte, fragment_byte(index), "FRAG.BIN byte {index}");
    }
    assert_eq!(reader.long, oracle.long);
    assert_eq!(reader.long, LONG_CONTENT);
    assert_eq!(reader.nested, oracle.nested);
    assert_eq!(reader.nested, NESTED_CONTENT);
}
