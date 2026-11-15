use core::fmt;

use crate::dmar::{DeviceScopeKind, DeviceScopes, DmarEntries, DmarStructure, Named};
use crate::{AcpiError, Bdf, BusRange, Sdt};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GoverningUnit {
    pub register_base: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RequesterInventory<const N: usize> {
    unit: GoverningUnit,
    requesters: [Option<Bdf>; N],
    count: usize,
}

impl<const N: usize> RequesterInventory<N> {
    pub const fn unit(&self) -> GoverningUnit {
        self.unit
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = Bdf> + '_ {
        self.requesters[..self.count].iter().flatten().copied()
    }

    fn push(&mut self, requester: Bdf) -> Result<(), RequesterInventoryError> {
        if self.requesters[..self.count].contains(&Some(requester)) {
            return Ok(());
        }
        let Some(slot) = self.requesters.get_mut(self.count) else {
            return Err(RequesterInventoryError::TooManyRequesters { maximum: N });
        };
        *slot = Some(requester);
        self.count += 1;
        Ok(())
    }

    fn push_if_governed(
        &mut self,
        dmar: &Dmar<'_>,
        bridge: &impl Fn(Bdf) -> Option<BusRange>,
        hierarchy: &impl Fn(Bdf, Bdf) -> bool,
        requester: Bdf,
    ) -> Result<(), RequesterInventoryError> {
        if dmar.unit_for(requester, bridge, hierarchy).is_none() {
            return Err(RequesterInventoryError::AmbiguousRequester);
        }
        self.push(requester)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RequesterInventoryError {
    Ungoverned,
    IncludesAllPci,
    UnresolvedScope,
    UnsupportedScope,
    AmbiguousRequester,
    MissingSelected,
    TooManyRequesters { maximum: usize },
    TooManyRequesterScopes { maximum: usize },
}

impl fmt::Display for RequesterInventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ungoverned => {
                formatter.write_str("no unambiguous remapping unit governs the selected function")
            }
            Self::IncludesAllPci => {
                formatter.write_str("the remapping unit includes unlisted PCI requesters")
            }
            Self::UnresolvedScope => formatter
                .write_str("a remapping-unit PCI scope does not resolve through the topology"),
            Self::UnsupportedScope => {
                formatter.write_str("the remapping unit contains an unsupported requester scope")
            }
            Self::AmbiguousRequester => formatter
                .write_str("a present hierarchy requester has ambiguous remapping-unit governance"),
            Self::MissingSelected => formatter.write_str(
                "the selected function is absent from the remapping-unit requester inventory",
            ),
            Self::TooManyRequesters { maximum } => {
                write!(
                    formatter,
                    "the remapping unit has more than {maximum} live requesters"
                )
            }
            Self::TooManyRequesterScopes { maximum } => write!(
                formatter,
                "the remapping unit has more than {maximum} PCI requester scopes"
            ),
        }
    }
}

impl core::error::Error for RequesterInventoryError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmarError {
    Table(AcpiError),
    ReservedRegionInverted { base: u64, limit: u64 },
    DuplicateUnit { register_base: u64 },
    TooManyUnits { maximum: usize },
}

impl fmt::Display for DmarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(reason) => write!(formatter, "dmar is unusable: {reason}"),
            Self::ReservedRegionInverted { base, limit } => write!(
                formatter,
                "dmar reserved region [{base:#x}, {limit:#x}] has its limit before its base"
            ),
            Self::DuplicateUnit { register_base } => write!(
                formatter,
                "dmar has two drhd units at register base {register_base:#x}"
            ),
            Self::TooManyUnits { maximum } => {
                write!(formatter, "dmar has more than {maximum} drhd units")
            }
        }
    }
}

impl core::error::Error for DmarError {}

// Bounds the allocation-free unit inventory and its duplicate scan on hostile firmware.
pub(crate) const MAX_DRHD_UNITS: usize = 64;

// Keeps hostile duplicate scopes from amplifying a bounded PCI scan per hierarchy.
pub(crate) const MAX_REQUESTER_SCOPES: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dmar<'table> {
    body: &'table [u8],
}

impl<'table> Dmar<'table> {
    pub const fn interrupt_remapping(self) -> bool {
        self.body[Sdt::LEN + 1] & 1 != 0
    }

    pub const fn x2apic_opt_out(self) -> bool {
        self.body[Sdt::LEN + 1] & (1 << 1) != 0
    }

    pub fn new(table: &'table [u8]) -> Result<Self, DmarError> {
        let entries = DmarEntries::new(table).map_err(DmarError::Table)?;
        let dmar = Self { body: entries.body };

        let units = dmar.drhds().take(MAX_DRHD_UNITS + 1).count();
        if units > MAX_DRHD_UNITS {
            return Err(DmarError::TooManyUnits {
                maximum: MAX_DRHD_UNITS,
            });
        }

        for (index, structure) in dmar.entries().enumerate() {
            match structure {
                DmarStructure::Drhd { register_base, .. } => {
                    let duplicate = dmar.entries().skip(index + 1).any(|later| {
                        matches!(
                            later,
                            DmarStructure::Drhd { register_base: other, .. }
                                if other == register_base
                        )
                    });
                    if duplicate {
                        return Err(DmarError::DuplicateUnit { register_base });
                    }
                }
                DmarStructure::Rmrr { base, limit, .. } if base > limit => {
                    return Err(DmarError::ReservedRegionInverted { base, limit });
                }
                _ => {}
            }
        }

        Ok(dmar)
    }

    // Skips `DmarEntries::new`'s validation, which `Dmar::new` already ran once.
    fn entries(&self) -> DmarEntries<'table> {
        DmarEntries {
            body: self.body,
            offset: DmarEntries::HEADER,
        }
    }

    fn drhds(&self) -> impl Iterator<Item = Drhd<'table>> {
        self.entries().filter_map(|structure| match structure {
            DmarStructure::Drhd {
                include_pci_all,
                segment,
                register_base,
                scopes,
            } => Some(Drhd {
                include_pci_all,
                segment,
                register_base,
                scopes,
            }),
            _ => None,
        })
    }

    pub fn unit_count(&self) -> usize {
        self.drhds().count()
    }

    // A single-segment consumer must refuse every other shape before claiming complete coverage.
    pub fn sole_unit_segment(&self) -> Option<u16> {
        let mut units = self.drhds().map(|unit| unit.segment);
        let segment = units.next()?;
        units.next().is_none().then_some(segment)
    }

    pub fn sole_rmrr_free_unit_register_base(&self, segment: u16) -> Option<u64> {
        if self.entries().any(
            |structure| matches!(structure, DmarStructure::Rmrr { segment: reserved, .. } if reserved == segment),
        ) {
            return None;
        }
        let mut units = self.entries().filter_map(|structure| match structure {
            DmarStructure::Drhd {
                include_pci_all,
                segment: unit_segment,
                register_base,
                ..
            } => Some((include_pci_all, unit_segment, register_base)),
            _ => None,
        });
        let (_, unit_segment, register_base) = units.next()?;
        (units.next().is_none() && unit_segment == segment).then_some(register_base)
    }

    // The static deny-all proof: one unit governs every possible requester, not just those seen.
    pub fn sole_complete_unit_register_base(&self, segment: u16) -> Option<u64> {
        let register_base = self.sole_rmrr_free_unit_register_base(segment)?;
        self.entries()
            .any(|structure| {
                matches!(
                    structure,
                    DmarStructure::Drhd {
                        include_pci_all: true,
                        register_base: candidate,
                        ..
                    } if candidate == register_base
                )
            })
            .then_some(register_base)
    }

    // Only a segment with exactly one unit is governed; several units are never arbitrated.
    pub fn unit_for(
        &self,
        bdf: Bdf,
        bridge: impl Fn(Bdf) -> Option<BusRange>,
        hierarchy: impl Fn(Bdf, Bdf) -> bool,
    ) -> Option<GoverningUnit> {
        let mut units = self.drhds().filter(|unit| unit.segment == bdf.segment());
        let unit = units.next()?;
        if units.next().is_some() {
            return None;
        }
        let covered = unit.include_pci_all
            || DeviceScopes::new(unit.scopes).any(|scope| {
                scope
                    .walk(unit.segment, &bridge)
                    .is_some_and(|named| named.covers(bdf, &hierarchy))
            });
        covered.then_some(GoverningUnit {
            register_base: unit.register_base,
        })
    }

    // `present` must report exactly whether each requester it is asked about exists now.
    pub fn requester_inventory<const N: usize>(
        &self,
        selected: Bdf,
        bridge: impl Fn(Bdf) -> Option<BusRange>,
        hierarchy: impl Fn(Bdf, Bdf) -> bool,
        present: impl Fn(Bdf) -> bool,
    ) -> Result<RequesterInventory<N>, RequesterInventoryError> {
        let unit = self
            .unit_for(selected, &bridge, &hierarchy)
            .ok_or(RequesterInventoryError::Ungoverned)?;
        let mut inventory = RequesterInventory {
            unit,
            requesters: [None; N],
            count: 0,
        };
        if self.sole_complete_unit_register_base(selected.segment()) == Some(unit.register_base) {
            for requester in functions_on(selected.segment(), 0..=u8::MAX) {
                if present(requester) {
                    inventory.push_if_governed(self, &bridge, &hierarchy, requester)?;
                }
            }
        } else {
            self.add_scoped_requesters(&mut inventory, &bridge, &hierarchy, &present)?;
        }
        if !inventory.requesters[..inventory.count].contains(&Some(selected)) {
            return Err(RequesterInventoryError::MissingSelected);
        }
        Ok(inventory)
    }

    fn add_scoped_requesters<const N: usize>(
        &self,
        inventory: &mut RequesterInventory<N>,
        bridge: &impl Fn(Bdf) -> Option<BusRange>,
        hierarchy: &impl Fn(Bdf, Bdf) -> bool,
        present: &impl Fn(Bdf) -> bool,
    ) -> Result<(), RequesterInventoryError> {
        let mut requester_scopes = 0usize;
        for unit in self.drhds() {
            if unit.register_base != inventory.unit.register_base {
                continue;
            }
            if unit.include_pci_all {
                return Err(RequesterInventoryError::IncludesAllPci);
            }
            for scope in DeviceScopes::new(unit.scopes) {
                if matches!(
                    scope.kind(),
                    DeviceScopeKind::PciEndpoint | DeviceScopeKind::PciSubHierarchy
                ) {
                    requester_scopes += 1;
                    if requester_scopes > MAX_REQUESTER_SCOPES {
                        return Err(RequesterInventoryError::TooManyRequesterScopes {
                            maximum: MAX_REQUESTER_SCOPES,
                        });
                    }
                }
                match scope.kind() {
                    DeviceScopeKind::PciEndpoint => {
                        let Some(Named::Function(endpoint)) = scope.walk(unit.segment, bridge)
                        else {
                            return Err(RequesterInventoryError::UnresolvedScope);
                        };
                        if present(endpoint) {
                            inventory.push(endpoint)?;
                        }
                    }
                    DeviceScopeKind::PciSubHierarchy => {
                        let Some(Named::Hierarchy(scope_hierarchy)) =
                            scope.walk(unit.segment, bridge)
                        else {
                            return Err(RequesterInventoryError::UnresolvedScope);
                        };
                        inventory.push_if_governed(
                            self,
                            bridge,
                            hierarchy,
                            scope_hierarchy.root,
                        )?;
                        let buses =
                            scope_hierarchy.buses.secondary..=scope_hierarchy.buses.subordinate;
                        for requester in functions_on(unit.segment, buses) {
                            if present(requester) {
                                inventory.push_if_governed(self, bridge, hierarchy, requester)?;
                            }
                        }
                    }
                    DeviceScopeKind::IoApic | DeviceScopeKind::Hpet => {}
                    DeviceScopeKind::AcpiNamespaceDevice | DeviceScopeKind::Other => {
                        return Err(RequesterInventoryError::UnsupportedScope);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn reserved_regions_for<F: Fn(Bdf) -> Option<BusRange>, H: Fn(Bdf, Bdf) -> bool>(
        &self,
        bdf: Bdf,
        bridge: F,
        hierarchy: H,
    ) -> ReservedRegions<'table, F, H> {
        ReservedRegions {
            entries: self.entries(),
            bdf,
            bridge,
            hierarchy,
        }
    }
}

#[derive(Debug)]
struct Drhd<'table> {
    include_pci_all: bool,
    segment: u16,
    register_base: u64,
    scopes: &'table [u8],
}

fn functions_on(segment: u16, buses: impl Iterator<Item = u8>) -> impl Iterator<Item = Bdf> {
    buses.flat_map(move |bus| {
        (0..32).flat_map(move |device| {
            (0..8).map(move |function| Bdf::new(segment, bus, device, function))
        })
    })
}

#[derive(Debug)]
pub struct ReservedRegions<'table, F, H> {
    entries: DmarEntries<'table>,
    bdf: Bdf,
    bridge: F,
    hierarchy: H,
}

impl<F: Fn(Bdf) -> Option<BusRange>, H: Fn(Bdf, Bdf) -> bool> Iterator
    for ReservedRegions<'_, F, H>
{
    type Item = (u64, u64);

    fn next(&mut self) -> Option<(u64, u64)> {
        for structure in self.entries.by_ref() {
            let DmarStructure::Rmrr {
                segment,
                base,
                limit,
                scopes,
            } = structure
            else {
                continue;
            };
            if segment != self.bdf.segment() {
                continue;
            }
            let named = DeviceScopes::new(scopes).any(|scope| {
                scope
                    .walk(segment, &self.bridge)
                    .is_some_and(|named| named.covers(self.bdf, &self.hierarchy))
            });
            if named {
                return Some((base, limit));
            }
        }
        None
    }
}
