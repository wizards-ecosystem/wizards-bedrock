use super::*;
use crate::dmar::DmarEntries;
use crate::madt::MadtEntries;

mod dmar;
mod madt;
mod root;

fn fix_checksum(bytes: &mut [u8], index: usize) {
    bytes[index] = 0;
    let sum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    bytes[index] = sum.wrapping_neg();
}

fn table(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; Sdt::LEN];
    bytes[..4].copy_from_slice(signature);
    bytes.extend_from_slice(body);
    let length = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    fix_checksum(&mut bytes, 9);
    bytes
}

fn dmar_body(structures: &[u8]) -> Vec<u8> {
    let mut body = vec![0u8; 12];
    body.extend_from_slice(structures);
    body
}

fn drhd_bytes(include_pci_all: bool, segment: u16, register_base: u64, scopes: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 16];
    bytes[0..2].copy_from_slice(&0u16.to_le_bytes());
    bytes[4] = u8::from(include_pci_all);
    bytes[6..8].copy_from_slice(&segment.to_le_bytes());
    bytes[8..16].copy_from_slice(&register_base.to_le_bytes());
    bytes.extend_from_slice(scopes);
    let length = u16::try_from(bytes.len()).expect("fits");
    bytes[2..4].copy_from_slice(&length.to_le_bytes());
    bytes
}
