use core::sync::atomic::Ordering;

use super::{DIRECT_MAP_OFFSET, MAP, PhysicalAddress, direct_pointer};
use crate::boot::{DirectMap, MemoryMap, MemoryRegion};

// Reads only bytes the given map describes exclusively as ACPI table storage.
#[derive(Clone, Copy, Debug)]
pub struct EarlyFirmwareMemory<'map> {
    map: &'map MemoryMap,
    direct: DirectMap,
}

impl<'map> EarlyFirmwareMemory<'map> {
    pub const fn new(map: &'map MemoryMap, direct: DirectMap) -> Self {
        Self { map, direct }
    }

    #[must_use = "a refused early firmware read copied no bytes"]
    pub fn read(&self, address: u64, into: &mut [u8]) -> bool {
        if !firmware_contains(self.map, address, into.len()) {
            return false;
        }
        let Ok(len) = u64::try_from(into.len()) else {
            return false;
        };
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        if self.direct.virtual_address(end).is_none() {
            return false;
        }
        let Some(base) = self.direct.virtual_address(address) else {
            return false;
        };
        read_firmware_bytes(base as *const u8, into);
        true
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FirmwareMemory(());

impl FirmwareMemory {
    #[must_use = "a refused firmware read copied no bytes"]
    pub fn read(&self, address: u64, into: &mut [u8]) -> bool {
        let contained = MAP
            .lock_leaf()
            .as_ref()
            .is_some_and(|map| firmware_contains(map, address, into.len()));
        if !contained {
            return false;
        }
        let Some(base) = PhysicalAddress::new(address)
            .and_then(|address| direct_pointer(address, into.len() as u64))
        else {
            return false;
        };
        read_firmware_bytes(base, into);
        true
    }
}

fn firmware_contains(map: &MemoryMap, address: u64, len: usize) -> bool {
    let Ok(len) = u64::try_from(len) else {
        return false;
    };
    if len == 0 {
        return false;
    }
    let Some(end) = address.checked_add(len) else {
        return false;
    };
    let overlaps = |region: &MemoryRegion| address < region.base + region.len && region.base < end;
    map.covers(address, end)
        && map
            .regions()
            .filter(overlaps)
            .all(|region| region.kind.is_firmware_table())
}

fn read_firmware_bytes(base: *const u8, into: &mut [u8]) {
    for (index, slot) in into.iter_mut().enumerate() {
        let byte = base.wrapping_add(index);
        // SAFETY: the callers proved every byte immutable ACPI storage at a non-overflowing
        // direct-map address, which the bootloader's root maps and activation verified ours does.
        *slot = unsafe { byte.read_volatile() };
    }
}

pub fn firmware_memory() -> Option<FirmwareMemory> {
    (MAP.lock_leaf().is_some() && DIRECT_MAP_OFFSET.load(Ordering::Acquire) != 0)
        .then_some(FirmwareMemory(()))
}
