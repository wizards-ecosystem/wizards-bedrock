use super::*;

/// A page-rounded MMIO range admitted by this CPU and the immutable boot map.
#[derive(Debug)]
pub struct AdmittedDeviceRange {
    checked: CheckedDeviceRange,
    len: usize,
    physical_bits: u8,
}

/// A refused PCI BAR map with its unchanged function-bound authority.
#[derive(Debug)]
#[must_use = "a refused BAR map returns its mapping authority"]
pub struct BarMapFailure {
    /// Why no mapping was installed.
    pub reason: MapError,
    /// The unchanged BAR and physical-range authority.
    pub admitted: pci::AdmittedBar,
}

/// A refused PCI BAR unmap with its unchanged register and BAR authority.
#[derive(Debug)]
#[must_use = "a refused BAR unmap returns its cleanup authority"]
pub struct BarUnmapFailure {
    /// Why no mapping was removed.
    pub reason: MapError,
    /// The unchanged register access and removal authority.
    pub bar: pci::ReleasableBar,
}

/// Result of removing a function-bound BAR mapping.
#[derive(Debug)]
#[must_use = "a refused BAR unmap retains the only cleanup authority"]
pub enum BarUnmapOutcome {
    /// Every leaf and the device ledger entry were removed.
    Unmapped(pci_config::BarAuthority),
    /// Nothing changed and the exact mapping can be retried.
    Refused(BarUnmapFailure),
}

impl AdmittedDeviceRange {
    pub(crate) const fn physical(&self) -> Paddr {
        self.checked.physical()
    }

    pub(crate) const fn page_start(&self) -> Paddr {
        self.checked.page_start()
    }

    pub(crate) const fn page_end(&self) -> Paddr {
        self.checked.page_end()
    }

    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    pub(crate) const fn physical_bits(&self) -> u8 {
        self.physical_bits
    }
}

/// Admit every physical byte a device mapping would reach, refusing any page of RAM or firmware.
///
/// The boot memory map is initialized once, so this proof stays valid until a mapping consumes it.
pub fn admit_device_range(physical: Paddr, len: u64) -> Result<AdmittedDeviceRange, MapError> {
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
    /// Map an admitted device range uncacheable and never executable in the device window.
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
        let pages = admitted.page_end() - base;
        let Some(physical_extent) = AddressRange::new(base, pages) else {
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
        let virtual_base = first_fit_by(device_window, pages, PAGE_SIZE, &self.devices, |extent| {
            extent.map(|extent| extent.virtual_range)
        })
        .ok_or(MapError::OutOfMemory)?;
        let virtual_extent =
            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::NO_CACHE);
        self.map_range(
            virtual_base,
            base,
            pages,
            expected_flags.difference(Flags::PRESENT),
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
            pages,
            device_slot,
            expected_flags,
        })
    }

    fn owns_device_bytes(&self, physical: AddressRange) -> bool {
        self.devices.iter().flatten().any(|extent| {
            physical.base() < extent.physical.end() && extent.physical.base() < physical.end()
        })
    }

    /// Map one exact function-bound PCI BAR, returning its authority unchanged on refusal.
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

    /// Remove the exact mapping behind a transient device region, dropping its access first.
    ///
    /// The whole range is preflighted, so a refusal changes neither page tables nor ownership.
    pub fn unmap_device(&mut self, region: ReleasableDevice) -> Result<(), DeviceUnmapFailure> {
        if let Err(reason) = self.verify_device_unmap(&region) {
            return Err(DeviceUnmapFailure { reason, region });
        }
        let ReleasableDevice {
            mapping,
            registers: _,
        } = region;
        for page in 0..mapping.pages / PAGE_SIZE {
            let address = mapping.virtual_base + page * PAGE_SIZE;
            let Ok(slot) = self.leaf(address) else {
                panic!("a preflighted device leaf disappeared");
            };
            clear_mapping_slot(slot, address, Level::Pt);
        }
        let Some(slot) = self.devices.get_mut(mapping.device_slot) else {
            panic!("a preflighted device ledger slot disappeared");
        };
        *slot = None;
        Ok(())
    }

    /// Remove one PCI BAR mapping, returning its exact BAR authority either way.
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
        #[cfg(feature = "selftest")]
        if one_shot_mapping_refusal(&DEVICE_UNMAPS_BEFORE_REFUSAL) {
            return Err(MapError::OutOfMemory);
        }
        let mapping = &region.mapping;
        if mapping.roots != self.roots() {
            return Err(MapError::NotYours(mapping.address));
        }
        let offset = mapping.physical % PAGE_SIZE;
        let physical_base = mapping.physical - offset;
        let expected_extent = DeviceExtent {
            physical: AddressRange::new(physical_base, mapping.pages)
                .ok_or(MapError::InvalidDeviceRange(mapping.physical))?,
            virtual_range: AddressRange::new(mapping.virtual_base, mapping.pages)
                .ok_or(MapError::InvalidDeviceRange(mapping.physical))?,
        };
        if self.devices.get(mapping.device_slot).copied().flatten() != Some(expected_extent) {
            return Err(MapError::NotMapped(mapping.address));
        }

        for page in 0..mapping.pages / PAGE_SIZE {
            let address = mapping.virtual_base + page * PAGE_SIZE;
            let slot = self.leaf(address)?;
            let entry = EntrySlot::read(&slot);
            if entry.flags().contains(Flags::OWNED)
                || entry.address() != physical_base + page * PAGE_SIZE
            {
                return Err(MapError::Discontiguous(address));
            }
        }
        Ok(())
    }
}

/// First address of the device window, clear of the direct map below and the kernel image above.
pub(super) const DEVICE_WINDOW_START: u64 = 0xffff_ff00_0000_0000;

/// One page past the last address device registers may occupy.
pub(super) const DEVICE_WINDOW_END: u64 = 0xffff_ff01_0000_0000;

/// Device ranges one address space can own before activation.
const MAX_DEVICE_MAPPINGS: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct DeviceExtent {
    physical: AddressRange,
    virtual_range: AddressRange,
}

/// A linear claim on a device's registers, made while the tables were being built.
///
/// The root tells apart equal window addresses that different spaces minted.
#[derive(Debug)]
pub struct DeviceMapping {
    address: u64,
    pub(super) physical: Paddr,
    len: usize,
    pub(super) roots: AddressSpaceRoots,
    pub(super) virtual_base: u64,
    pub(super) pages: u64,
    device_slot: usize,
    pub(super) expected_flags: Flags,
}

impl DeviceMapping {
    /// Where the registers are readable, once the space is active.
    pub const fn address(&self) -> u64 {
        self.address
    }

    /// Take a region holding this token; [`AddressSpace::unmap_device`] consumes both to clear it.
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

    /// Whether the complete page-rounded extent is unchanged in the live root.
    pub(super) fn is_live(&self) -> bool {
        let Some(current) = current_mapping_root(self.roots) else {
            return false;
        };
        let mut offset = 0u64;
        while offset < self.pages {
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
            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {
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

/// A unique live MMIO region paired with the token that can remove it.
#[derive(Debug)]
pub struct ReleasableDevice {
    pub(super) mapping: DeviceMapping,
    pub(super) registers: MmioRegion,
}

/// A refused device unmap with its unchanged retry authority.
#[derive(Debug)]
#[must_use = "a refused device unmap returns its cleanup authority"]
pub struct DeviceUnmapFailure {
    /// Why no leaf or ownership record changed.
    pub reason: MapError,
    /// The unique register access and removal token.
    pub region: ReleasableDevice,
}

impl fmt::Display for DeviceUnmapFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.reason, f)
    }
}

impl core::error::Error for DeviceUnmapFailure {}

impl ReleasableDevice {
    pub(crate) const fn address(&self) -> u64 {
        self.mapping.address
    }

    /// Read one register word.
    pub fn read_u32(&self, offset: usize) -> Result<u32, crate::mmio::MmioError> {
        MmioRegion::read_u32(&self.registers, offset)
    }

    /// Write one register word.
    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), crate::mmio::MmioError> {
        MmioRegion::write_u32(&mut self.registers, offset, value)
    }

    /// Keep this mapping for good, consuming its removal token; rejected candidates return theirs.
    #[must_use]
    pub fn retain(self) -> MmioRegion {
        let Self {
            mapping: _,
            registers,
        } = self;
        registers
    }
}
