use core::fmt;

use crate::dmar::Named;
use crate::{
    AcpiError, Bdf, BusRange, DeviceScopeKind, DeviceScopes, DmarEntries, DmarStructure, Sdt,
};

/// How a remapping unit's scope reaches one function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Coverage {
    /// An endpoint scope names the function itself.
    Endpoint,
    /// A sub-hierarchy scope names this bridge, and the function is the bridge or behind it.
    Bridge(Bdf),
    /// No scope names the function, and the unit sets `INCLUDE_PCI_ALL`.
    CatchAll,
}

/// The remapping unit that governs one function, and the scope that chose it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GoverningUnit {
    pub register_base: u64,
    pub coverage: Coverage,
}

/// Every live PCI requester proved to belong to one remapping unit, in caller-bounded storage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RequesterInventory<const N: usize> {
    unit: GoverningUnit,
    requesters: [Option<Bdf>; N],
    count: usize,
}

impl<const N: usize> RequesterInventory<N> {
    /// The exact DRHD whose requester inventory was accepted.
    pub const fn unit(&self) -> GoverningUnit {
        self.unit
    }

    /// Number of distinct live requesters in the unit.
    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The distinct requester BDFs in declaration order.
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

    // A more-specific scope may validly assign one requester of a broad hierarchy to another unit.
    fn push_if_governed(
        &mut self,
        dmar: &Dmar<'_>,
        bridge: &impl Fn(Bdf) -> Option<BusRange>,
        hierarchy: &impl Fn(Bdf, Bdf) -> bool,
        requester: Bdf,
    ) -> Result<(), RequesterInventoryError> {
        match dmar.unit_for(requester, bridge, hierarchy) {
            Some(governing) if governing.register_base == self.unit.register_base => {
                self.push(requester)
            }
            Some(_) => Ok(()),
            None => Err(RequesterInventoryError::AmbiguousRequester),
        }
    }
}

/// Why one DRHD cannot become a complete fixed requester inventory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum RequesterInventoryError {
    /// No unambiguous unit governs the selected function.
    Ungoverned,
    /// The unit can govern functions not named by an endpoint scope.
    IncludesAllPci,
    /// A PCI scope could not be resolved through the supplied topology.
    UnresolvedScope,
    /// A declared endpoint resolves to another unit or ambiguously.
    ForeignEndpoint,
    /// A non-interrupt scope can name a requester the PCI inventory cannot fence.
    UnsupportedScope,
    /// A present requester within a hierarchy has ambiguous governance.
    AmbiguousRequester,
    /// The selected function was not among the unit's resolved requesters.
    MissingSelected,
    /// The unit has more distinct live requesters than the caller can own.
    TooManyRequesters { maximum: usize },
    /// One unit declares more PCI requester scopes than a boot may probe.
    TooManyRequesterScopes { maximum: usize },
}

impl fmt::Display for RequesterInventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ungoverned => core::fmt::Write::write_str(
                f,
                "no unambiguous remapping unit governs the selected function",
            ),
            Self::IncludesAllPci => core::fmt::Write::write_str(
                f,
                "the remapping unit includes unlisted PCI requesters",
            ),
            Self::UnresolvedScope => core::fmt::Write::write_str(
                f,
                "a remapping-unit PCI scope does not resolve through the topology",
            ),
            Self::ForeignEndpoint => core::fmt::Write::write_str(
                f,
                "a declared endpoint is not governed by the same remapping unit",
            ),
            Self::UnsupportedScope => core::fmt::Write::write_str(
                f,
                "the remapping unit contains an unsupported requester scope",
            ),
            Self::AmbiguousRequester => core::fmt::Write::write_str(
                f,
                "a present hierarchy requester has ambiguous remapping-unit governance",
            ),
            Self::MissingSelected => core::fmt::Write::write_str(
                f,
                "the selected function is absent from the remapping-unit requester inventory",
            ),
            Self::TooManyRequesters { maximum } => {
                write!(
                    f,
                    "the remapping unit has more than {maximum} live requesters"
                )
            }
            Self::TooManyRequesterScopes { maximum } => write!(
                f,
                "the remapping unit has more than {maximum} PCI requester scopes"
            ),
        }
    }
}

impl core::error::Error for RequesterInventoryError {}

/// Why a DMAR could not become a validated VT-d inventory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum DmarError {
    Table(AcpiError),
    ReservedRegionInverted {
        base: u64,
        limit: u64,
    },
    /// Two DRHDs advertise the same register base.
    DuplicateUnit {
        register_base: u64,
    },
    /// More units were supplied than the allocation-free semantic pass admits.
    TooManyUnits {
        maximum: usize,
    },
}

impl fmt::Display for DmarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(reason) => write!(f, "dmar is unusable: {reason}"),
            Self::ReservedRegionInverted { base, limit } => write!(
                f,
                "dmar reserved region [{base:#x}, {limit:#x}] has its limit before its base"
            ),
            Self::DuplicateUnit { register_base } => write!(
                f,
                "dmar has two drhd units at register base {register_base:#x}"
            ),
            Self::TooManyUnits { maximum } => {
                write!(f, "dmar has more than {maximum} drhd units")
            }
        }
    }
}

impl core::error::Error for DmarError {}

/// Bounds the allocation-free unit inventory and its duplicate scan on hostile firmware.
pub(crate) const MAX_DRHD_UNITS: usize = 64;

/// Keeps hostile duplicate scopes from amplifying a bounded PCI scan per hierarchy.
pub(crate) const MAX_REQUESTER_SCOPES: usize = 16;

/// A validated DMAR, so the queries below are pure arithmetic over trusted bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dmar<'table> {
    body: &'table [u8],
}

impl<'table> Dmar<'table> {
    /// Whether firmware reports platform interrupt-remapping support.
    pub const fn interrupt_remapping(self) -> bool {
        self.body[Sdt::LEN + 1] & 1 != 0
    }

    /// Whether firmware requires x2APIC to stay out of extended interrupt mode.
    pub const fn x2apic_opt_out(self) -> bool {
        self.body[Sdt::LEN + 1] & (1 << 1) != 0
    }

    /// Validate `table` as a DMAR with forward RMRRs and distinct DRHD register bases.
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

    /// Segment of the one and only DRHD, or `None` for zero or multiple units.
    pub fn sole_unit_segment(&self) -> Option<u16> {
        let mut units = self.drhds().map(|unit| unit.segment);
        let segment = units.next()?;
        units.next().is_none().then_some(segment)
    }

    /// Register base of the sole DRHD for an RMRR-free `segment`, catch-all or not.
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

    /// Register base of the sole catch-all DRHD for an RMRR-free `segment`.
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

    /// The unit governing `bdf`: the most specific scope wins and a tie between units is `None`.
    pub fn unit_for(
        &self,
        bdf: Bdf,
        bridge: impl Fn(Bdf) -> Option<BusRange>,
        hierarchy: impl Fn(Bdf, Bdf) -> bool,
    ) -> Option<GoverningUnit> {
        let mut claims = Claims::default();
        for unit in self.drhds() {
            if unit.segment != bdf.segment() {
                continue;
            }
            for scope in DeviceScopes::new(unit.scopes) {
                match scope.walk(unit.segment, &bridge) {
                    Some(named @ Named::Function(_)) if named.covers(bdf, &hierarchy) => {
                        claims.offer_endpoint(unit.register_base);
                    }
                    Some(named @ Named::Hierarchy(scope)) if named.covers(bdf, &hierarchy) => {
                        claims.offer_bridge(named, scope.root, unit.register_base, &hierarchy);
                    }
                    _ => {}
                }
            }
            if unit.include_pci_all {
                claims.offer_catch_all(unit.register_base);
            }
        }
        claims.governing()
    }

    /// Inventory every live requester of the selected function's DRHD, each rechecked.
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
        let mut found_unit = false;
        for unit in self.drhds() {
            if unit.register_base != inventory.unit.register_base {
                continue;
            }
            found_unit = true;
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
                        if self
                            .unit_for(endpoint, bridge, hierarchy)
                            .is_none_or(|governing| governing.register_base != unit.register_base)
                        {
                            return Err(RequesterInventoryError::ForeignEndpoint);
                        }
                        inventory.push(endpoint)?;
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
                    DeviceScopeKind::AcpiNamespaceDevice | DeviceScopeKind::Other { .. } => {
                        return Err(RequesterInventoryError::UnsupportedScope);
                    }
                }
            }
        }
        debug_assert!(found_unit, "a governing unit must name one DRHD");
        Ok(())
    }

    /// Every reserved region a device at `bdf` must have identity-mapped.
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

/// The unit each kind of scope offers for one function, and whether two units tie at that kind.
#[derive(Default, Debug)]
struct Claims {
    endpoint: Option<GoverningUnit>,
    endpoint_ambiguous: bool,
    enclosing: Option<(Named, GoverningUnit)>,
    enclosing_ambiguous: bool,
    catch_all: Option<GoverningUnit>,
    catch_all_ambiguous: bool,
}

impl Claims {
    fn offer_endpoint(&mut self, register_base: u64) {
        let candidate = GoverningUnit {
            register_base,
            coverage: Coverage::Endpoint,
        };
        if self
            .endpoint
            .is_some_and(|unit| unit.register_base != register_base)
        {
            self.endpoint_ambiguous = true;
        } else {
            self.endpoint.get_or_insert(candidate);
        }
    }

    // An inner bridge outranks the bridges above it; overlapping siblings on other units tie.
    fn offer_bridge(
        &mut self,
        named: Named,
        root: Bdf,
        register_base: u64,
        hierarchy: &impl Fn(Bdf, Bdf) -> bool,
    ) {
        let candidate = GoverningUnit {
            register_base,
            coverage: Coverage::Bridge(root),
        };
        match self.enclosing {
            Some((current, unit)) if unit.register_base == register_base => {
                if current.strictly_contains(named, hierarchy) {
                    self.enclosing = Some((named, candidate));
                }
            }
            Some((current, _)) if current.strictly_contains(named, hierarchy) => {
                self.enclosing = Some((named, candidate));
            }
            Some((current, _)) if named.strictly_contains(current, hierarchy) => {}
            Some(_) => {
                self.enclosing_ambiguous = true;
            }
            None => self.enclosing = Some((named, candidate)),
        }
    }

    fn offer_catch_all(&mut self, register_base: u64) {
        let candidate = GoverningUnit {
            register_base,
            coverage: Coverage::CatchAll,
        };
        if self
            .catch_all
            .is_some_and(|unit| unit.register_base != register_base)
        {
            self.catch_all_ambiguous = true;
        } else {
            self.catch_all.get_or_insert(candidate);
        }
    }

    fn governing(self) -> Option<GoverningUnit> {
        if self.endpoint.is_some() {
            return (!self.endpoint_ambiguous)
                .then_some(self.endpoint)
                .flatten();
        }
        if self.enclosing.is_some() {
            return (!self.enclosing_ambiguous)
                .then_some(self.enclosing.map(|(_, unit)| unit))
                .flatten();
        }
        (!self.catch_all_ambiguous)
            .then_some(self.catch_all)
            .flatten()
    }
}

/// Every function on `buses` of `segment`, in bus, device, function order.
fn functions_on(segment: u16, buses: impl Iterator<Item = u8>) -> impl Iterator<Item = Bdf> {
    buses.flat_map(move |bus| {
        (0..32).flat_map(move |device| {
            (0..8).map(move |function| Bdf::new(segment, bus, device, function))
        })
    })
}

/// The inclusive `(base, limit)` ranges [`Dmar::reserved_regions_for`] finds for one device.
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
