use crate::{AcpiError, Sdt, u16_at, u32_at, u64_at};

/// A remapping structure of the DMA Remapping Reporting table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DmarStructure<'table> {
    /// A DMA remapping hardware unit: one IOMMU and the segment or devices it covers.
    Drhd {
        /// Whether this unit covers every device in `segment` that no other unit scopes explicitly.
        include_pci_all: bool,
        segment: u16,
        register_base: u64,
        /// Raw device-scope bytes; walk with [`DeviceScopes`].
        scopes: &'table [u8],
    },
    /// A reserved memory region a device DMAs into, which remapping must identity-map.
    Rmrr {
        segment: u16,
        base: u64,
        /// Last byte of the reserved region, inclusive.
        limit: u64,
        /// Raw device-scope bytes; walk with [`DeviceScopes`].
        scopes: &'table [u8],
    },
    /// A root-port ATS capability report.
    Atsr {
        segment: u16,
        /// Raw device-scope bytes; walk with [`DeviceScopes`].
        scopes: &'table [u8],
    },
    /// A remapping hardware static affinity, linking a unit to an SRAT proximity domain.
    Rhsa {
        /// Physical address of the DRHD this affinity describes.
        register_base: u64,
        proximity_domain: u32,
    },
    Other {
        kind: u16,
    },
}

/// The remapping structures of a validated DMAR.
#[derive(Debug)]
pub struct DmarEntries<'table> {
    pub(crate) body: &'table [u8],
    pub(crate) offset: usize,
}

impl<'table> DmarEntries<'table> {
    /// The SDT header, then the host address width, flags, and ten reserved bytes.
    pub(crate) const HEADER: usize = Sdt::LEN + 12;

    /// Validate a DMAR, every structure and every nested device scope before iteration.
    pub fn new(table: &'table [u8]) -> Result<Self, AcpiError> {
        let header = Sdt::parse(table)?;
        if header.signature() != Sdt::DMAR {
            return Err(AcpiError::WrongTable {
                expected: Sdt::DMAR,
                found: header.signature(),
            });
        }
        let length = header.length() as usize;
        if length < Self::HEADER {
            return Err(AcpiError::BadLength(header.length()));
        }
        let body = &table[..length];
        let mut offset = Self::HEADER;
        while offset < body.len() {
            let available = body.len() - offset;
            let Some(prefix) = body.get(offset..offset.saturating_add(4)) else {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: 0,
                    available,
                });
            };
            let kind = u16::from_le_bytes([prefix[0], prefix[1]]);
            let entry_len = usize::from(u16::from_le_bytes([prefix[2], prefix[3]]));
            // Each minimum, the unknown kind's 4 too, rejects a zero length that never advances.
            let (minimum, scope_start) = match kind {
                0 => (16, Some(16)), // DRHD
                1 => (24, Some(24)), // RMRR
                2 => (8, Some(8)),   // ATSR
                3 => (20, None),     // RHSA, no device scope
                _ => (4, None),      // unknown: only the header need be there
            };
            if entry_len < minimum || entry_len > available {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: entry_len,
                    available,
                });
            }
            if let Some(scope_start) = scope_start {
                let structure = &body[offset..offset + entry_len];
                let scope_base = offset.saturating_add(scope_start);
                validate_device_scopes(scope_base, &structure[scope_start..])?;
            }
            offset += entry_len;
        }
        Ok(Self {
            body,
            offset: Self::HEADER,
        })
    }
}

/// Bounds-check every device scope at absolute table offset `base`, so a refusal names the byte.
fn validate_device_scopes(base: usize, scopes: &[u8]) -> Result<(), AcpiError> {
    let mut offset = 0;
    while offset < scopes.len() {
        let available = scopes.len() - offset;
        let Some(prefix) = scopes.get(offset..offset.saturating_add(2)) else {
            return Err(AcpiError::MalformedEntry {
                offset: base.saturating_add(offset),
                length: 0,
                available,
            });
        };
        let entry_len = usize::from(prefix[1]);
        if entry_len < DeviceScope::MIN_LEN || entry_len > available {
            return Err(AcpiError::MalformedEntry {
                offset: base.saturating_add(offset),
                length: entry_len,
                available,
            });
        }
        let entry = &scopes[offset..offset + entry_len];
        if !device_scope_is_well_formed(entry) {
            return Err(AcpiError::MalformedEntry {
                offset: base.saturating_add(offset),
                length: entry_len,
                available,
            });
        }
        offset += entry_len;
    }
    Ok(())
}

/// Whether one bounded device scope has complete, encodable PCI path hops.
fn device_scope_is_well_formed(entry: &[u8]) -> bool {
    let Some((&kind, path)) = entry.first().zip(entry.get(DeviceScope::MIN_LEN..)) else {
        return false;
    };
    if !path.len().is_multiple_of(2)
        || matches!(
            DeviceScopeKind::from_raw(kind),
            DeviceScopeKind::PciEndpoint | DeviceScopeKind::PciSubHierarchy
        ) && path.is_empty()
    {
        return false;
    }
    path.chunks_exact(2).all(|hop| hop[0] < 32 && hop[1] < 8)
}

impl<'table> Iterator for DmarEntries<'table> {
    type Item = DmarStructure<'table>;

    fn next(&mut self) -> Option<DmarStructure<'table>> {
        let prefix = self.body.get(self.offset..self.offset + 4)?;
        let kind = u16::from_le_bytes([prefix[0], prefix[1]]);
        let length = usize::from(u16::from_le_bytes([prefix[2], prefix[3]]));
        let entry = self.body.get(self.offset..self.offset + length)?;
        self.offset += length;

        Some(match kind {
            0 => DmarStructure::Drhd {
                include_pci_all: entry[4] & 1 != 0,
                segment: u16_at(entry, 6).unwrap_or(0),
                register_base: u64_at(entry, 8).unwrap_or(0),
                scopes: &entry[16..],
            },
            1 => DmarStructure::Rmrr {
                segment: u16_at(entry, 6).unwrap_or(0),
                base: u64_at(entry, 8).unwrap_or(0),
                limit: u64_at(entry, 16).unwrap_or(0),
                scopes: &entry[24..],
            },
            2 => DmarStructure::Atsr {
                segment: u16_at(entry, 6).unwrap_or(0),
                scopes: &entry[8..],
            },
            3 => DmarStructure::Rhsa {
                register_base: u64_at(entry, 8).unwrap_or(0),
                proximity_domain: u32_at(entry, 16).unwrap_or(0),
            },
            kind => DmarStructure::Other { kind },
        })
    }
}

/// The kind of hardware a device-scope entry names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DeviceScopeKind {
    PciEndpoint,
    /// A PCI-to-PCI bridge, and everything behind it.
    PciSubHierarchy,
    /// An I/O APIC, named by its ACPI enumeration id.
    IoApic,
    /// An HPET, named by its ACPI enumeration id.
    Hpet,
    AcpiNamespaceDevice,
    Other {
        kind: u8,
    },
}

impl DeviceScopeKind {
    fn from_raw(kind: u8) -> Self {
        match kind {
            1 => Self::PciEndpoint,
            2 => Self::PciSubHierarchy,
            3 => Self::IoApic,
            4 => Self::Hpet,
            5 => Self::AcpiNamespaceDevice,
            kind => Self::Other { kind },
        }
    }
}

/// One device-scope entry nested in a DRHD, RMRR, or ATSR structure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceScope<'table> {
    kind: DeviceScopeKind,
    start_bus: u8,
    path: &'table [u8],
}

impl<'table> DeviceScope<'table> {
    /// The type, length, flags, reserved byte, enumeration id, and start bus.
    const MIN_LEN: usize = 6;

    pub const fn kind(self) -> DeviceScopeKind {
        self.kind
    }

    /// The first bus number in the hierarchical path to the device.
    pub const fn start_bus(self) -> u8 {
        self.start_bus
    }

    /// The (device, function) hops from `start_bus` down to the target.
    pub const fn path(self) -> PciPath<'table> {
        PciPath { bytes: self.path }
    }

    /// What this scope names in `segment`, or `None` for a non-PCI kind or an unvouched hop.
    pub(crate) fn walk(
        self,
        segment: u16,
        bridge: &impl Fn(Bdf) -> Option<BusRange>,
    ) -> Option<Named> {
        let hierarchy = match self.kind() {
            DeviceScopeKind::PciEndpoint => false,
            DeviceScopeKind::PciSubHierarchy => true,
            DeviceScopeKind::IoApic
            | DeviceScopeKind::Hpet
            | DeviceScopeKind::AcpiNamespaceDevice
            | DeviceScopeKind::Other { .. } => return None,
        };
        let mut hops = self.path();
        let (mut device, mut function) = hops.next()?;
        let mut bus = self.start_bus();
        for (next_device, next_function) in hops {
            let at = Bdf::new(segment, bus, device, function);
            let buses = behind(at, bridge)?;
            bus = buses.secondary;
            (device, function) = (next_device, next_function);
        }
        let at = Bdf::new(segment, bus, device, function);
        if hierarchy {
            Some(Named::Hierarchy(Hierarchy {
                root: at,
                buses: behind(at, bridge)?,
            }))
        } else {
            Some(Named::Function(at))
        }
    }
}

/// The (device, function) hops of a [`DeviceScope`]'s PCI path.
#[derive(Debug)]
pub struct PciPath<'table> {
    bytes: &'table [u8],
}

impl Iterator for PciPath<'_> {
    type Item = (u8, u8);

    fn next(&mut self) -> Option<(u8, u8)> {
        let pair = self.bytes.get(..2)?;
        self.bytes = &self.bytes[2..];
        Some((pair[0], pair[1]))
    }
}

/// The device-scope entries nested in a DRHD, RMRR, or ATSR [`DmarStructure`].
#[derive(Debug)]
pub struct DeviceScopes<'table> {
    bytes: &'table [u8],
}

impl<'table> DeviceScopes<'table> {
    pub const fn new(bytes: &'table [u8]) -> Self {
        Self { bytes }
    }
}

impl<'table> Iterator for DeviceScopes<'table> {
    type Item = DeviceScope<'table>;

    // Reachable without a validating constructor, so malformed bytes end the walk.
    fn next(&mut self) -> Option<DeviceScope<'table>> {
        let kind = *self.bytes.first()?;
        let length = usize::from(*self.bytes.get(1)?);
        if length < DeviceScope::MIN_LEN {
            self.bytes = &[];
            return None;
        }
        let Some(entry) = self.bytes.get(..length) else {
            self.bytes = &[];
            return None;
        };
        if !device_scope_is_well_formed(entry) {
            self.bytes = &[];
            return None;
        }
        self.bytes = &self.bytes[length..];

        Some(DeviceScope {
            kind: DeviceScopeKind::from_raw(kind),
            start_bus: entry[5],
            path: &entry[6..],
        })
    }
}

/// A PCI function, addressed the way a DMAR device scope names one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bdf {
    segment: u16,
    bus: u8,
    device: u8,
    function: u8,
}

impl Bdf {
    pub const fn new(segment: u16, bus: u8, device: u8, function: u8) -> Self {
        Self {
            segment,
            bus,
            device,
            function,
        }
    }

    pub const fn segment(self) -> u16 {
        self.segment
    }

    pub const fn bus(self) -> u8 {
        self.bus
    }

    pub const fn device(self) -> u8 {
        self.device
    }

    pub const fn function(self) -> u8 {
        self.function
    }
}

/// The buses a PCI-to-PCI bridge forwards to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BusRange {
    /// The bus directly behind the bridge.
    pub secondary: u8,
    /// The highest-numbered bus behind the bridge.
    pub subordinate: u8,
}

/// The buses behind the bridge at `at`, if its numbering leads forward past its own bus.
fn behind(at: Bdf, bridge: &impl Fn(Bdf) -> Option<BusRange>) -> Option<BusRange> {
    let buses = bridge(at)?;
    (at.bus() < buses.secondary && buses.secondary <= buses.subordinate).then_some(buses)
}

/// What a PCI device scope names once its path is walked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Named {
    Function(Bdf),
    /// A bridge, and every function on the buses behind it.
    Hierarchy(Hierarchy),
}

/// One resolved hierarchy and the exact live bridge path that reached it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Hierarchy {
    pub(crate) root: Bdf,
    pub(crate) buses: BusRange,
}

impl Named {
    /// Whether this names `bdf`, as itself or below a proved ancestor bridge.
    pub(crate) fn covers(self, bdf: Bdf, contains: &impl Fn(Bdf, Bdf) -> bool) -> bool {
        match self {
            Self::Function(at) => at == bdf,
            Self::Hierarchy(scope) => {
                scope.root == bdf
                    || (scope.buses.secondary..=scope.buses.subordinate).contains(&bdf.bus())
                        && contains(scope.root, bdf)
            }
        }
    }

    /// Whether this hierarchy strictly contains another through the live bridge topology.
    pub(crate) fn strictly_contains(
        self,
        inner: Self,
        contains: &impl Fn(Bdf, Bdf) -> bool,
    ) -> bool {
        let (Self::Hierarchy(outer), Self::Hierarchy(inner)) = (self, inner) else {
            return false;
        };
        outer.root != inner.root
            && contains(outer.root, inner.root)
            && outer.buses.secondary <= inner.buses.secondary
            && inner.buses.subordinate <= outer.buses.subordinate
    }
}
