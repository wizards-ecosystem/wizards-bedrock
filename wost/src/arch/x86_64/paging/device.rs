use core::fmt;

use mmio_range::{AddressRange, first_fit_by};
use x86_paging::device::{DeviceRange as CheckedDeviceRange, DeviceRangeError};
use x86_paging::{Flags, Level, PAGE_SIZE, page_identity_matches};

use super::super::{cpu, pci};
use super::{
    AddressSpace, AddressSpaceRoots, Grain, MapError, clear_mapping_slot, current_mapping_root,
    leaf_in, walk,
};
use crate::mm;
use crate::mmio::MmioRegion;

#[derive(Debug)]
pub struct AdmittedDeviceRange {
    checked: CheckedDeviceRange,
    len: usize,
    physical_bits: u8,
}

#[derive(Debug)]
#[must_use = "a refused BAR map returns its mapping authority"]
pub struct BarMapFailure {
    pub reason: MapError,
    pub admitted: pci::AdmittedBar,
}

#[derive(Debug)]
#[must_use = "a refused BAR unmap returns its cleanup authority"]
pub struct BarUnmapFailure {
    pub reason: MapError,
    pub bar: pci::ReleasableBar,
}

#[derive(Debug)]
#[must_use = "a refused BAR unmap retains the only cleanup authority"]
pub enum BarUnmapOutcome {
    Unmapped(pci_config::BarAuthority),
    Refused(BarUnmapFailure),
}

impl AdmittedDeviceRange {
    pub(crate) const fn physical(&self) -> u64 {
        self.checked.physical()
    }

    pub(crate) const fn page_start(&self) -> u64 {
        self.checked.page_start()
    }

    pub(crate) const fn page_end(&self) -> u64 {
        self.checked.page_end()
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn physical_bits(&self) -> u8 {
        self.physical_bits
    }
}

// The boot map is set once, so this proof stays valid until a mapping consumes it.
pub fn admit_device_range(physical: u64, len: u64) -> Result<AdmittedDeviceRange, MapError> {
    let width = cpu::physical_address_width().ok_or(MapError::PhysicalWidthUnavailable)?;
    let physical_bits = width.bits();
    let checked = CheckedDeviceRange::new(physical, len, width).map_err(|reason| match reason {
        DeviceRangeError::Empty | DeviceRangeError::Overflow => {
            MapError::InvalidDeviceRange(physical)
        }
        DeviceRangeError::Unaligned(at) => MapError::UnalignedDevice(at),
        DeviceRangeError::OutsideWidth { bits, .. } => {
            MapError::OutsidePhysicalWidth { bits, physical }
        }
        _ => MapError::InvalidDeviceRange(physical),
    })?;
    if !mm::device_range_is_eligible(checked.page_start(), checked.page_end()) {
        return Err(MapError::NotDeviceMemory(physical));
    }
    let len = usize::try_from(len).map_err(|_| MapError::InvalidDeviceRange(physical))?;
    Ok(AdmittedDeviceRange {
        checked,
        len,
        physical_bits,
    })
}

impl AddressSpace {
    pub fn map_device(&mut self, admitted: AdmittedDeviceRange) -> Result<DeviceMapping, MapError> {
        self.map_device_range(&admitted)
    }

    fn map_device_range(
        &mut self,
        admitted: &AdmittedDeviceRange,
    ) -> Result<DeviceMapping, MapError> {
        let physical = admitted.physical();
        let base = admitted.page_start();
        self.may_reach(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)?;
        let mapped_len = admitted.page_end() - base;
        let Some(physical_extent) = AddressRange::new(base, mapped_len) else {
            panic!("an admitted device range always describes complete pages");
        };
        let offset = physical - base;
        if self.owns_device_bytes(physical_extent) {
            return Err(MapError::DeviceOverlap(physical));
        }
        let device_slot = match self.devices.iter().position(Option::is_none) {
            Some(slot) => slot,
            None if self.devices.len() >= MAX_DEVICE_MAPPINGS => {
                return Err(MapError::TooManyDevices);
            }
            None => {
                self.devices
                    .try_reserve(1)
                    .map_err(|_| MapError::OutOfMemory)?;
                self.devices.len()
            }
        };

        let Some(device_window) =
            AddressRange::new(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)
        else {
            return Err(MapError::OutOfMemory);
        };
        let virtual_base = first_fit_by(
            device_window,
            mapped_len,
            PAGE_SIZE,
            &self.devices,
            |extent| extent.map(|extent| extent.virtual_range),
        )
        .ok_or(MapError::OutOfMemory)?;
        let virtual_extent = AddressRange::new(virtual_base, mapped_len)
            .ok_or(MapError::InvalidDeviceRange(physical))?;

        self.map_range(
            virtual_base,
            base,
            mapped_len,
            DEVICE_FLAGS.difference(Flags::PRESENT),
            Grain::Pages,
        )?;
        let extent = DeviceExtent {
            physical: physical_extent,
            virtual_range: virtual_extent,
        };
        if device_slot == self.devices.len() {
            self.devices.push(Some(extent));
        } else {
            let Some(slot) = self.devices.get_mut(device_slot) else {
                panic!("device ledger lost its vacant slot");
            };
            *slot = Some(extent);
        }

        Ok(DeviceMapping {
            address: virtual_base + offset,
            physical,
            len: admitted.len,
            roots: self.roots(),
            virtual_base,
            mapped_len,
            device_slot,
        })
    }

    fn owns_device_bytes(&self, physical: AddressRange) -> bool {
        self.devices.iter().flatten().any(|extent| {
            physical.base() < extent.physical.end() && extent.physical.base() < physical.end()
        })
    }

    pub fn map_pci_bar(
        &mut self,
        admitted: pci::AdmittedBar,
    ) -> Result<pci::BarMapping, BarMapFailure> {
        let pci::AdmittedBar { authority, range } = admitted;
        match self.map_device_range(&range) {
            Ok(mapping) => Ok(pci::BarMapping { authority, mapping }),
            Err(reason) => Err(BarMapFailure {
                reason,
                admitted: pci::AdmittedBar { authority, range },
            }),
        }
    }

    pub fn unmap_device(&mut self, region: ReleasableDevice) -> Result<(), DeviceUnmapFailure> {
        if let Err(reason) = self.verify_device_unmap(&region) {
            return Err(DeviceUnmapFailure { reason, region });
        }
        let ReleasableDevice {
            mapping,
            registers: _,
        } = region;
        for page in 0..mapping.mapped_len / PAGE_SIZE {
            let address = mapping.virtual_base + page * PAGE_SIZE;
            let Ok(slot) = leaf_in(self.root(), address) else {
                panic!("a preflighted device leaf disappeared");
            };
            // Under the kernel's space lock this refuses rather than wait (see `Flush::settle`).
            clear_mapping_slot(slot, address, Level::Pt, self.roots()).settle();
        }
        let Some(slot) = self.devices.get_mut(mapping.device_slot) else {
            panic!("a preflighted device ledger slot disappeared");
        };
        *slot = None;
        Ok(())
    }

    pub fn unmap_pci_bar(&mut self, bar: pci::ReleasableBar) -> BarUnmapOutcome {
        let pci::ReleasableBar { authority, region } = bar;
        match self.unmap_device(region) {
            Ok(()) => BarUnmapOutcome::Unmapped(authority),
            Err(failure) => BarUnmapOutcome::Refused(BarUnmapFailure {
                reason: failure.reason,
                bar: pci::ReleasableBar {
                    authority,
                    region: failure.region,
                },
            }),
        }
    }

    fn verify_device_unmap(&self, region: &ReleasableDevice) -> Result<(), MapError> {
        let mapping = &region.mapping;
        if self.roots != Some(mapping.roots) {
            return Err(MapError::NotYours(mapping.address));
        }
        let offset = mapping.physical % PAGE_SIZE;
        let physical_base = mapping.physical - offset;
        let expected_extent = DeviceExtent {
            physical: AddressRange::new(physical_base, mapping.mapped_len)
                .ok_or(MapError::InvalidDeviceRange(mapping.physical))?,
            virtual_range: AddressRange::new(mapping.virtual_base, mapping.mapped_len)
                .ok_or(MapError::InvalidDeviceRange(mapping.physical))?,
        };
        if self.devices.get(mapping.device_slot).copied().flatten() != Some(expected_extent) {
            return Err(MapError::NotMapped(mapping.address));
        }

        for page in 0..mapping.mapped_len / PAGE_SIZE {
            let address = mapping.virtual_base + page * PAGE_SIZE;
            let slot = leaf_in(self.root(), address)?;
            let entry = slot.read();
            if entry.flags().contains(Flags::OWNED)
                || entry.address() != physical_base + page * PAGE_SIZE
            {
                return Err(MapError::Discontiguous(address));
            }
        }
        Ok(())
    }
}

pub(super) const DEVICE_WINDOW_START: u64 = 0xffff_ff00_0000_0000;

pub(super) const DEVICE_WINDOW_END: u64 = 0xffff_ff01_0000_0000;

const MAX_DEVICE_MAPPINGS: usize = 32;

const DEVICE_FLAGS: Flags = Flags::PRESENT
    .union(Flags::WRITABLE)
    .union(Flags::NO_EXECUTE)
    .union(Flags::NO_CACHE);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct DeviceExtent {
    physical: AddressRange,
    virtual_range: AddressRange,
}

#[derive(Debug)]
pub struct DeviceMapping {
    address: u64,
    pub(super) physical: u64,
    len: usize,
    pub(super) roots: AddressSpaceRoots,
    pub(super) virtual_base: u64,
    pub(super) mapped_len: u64,
    device_slot: usize,
}

impl DeviceMapping {
    pub const fn address(&self) -> u64 {
        self.address
    }

    pub fn claim_releasable(self) -> Result<ReleasableDevice, Self> {
        if !self.is_live() {
            return Err(self);
        }
        // SAFETY: `is_live` proved every page live, `map_device` four-byte alignment; all runnable
        // roots share its device table, reserved then frozen; the region keeps `self` until unmap.
        let registers = unsafe { MmioRegion::new(self.address as *mut u8, self.len) };
        Ok(ReleasableDevice {
            mapping: self,
            registers,
        })
    }

    pub(super) fn is_live(&self) -> bool {
        let Some(current) = current_mapping_root(self.roots) else {
            return false;
        };
        let mut offset = 0u64;
        while offset < self.mapped_len {
            let Some(virtual_address) = self.virtual_base.checked_add(offset) else {
                return false;
            };
            let physical_base = self.physical - self.physical % PAGE_SIZE;
            let Some(expected) = physical_base.checked_add(offset) else {
                return false;
            };
            let Some((mapped, flags, span)) = walk(current, virtual_address) else {
                return false;
            };
            if !page_identity_matches(mapped.get(), flags, span, expected, DEVICE_FLAGS) {
                return false;
            }
            let Some(next) = offset.checked_add(PAGE_SIZE) else {
                return false;
            };
            offset = next;
        }
        true
    }
}

#[derive(Debug)]
pub struct ReleasableDevice {
    pub(super) mapping: DeviceMapping,
    pub(super) registers: MmioRegion,
}

#[derive(Debug)]
#[must_use = "a refused device unmap returns its cleanup authority"]
pub struct DeviceUnmapFailure {
    pub reason: MapError,
    pub region: ReleasableDevice,
}

impl fmt::Display for DeviceUnmapFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, formatter)
    }
}

impl core::error::Error for DeviceUnmapFailure {}

impl ReleasableDevice {
    pub fn read_u32(&self, offset: usize) -> Result<u32, crate::mmio::MmioError> {
        self.registers.read_u32(offset)
    }

    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), crate::mmio::MmioError> {
        self.registers.write_u32(offset, value)
    }

    #[must_use]
    pub fn retain(self) -> MmioRegion {
        let Self {
            mapping: _,
            registers,
        } = self;
        registers
    }
}
