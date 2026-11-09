use super::*;

mod attestation;
mod dmar;
mod madt;
mod requesters;
mod rsdp;

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
    let mut body = vec![0u8; 12]; // host address width, flags, 10 reserved
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
    let len = u16::try_from(bytes.len()).expect("fits");
    bytes[2..4].copy_from_slice(&len.to_le_bytes());
    bytes
}

fn rmrr_bytes(segment: u16, base: u64, limit: u64, scopes: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 24];
    bytes[0..2].copy_from_slice(&1u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&segment.to_le_bytes());
    bytes[8..16].copy_from_slice(&base.to_le_bytes());
    bytes[16..24].copy_from_slice(&limit.to_le_bytes());
    bytes.extend_from_slice(scopes);
    let len = u16::try_from(bytes.len()).expect("fits");
    bytes[2..4].copy_from_slice(&len.to_le_bytes());
    bytes
}

fn atsr_bytes(segment: u16, scopes: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 8];
    bytes[0..2].copy_from_slice(&2u16.to_le_bytes());
    bytes[6..8].copy_from_slice(&segment.to_le_bytes());
    bytes.extend_from_slice(scopes);
    let len = u16::try_from(bytes.len()).expect("fits");
    bytes[2..4].copy_from_slice(&len.to_le_bytes());
    bytes
}

fn rhsa_bytes(register_base: u64, proximity_domain: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 20];
    bytes[0..2].copy_from_slice(&3u16.to_le_bytes());
    bytes[2..4].copy_from_slice(&20u16.to_le_bytes());
    bytes[8..16].copy_from_slice(&register_base.to_le_bytes());
    bytes[16..20].copy_from_slice(&proximity_domain.to_le_bytes());
    bytes
}

fn pci_endpoint_scope(start_bus: u8, device: u8, function: u8) -> Vec<u8> {
    vec![1, 8, 0, 0, 0, start_bus, device, function]
}

// Device-scope type bytes from the VT-d specification's DMAR chapter.
const ENDPOINT: u8 = 1;
const SUB_HIERARCHY: u8 = 2;
const IO_APIC: u8 = 3;
const HPET: u8 = 4;
const NAMESPACE_DEVICE: u8 = 5;

fn scope(kind: u8, start_bus: u8, path: &[(u8, u8)]) -> Vec<u8> {
    let mut bytes = vec![kind, 0, 0, 0, 0, start_bus];
    for &(device, function) in path {
        bytes.extend_from_slice(&[device, function]);
    }
    bytes[1] = u8::try_from(bytes.len()).expect("fits");
    bytes
}
