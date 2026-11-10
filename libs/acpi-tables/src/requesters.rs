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
    /// A sub-hierarchy scope names this bridge, and the function is the
    /// bridge or behind it.
    Bridge(Bdf),
    /// No scope names the function, and the unit sets `INCLUDE_PCI_ALL`.
    CatchAll,
}

/// The remapping unit that governs one function, and the scope that chose it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GoverningUnit {
    /// Physical address of the unit's register set.
    pub register_base: u64,
    /// How the unit's scope reaches the function.
    pub coverage: Coverage,
}

/// Every live PCI requester proved to belong to one remapping unit.
///
/// The fixed storage is deliberate: callers must choose and enforce their
/// ownership bound before a unit-wide hardware transition can begin.
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

    /// Whether the inventory contains no requester.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The distinct requester BDFs in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = Bdf> + '_ {
        self.requesters[..self.count].iter().flatten().copied()
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
    TooManyRequesters {
        /// Fixed requester capacity supplied by the caller.
        maximum: usize,
    },
    /// One unit declares more PCI requester scopes than a boot may probe.
    TooManyRequesterScopes {
        /// Fixed per-unit scope budget.
        maximum: usize,
    },
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
    /// The DMAR itself is not structurally valid.
    Table(AcpiError),
    /// An RMRR's limit precedes its base.
    ReservedRegionInverted {
        /// First byte of the declared region.
        base: u64,
        /// Last byte of the declared region.
        limit: u64,
    },
    /// Two DRHDs advertise the same register base.
    DuplicateUnit {
        /// The register base both units claim.
        register_base: u64,
    },
    /// More units were supplied than the allocation-free semantic pass admits.
    TooManyUnits {
        /// Maximum number of DRHD units accepted in one table.
        maximum: usize,
    },
}

impl fmt::Display for DmarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Table(error) => write!(f, "dmar is unusable: {error}"),
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

/// A validated DMAR: every remapping structure and nested device scope
/// checked once, so the queries below are pure arithmetic over trusted
/// bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Dmar<'a> {
    body: &'a [u8],
}

fn push_requester<const N: usize>(
    requesters: &mut [Option<Bdf>; N],
    count: &mut usize,
    requester: Bdf,
) -> Result<(), RequesterInventoryError> {
    if requesters[..*count].contains(&Some(requester)) {
        return Ok(());
    }
    let Some(slot) = requesters.get_mut(*count) else {
        return Err(RequesterInventoryError::TooManyRequesters { maximum: N });
    };
    *slot = Some(requester);
    *count += 1;
    Ok(())
}

fn add_live_requester<const N: usize>(
    dmar: &Dmar<'_>,
    bridge: &impl Fn(Bdf) -> Option<BusRange>,
    hierarchy: &impl Fn(Bdf, Bdf) -> bool,
    register_base: u64,
    requester: Bdf,
    requesters: &mut [Option<Bdf>; N],
    count: &mut usize,
) -> Result<(), RequesterInventoryError> {
    match dmar.unit_for(requester, bridge, hierarchy) {
        Some(governing) if governing.register_base == register_base => {
            push_requester(requesters, count, requester)
        }
        // A more-specific scope may validly remove one requester from a broad
        // hierarchy and assign it to another unit.
        Some(_) => Ok(()),
        None => Err(RequesterInventoryError::AmbiguousRequester),
    }
}

/// A platform needs only a handful of units. Bounding this allocation-free
/// inventory also bounds the duplicate scan below on hostile firmware.
pub(crate) const MAX_DRHD_UNITS: usize = 64;

/// A requester inventory may perform a full bounded PCI scan for each
/// hierarchy. Keep hostile duplicate scopes from amplifying that proof into an
/// impractical number of configuration cycles.
pub(crate) const MAX_REQUESTER_SCOPES: usize = 16;

impl<'a> Dmar<'a> {
    /// Whether firmware reports platform interrupt-remapping support.
    pub const fn interrupt_remapping(self) -> bool {
        self.body[Sdt::LEN + 1] & 1 != 0
    }

    /// Whether firmware requires x2APIC to stay out of extended interrupt mode.
    pub const fn x2apic_opt_out(self) -> bool {
        self.body[Sdt::LEN + 1] & (1 << 1) != 0
    }

    /// Validate `table` as a DMAR, bound its unit inventory, and additionally
    /// enforce the two rules bytes alone cannot express: an RMRR's limit must
    /// not precede its base, and no two DRHDs may claim the same register base.
    ///
    /// # Errors
    ///
    /// [`DmarError::Table`] if the table is not a structurally valid DMAR,
    /// [`DmarError::ReservedRegionInverted`] for a backwards RMRR range, or
    /// [`DmarError::DuplicateUnit`] if two DRHDs share a register base, or
    /// [`DmarError::TooManyUnits`] if the semantic pass would exceed its bound.
    pub fn new(table: &'a [u8]) -> Result<Self, DmarError> {
        let entries = DmarEntries::new(table).map_err(DmarError::Table)?;
        let dmar = Self { body: entries.body };

        let units = dmar
            .entries()
            .filter(|structure| matches!(structure, DmarStructure::Drhd { .. }))
            .take(MAX_DRHD_UNITS + 1)
            .count();
        if units > MAX_DRHD_UNITS {
            return Err(DmarError::TooManyUnits {
                maximum: MAX_DRHD_UNITS,
            });
        }

        // O(n^2), not a growable set; the explicit unit ceiling above keeps
        // this allocation-free duplicate pass finite on hostile firmware.
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

    // Skips `DmarEntries::new`'s validation: `Dmar::new` already ran it
    // once, and repeating it on every query would defeat the point of
    // validating once.
    fn entries(&self) -> DmarEntries<'a> {
        DmarEntries {
            body: self.body,
            offset: DmarEntries::HEADER,
        }
    }

    /// How many remapping units the table describes.
    pub fn unit_count(&self) -> usize {
        self.entries()
            .filter(|structure| matches!(structure, DmarStructure::Drhd { .. }))
            .count()
    }

    /// Segment of the one and only DRHD, or `None` for zero or multiple units.
    ///
    /// A consumer that can enumerate only one PCI segment must reject every
    /// other shape before claiming complete requester coverage.
    pub fn sole_unit_segment(&self) -> Option<u16> {
        let mut units = self.entries().filter_map(|structure| match structure {
            DmarStructure::Drhd { segment, .. } => Some(segment),
            _ => None,
        });
        let segment = units.next()?;
        units.next().is_none().then_some(segment)
    }

    /// Register base of the sole DRHD for an RMRR-free `segment`.
    ///
    /// This proves that a coverage attestation names the same one unit as the
    /// standards-defined DMAR; it does not itself prove catch-all coverage.
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
    ///
    /// This is the static proof needed to establish a deny-all owner before
    /// requester discovery: one unit must govern every possible requester in
    /// the segment, not just every function a mutable presence scan observed.
    /// Any RMRR for the segment refuses early ownership before a unit write.
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

    /// The DRHD unit that governs `bdf`, and the scope that decided it.
    ///
    /// Scopes resolve through one sealed topology: `bridge` reports the buses
    /// behind a function and `hierarchy` proves that its first function is an
    /// ancestor of the second. The most specific scope wins: an endpoint
    /// scope naming `bdf` outranks a bridge scope enclosing it, an inner
    /// bridge outranks the bridges above it, and an `INCLUDE_PCI_ALL` unit
    /// governs only what no scope in its segment names. Equal-specificity
    /// claims from distinct units are ambiguous and fail closed as `None`.
    pub fn unit_for(
        &self,
        bdf: Bdf,
        bridge: impl Fn(Bdf) -> Option<BusRange>,
        hierarchy: impl Fn(Bdf, Bdf) -> bool,
    ) -> Option<GoverningUnit> {
        let mut endpoint: Option<GoverningUnit> = None;
        let mut endpoint_ambiguous = false;
        let mut enclosing: Option<(Named, GoverningUnit)> = None;
        let mut enclosing_ambiguous = false;
        let mut catch_all: Option<GoverningUnit> = None;
        let mut catch_all_ambiguous = false;
        for structure in self.entries() {
            let DmarStructure::Drhd {
                include_pci_all,
                segment,
                register_base,
                scopes,
            } = structure
            else {
                continue;
            };
            if segment != bdf.segment() {
                continue;
            }
            for scope in DeviceScopes::new(scopes) {
                match scope.walk(segment, &bridge) {
                    Some(named @ Named::Function(_)) if named.covers(bdf, &hierarchy) => {
                        let candidate = GoverningUnit {
                            register_base,
                            coverage: Coverage::Endpoint,
                        };
                        if endpoint.is_some_and(|unit| unit.register_base != register_base) {
                            endpoint_ambiguous = true;
                        } else {
                            endpoint.get_or_insert(candidate);
                        }
                    }
                    Some(named @ Named::Hierarchy(scope)) if named.covers(bdf, &hierarchy) => {
                        let candidate = GoverningUnit {
                            register_base,
                            coverage: Coverage::Bridge(scope.root),
                        };
                        match enclosing {
                            Some((current, unit)) if unit.register_base == register_base => {
                                if current.strictly_contains(named, &hierarchy) {
                                    enclosing = Some((named, candidate));
                                }
                            }
                            Some((current, _)) if current.strictly_contains(named, &hierarchy) => {
                                enclosing = Some((named, candidate));
                            }
                            Some((current, _)) if named.strictly_contains(current, &hierarchy) => {}
                            Some(_) => {
                                enclosing_ambiguous = true;
                            }
                            None => enclosing = Some((named, candidate)),
                        }
                    }
                    _ => {}
                }
            }
            if include_pci_all {
                let candidate = GoverningUnit {
                    register_base,
                    coverage: Coverage::CatchAll,
                };
                if catch_all.is_some_and(|unit| unit.register_base != register_base) {
                    catch_all_ambiguous = true;
                } else {
                    catch_all.get_or_insert(candidate);
                }
            }
        }
        if endpoint.is_some() {
            return (!endpoint_ambiguous).then_some(endpoint).flatten();
        }
        if enclosing.is_some() {
            return (!enclosing_ambiguous)
                .then_some(enclosing.map(|(_, unit)| unit))
                .flatten();
        }
        (!catch_all_ambiguous).then_some(catch_all).flatten()
    }

    /// Inventory every live PCI requester governed by the selected function's
    /// exact DRHD.
    ///
    /// Endpoint paths are resolved exactly. Sub-hierarchy scopes are expanded
    /// across every bus, device and function number in their bounded range;
    /// `present` must report whether that exact requester currently exists.
    /// Each result is checked through [`Self::unit_for`] again so a scope
    /// shadowed by a provably more-specific unit cannot enter the inventory.
    /// Overlapping sibling hierarchies are ambiguous, and a fixed scope budget
    /// bounds hostile duplicate scans. A sole RMRR-free catch-all unit instead
    /// scans the complete, fixed segment requester-ID space through `present`,
    /// even when a more-specific scope names `selected`; the caller's fixed
    /// inventory capacity remains the admission bound.
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
        if self.sole_complete_unit_register_base(selected.segment()) == Some(unit.register_base) {
            let mut requesters = [None; N];
            let mut count = 0usize;
            for bus in 0..=u8::MAX {
                for device in 0..32 {
                    for function in 0..8 {
                        let requester = Bdf::new(selected.segment(), bus, device, function);
                        if !present(requester) {
                            continue;
                        }
                        match self.unit_for(requester, &bridge, &hierarchy) {
                            Some(governing) if governing.register_base == unit.register_base => {
                                push_requester(&mut requesters, &mut count, requester)?;
                            }
                            Some(_) => {}
                            None => return Err(RequesterInventoryError::AmbiguousRequester),
                        }
                    }
                }
            }
            if !requesters[..count].contains(&Some(selected)) {
                return Err(RequesterInventoryError::MissingSelected);
            }
            return Ok(RequesterInventory {
                unit,
                requesters,
                count,
            });
        }

        let mut requesters = [None; N];
        let mut count = 0usize;
        let mut requester_scopes = 0usize;
        let mut found_unit = false;
        for structure in self.entries() {
            let DmarStructure::Drhd {
                include_pci_all,
                segment,
                register_base,
                scopes,
            } = structure
            else {
                continue;
            };
            if register_base != unit.register_base {
                continue;
            }
            found_unit = true;
            if include_pci_all {
                return Err(RequesterInventoryError::IncludesAllPci);
            }
            for scope in DeviceScopes::new(scopes) {
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
                        let Some(Named::Function(endpoint)) = scope.walk(segment, &bridge) else {
                            return Err(RequesterInventoryError::UnresolvedScope);
                        };
                        if self
                            .unit_for(endpoint, &bridge, &hierarchy)
                            .is_none_or(|governing| governing.register_base != register_base)
                        {
                            return Err(RequesterInventoryError::ForeignEndpoint);
                        }
                        push_requester(&mut requesters, &mut count, endpoint)?;
                    }
                    DeviceScopeKind::PciSubHierarchy => {
                        let Some(Named::Hierarchy(scope_hierarchy)) = scope.walk(segment, &bridge)
                        else {
                            return Err(RequesterInventoryError::UnresolvedScope);
                        };
                        add_live_requester(
                            self,
                            &bridge,
                            &hierarchy,
                            register_base,
                            scope_hierarchy.root,
                            &mut requesters,
                            &mut count,
                        )?;
                        for bus in
                            scope_hierarchy.buses.secondary..=scope_hierarchy.buses.subordinate
                        {
                            for device in 0..32 {
                                for function in 0..8 {
                                    let requester = Bdf::new(segment, bus, device, function);
                                    if present(requester) {
                                        add_live_requester(
                                            self,
                                            &bridge,
                                            &hierarchy,
                                            register_base,
                                            requester,
                                            &mut requesters,
                                            &mut count,
                                        )?;
                                    }
                                }
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
        if !requesters[..count].contains(&Some(selected)) {
            return Err(RequesterInventoryError::MissingSelected);
        }
        Ok(RequesterInventory {
            unit,
            requesters,
            count,
        })
    }

    /// Every reserved memory region a device at `bdf` must have
    /// identity-mapped rather than translated.
    ///
    /// An RMRR's scopes resolve through `bridge` as [`Self::unit_for`]'s do,
    /// and its region applies when any of them names `bdf`, as itself or
    /// behind a named bridge.
    pub fn reserved_regions_for<F: Fn(Bdf) -> Option<BusRange>, H: Fn(Bdf, Bdf) -> bool>(
        &self,
        bdf: Bdf,
        bridge: F,
        hierarchy: H,
    ) -> ReservedRegions<'a, F, H> {
        ReservedRegions {
            entries: self.entries(),
            bdf,
            bridge,
            hierarchy,
        }
    }
}

/// The `(base, limit)` ranges [`Dmar::reserved_regions_for`] finds for one
/// device, inclusive of both ends.
#[derive(Debug)]
pub struct ReservedRegions<'a, F, H> {
    entries: DmarEntries<'a>,
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
