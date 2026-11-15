use crate::{AcpiError, Sdt, u16_at, u64_at};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DmarStructure<'table> {
    Drhd {
        include_pci_all: bool,
        segment: u16,
        register_base: u64,
        scopes: &'table [u8],
    },
    Rmrr {
        segment: u16,
        base: u64,
        limit: u64,
        scopes: &'table [u8],
    },
    Other,
}

#[derive(Debug)]
pub(crate) struct DmarEntries<'table> {
    pub(crate) body: &'table [u8],
    pub(crate) offset: usize,
}

impl<'table> DmarEntries<'table> {
    pub(crate) const HEADER: usize = Sdt::LEN + 12;

    pub(crate) fn new(table: &'table [u8]) -> Result<Self, AcpiError> {
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
            let entry_length = usize::from(u16::from_le_bytes([prefix[2], prefix[3]]));
            // Each minimum, the unknown kind's 4 too, rejects a zero length that never advances.
            let (minimum, scope_start) = match kind {
                0 => (16, Some(16)),
                1 => (24, Some(24)),
                2 => (8, Some(8)),
                3 => (20, None),
                _ => (4, None),
            };
            if entry_length < minimum || entry_length > available {
                return Err(AcpiError::MalformedEntry {
                    offset,
                    length: entry_length,
                    available,
                });
            }
            if let Some(scope_start) = scope_start {
                let structure = &body[offset..offset + entry_length];
                let scope_base = offset.saturating_add(scope_start);
                validate_device_scopes(scope_base, &structure[scope_start..])?;
            }
            offset += entry_length;
        }
        Ok(Self {
            body,
            offset: Self::HEADER,
        })
    }
}

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
        let entry_length = usize::from(prefix[1]);
        if entry_length < DeviceScope::MIN_LEN || entry_length > available {
            return Err(AcpiError::MalformedEntry {
                offset: base.saturating_add(offset),
                length: entry_length,
                available,
            });
        }
        let entry = &scopes[offset..offset + entry_length];
        if !device_scope_is_well_formed(entry) {
            return Err(AcpiError::MalformedEntry {
                offset: base.saturating_add(offset),
                length: entry_length,
                available,
            });
        }
        offset += entry_length;
    }
    Ok(())
}

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
            _ => DmarStructure::Other,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DeviceScopeKind {
    PciEndpoint,
    PciSubHierarchy,
    IoApic,
    Hpet,
    AcpiNamespaceDevice,
    Other,
}

impl DeviceScopeKind {
    fn from_raw(kind: u8) -> Self {
        match kind {
            1 => Self::PciEndpoint,
            2 => Self::PciSubHierarchy,
            3 => Self::IoApic,
            4 => Self::Hpet,
            5 => Self::AcpiNamespaceDevice,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DeviceScope<'table> {
    kind: DeviceScopeKind,
    start_bus: u8,
    path: &'table [u8],
}

impl<'table> DeviceScope<'table> {
    const MIN_LEN: usize = 6;

    pub(crate) const fn kind(self) -> DeviceScopeKind {
        self.kind
    }

    const fn start_bus(self) -> u8 {
        self.start_bus
    }

    const fn path(self) -> PciPath<'table> {
        PciPath { bytes: self.path }
    }

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
            | DeviceScopeKind::Other => return None,
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

#[derive(Debug)]
pub(crate) struct PciPath<'table> {
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

#[derive(Debug)]
pub(crate) struct DeviceScopes<'table> {
    bytes: &'table [u8],
}

impl<'table> DeviceScopes<'table> {
    pub(crate) const fn new(bytes: &'table [u8]) -> Self {
        Self { bytes }
    }
}

impl<'table> Iterator for DeviceScopes<'table> {
    type Item = DeviceScope<'table>;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BusRange {
    pub secondary: u8,
    pub subordinate: u8,
}

fn behind(at: Bdf, bridge: &impl Fn(Bdf) -> Option<BusRange>) -> Option<BusRange> {
    let buses = bridge(at)?;
    (at.bus() < buses.secondary && buses.secondary <= buses.subordinate).then_some(buses)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Named {
    Function(Bdf),
    Hierarchy(Hierarchy),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Hierarchy {
    pub(crate) root: Bdf,
    pub(crate) buses: BusRange,
}

impl Named {
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
}
