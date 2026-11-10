use crate::{
    Address, Class, DEVICES, FUNCTIONS, FunctionObservation, HeaderType, QualifiedNonRequester,
    QuiescedIsaRequester, QuiescedRequester, primary_bus, secondary_bus, subordinate_bus,
};

/// Buses in one legacy PCI segment.
const BUSES: usize = 256;

/// Function identities in one legacy PCI segment.
const SEGMENT_FUNCTIONS: usize = BUSES * DEVICES as usize * FUNCTIONS as usize;

/// Machine words needed to remember every function identity in one segment.
const PRESENCE_WORDS: usize = SEGMENT_FUNCTIONS.div_ceil(u64::BITS as usize);

/// A bridge range admitted into a sealed [`PciTopology`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BridgeRange {
    /// Bus immediately behind the bridge.
    pub secondary: u8,
    /// Highest bus the bridge forwards to.
    pub subordinate: u8,
}

/// Why a complete PCI topology snapshot cannot be trusted for DMA authority.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum TopologyError {
    /// The function named by entry-fence authority was absent from the later snapshot.
    QualifiedNonRequesterMissing {
        /// Exact function which had been classified at entry.
        at: Address,
    },
    /// The function named by entry-fence authority changed before the snapshot.
    QualifiedNonRequesterChanged {
        /// Exact function whose identity, class or command changed.
        at: Address,
    },
    /// The requester quiesced at entry was absent from the later snapshot.
    QuiescedRequesterMissing {
        /// Exact function which had been quiesced at entry.
        at: Address,
    },
    /// The requester quiesced at entry changed before the snapshot.
    QuiescedRequesterChanged {
        /// Exact function whose identity, class or command changed.
        at: Address,
    },
    /// The class and header layout disagree about whether a function is a bridge.
    BridgeIdentity {
        /// Function with contradictory bridge identity.
        at: Address,
    },
    /// A bridge function had no readable bus-number register.
    MissingBusNumbers {
        /// Bridge whose routing could not be read.
        at: Address,
    },
    /// A bridge's primary or forwarded range cannot describe a child bus.
    InvalidRange {
        /// Bridge with invalid bus numbers.
        at: Address,
        /// Primary bus encoded by the bridge.
        primary: u8,
        /// First forwarded bus.
        secondary: u8,
        /// Last forwarded bus.
        subordinate: u8,
    },
    /// Two different bridges claim to be one bus's direct parent.
    DuplicateParent {
        /// Bus with two parent bridges.
        bus: u8,
        /// First parent found.
        first: Address,
        /// Second parent found.
        second: Address,
    },
    /// A present function is on a bus with no rooted parent chain.
    UnrootedFunction {
        /// Present function outside the proved tree.
        at: Address,
    },
    /// A child bridge forwards beyond the range admitted by its parent.
    ChildEscapesParent {
        /// Child bridge whose range escapes.
        child: Address,
        /// Direct parent bridge.
        parent: Address,
    },
    /// Two unrelated bridge branches advertise intersecting ranges.
    UnrelatedOverlap {
        /// First bridge in the overlap.
        first: Address,
        /// Second bridge in the overlap.
        second: Address,
    },
}

impl core::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::QualifiedNonRequesterMissing { at } => write!(
                f,
                "entry-qualified PCI non-requester {:02x}:{:02x}.{} is absent from the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::QualifiedNonRequesterChanged { at } => write!(
                f,
                "entry-qualified PCI non-requester {:02x}:{:02x}.{} changed before the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::QuiescedRequesterMissing { at } => write!(
                f,
                "entry-quiesced PCI requester {:02x}:{:02x}.{} is absent from the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::QuiescedRequesterChanged { at } => write!(
                f,
                "entry-quiesced PCI requester {:02x}:{:02x}.{} changed before the sealed snapshot",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::BridgeIdentity { at } => write!(
                f,
                "PCI function {:02x}:{:02x}.{} has contradictory bridge class and header",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::MissingBusNumbers { at } => write!(
                f,
                "PCI bridge {:02x}:{:02x}.{} has no readable bus numbers",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::InvalidRange {
                at,
                primary,
                secondary,
                subordinate,
            } => write!(
                f,
                "PCI bridge {:02x}:{:02x}.{} reports invalid buses {primary:#04x}/{secondary:#04x}/{subordinate:#04x}",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::DuplicateParent { bus, first, second } => write!(
                f,
                "PCI bus {bus:#04x} has parent bridges {:02x}:{:02x}.{} and {:02x}:{:02x}.{}",
                first.bus(),
                first.device(),
                first.function(),
                second.bus(),
                second.device(),
                second.function()
            ),
            Self::UnrootedFunction { at } => write!(
                f,
                "PCI function {:02x}:{:02x}.{} has no unique path to bus zero",
                at.bus(),
                at.device(),
                at.function()
            ),
            Self::ChildEscapesParent { child, parent } => write!(
                f,
                "PCI bridge {:02x}:{:02x}.{} escapes parent {:02x}:{:02x}.{}",
                child.bus(),
                child.device(),
                child.function(),
                parent.bus(),
                parent.device(),
                parent.function()
            ),
            Self::UnrelatedOverlap { first, second } => write!(
                f,
                "unrelated PCI bridges {:02x}:{:02x}.{} and {:02x}:{:02x}.{} advertise overlapping buses",
                first.bus(),
                first.device(),
                first.function(),
                second.bus(),
                second.device(),
                second.function()
            ),
        }
    }
}

impl core::error::Error for TopologyError {}

/// A fixed, allocation-free snapshot of one rooted legacy PCI segment.
///
/// Each non-root bus is indexed by its unique direct parent bridge. Queries
/// return nothing until [`Self::seal`] proves that all present functions reach
/// bus zero, child ranges stay inside their parents, and unrelated branches
/// do not overlap. An error from any observation door invalidates the snapshot
/// and is returned by every later observation and seal until [`Self::reset`].
/// Observe each function once per snapshot; successful observations are not
/// replacement transactions.
#[derive(PartialEq, Eq, Debug)]
pub struct PciTopology {
    parent: [u32; BUSES],
    subordinate: [u8; BUSES],
    present: [u64; PRESENCE_WORDS],
    requesters: [u64; PRESENCE_WORDS],
    quiesced: Option<QuiescedRequester>,
    observation_failure: Option<TopologyError>,
    sealed: bool,
}

impl PciTopology {
    /// Begin an empty, unsealed segment-zero snapshot.
    pub const fn new() -> Self {
        Self {
            parent: [0; BUSES],
            subordinate: [0; BUSES],
            present: [0; PRESENCE_WORDS],
            requesters: [0; PRESENCE_WORDS],
            quiesced: None,
            observation_failure: None,
            sealed: false,
        }
    }

    /// Discard every observation and begin another snapshot.
    pub fn reset(&mut self) {
        self.parent.fill(0);
        self.subordinate.fill(0);
        self.present.fill(0);
        self.requesters.fill(0);
        self.quiesced = None;
        self.observation_failure = None;
        self.sealed = false;
    }

    fn begin_observation(&mut self) -> Result<(), TopologyError> {
        self.sealed = false;
        match self.observation_failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn retain_observation_failure(
        &mut self,
        result: Result<(), TopologyError>,
    ) -> Result<(), TopologyError> {
        if let Err(error) = result {
            self.observation_failure = Some(error);
        }
        result
    }

    /// Record one independently probed function and, for a bridge, its routing.
    ///
    /// # Errors
    ///
    /// Returns [`TopologyError`] for contradictory bridge identity, unreadable
    /// bus numbers, an invalid range, or a second direct parent for one bus.
    /// The first observation error is retained until [`Self::reset`].
    pub fn observe(
        &mut self,
        at: Address,
        class: Class,
        header: HeaderType,
        bus_numbers: Option<u32>,
    ) -> Result<(), TopologyError> {
        self.begin_observation()?;
        let result = self.observe_function(at, class, header, bus_numbers);
        self.retain_observation_failure(result)
    }

    fn observe_function(
        &mut self,
        at: Address,
        class: Class,
        header: HeaderType,
        bus_numbers: Option<u32>,
    ) -> Result<(), TopologyError> {
        let at = at.function_base();
        let index = function_index(at);
        self.present[index / u64::BITS as usize] |= 1 << (index % u64::BITS as usize);
        self.requesters[index / u64::BITS as usize] |= 1 << (index % u64::BITS as usize);

        if class.is_bridge() != header.is_bridge() {
            return Err(TopologyError::BridgeIdentity { at });
        }
        if !class.is_bridge() {
            return Ok(());
        }
        let Some(bits) = bus_numbers else {
            return Err(TopologyError::MissingBusNumbers { at });
        };
        let primary = primary_bus(bits);
        let secondary = secondary_bus(bits);
        let subordinate = subordinate_bus(bits);
        if primary != at.bus() || secondary <= at.bus() || secondary > subordinate {
            return Err(TopologyError::InvalidRange {
                at,
                primary,
                secondary,
                subordinate,
            });
        }
        let slot = &mut self.parent[usize::from(secondary)];
        if *slot != 0 && *slot != at.bits() {
            return Err(TopologyError::DuplicateParent {
                bus: secondary,
                first: Address(*slot),
                second: at,
            });
        }
        *slot = at.bits();
        self.subordinate[usize::from(secondary)] = subordinate;
        Ok(())
    }

    /// Record an exact platform-qualified function that is present but is not
    /// a PCI DMA requester.
    ///
    /// # Errors
    ///
    /// The same topology errors as [`Self::observe`]. The consumed authority
    /// can name only the exact platform tuple recognized by the handoff policy.
    pub fn observe_non_requester(
        &mut self,
        authority: QualifiedNonRequester,
        observation: FunctionObservation,
    ) -> Result<(), TopologyError> {
        self.begin_observation()?;
        let FunctionObservation {
            at,
            identity,
            class_bits,
            command,
            class,
            header,
            bus_numbers,
        } = observation;
        let at = at.function_base();
        if at != authority.at
            || identity != authority.identity
            || class_bits != authority.class
            || command != authority.command
        {
            return self.retain_observation_failure(Err(
                TopologyError::QualifiedNonRequesterChanged { at },
            ));
        }
        self.observe(at, class, header, bus_numbers)?;
        let index = function_index(at);
        self.requesters[index / u64::BITS as usize] &= !(1 << (index % u64::BITS as usize));
        Ok(())
    }

    /// Record the exact entry-quiesced ISA requester, which stays a requester.
    ///
    /// # Errors
    ///
    /// The same topology errors as [`Self::observe`], or
    /// [`TopologyError::QuiescedRequesterChanged`] when any field the entry
    /// fence bound differs.
    pub fn observe_quiesced_requester(
        &mut self,
        authority: QuiescedIsaRequester,
        observation: FunctionObservation,
    ) -> Result<(), TopologyError> {
        self.begin_observation()?;
        let at = observation.at.function_base();
        let observed = (
            at,
            observation.identity,
            observation.class_bits,
            observation.command,
        );
        let bound = (
            authority.at,
            authority.identity,
            authority.class,
            authority.command,
        );
        if observed != bound {
            return self
                .retain_observation_failure(Err(TopologyError::QuiescedRequesterChanged { at }));
        }
        self.observe(
            at,
            observation.class,
            observation.header,
            observation.bus_numbers,
        )?;
        self.quiesced = Some(QuiescedRequester {
            at,
            identity: authority.identity,
            command: authority.command,
        });
        Ok(())
    }

    /// The entry-quiesced requester at `at`, once the snapshot is sealed.
    pub fn quiesced_requester(&self, at: Address) -> Option<QuiescedRequester> {
        if !self.sealed {
            return None;
        }
        self.quiesced
            .filter(|record| record.at == at.function_base())
    }

    /// Seal the snapshot after validating its complete rooted interval tree.
    ///
    /// # Errors
    ///
    /// Returns [`TopologyError`] when a present function is unrooted, a child
    /// range escapes its parent, or unrelated branches overlap. A retained
    /// observation error is returned first and requires [`Self::reset`].
    pub fn seal(&mut self) -> Result<(), TopologyError> {
        self.sealed = false;
        if let Some(error) = self.observation_failure {
            return Err(error);
        }
        for index in 0..SEGMENT_FUNCTIONS {
            if self.present[index / u64::BITS as usize] & (1 << (index % u64::BITS as usize)) == 0 {
                continue;
            }
            let at = address_for_index(index);
            if at.bus() != 0 && !self.bus_reaches_root(at.bus()) {
                return Err(TopologyError::UnrootedFunction { at });
            }
        }

        for secondary in 1u16..=u16::from(u8::MAX) {
            let secondary = secondary as u8;
            let Some(child) = self.parent(secondary) else {
                continue;
            };
            if child.bus() == 0 {
                continue;
            }
            let Some(parent) = self.parent(child.bus()) else {
                return Err(TopologyError::UnrootedFunction { at: child });
            };
            let parent_secondary = self.secondary_for(parent);
            if secondary < parent_secondary
                || self.subordinate[usize::from(secondary)]
                    > self.subordinate[usize::from(parent_secondary)]
            {
                return Err(TopologyError::ChildEscapesParent { child, parent });
            }
        }

        for first_bus in 1u16..=u16::from(u8::MAX) {
            let first_bus = first_bus as u8;
            let Some(first) = self.parent(first_bus) else {
                continue;
            };
            for second_bus in (u16::from(first_bus) + 1)..=u16::from(u8::MAX) {
                let second_bus = second_bus as u8;
                let Some(second) = self.parent(second_bus) else {
                    continue;
                };
                let overlap = first_bus <= self.subordinate[usize::from(second_bus)]
                    && second_bus <= self.subordinate[usize::from(first_bus)];
                if overlap
                    && !self.bridge_is_ancestor(first, second)
                    && !self.bridge_is_ancestor(second, first)
                {
                    return Err(TopologyError::UnrelatedOverlap { first, second });
                }
            }
        }
        self.sealed = true;
        Ok(())
    }

    /// Whether the complete snapshot passed validation.
    pub const fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Whether one exact function was present in the sealed snapshot.
    pub fn contains_function(&self, at: Address) -> bool {
        if !self.sealed {
            return false;
        }
        let index = function_index(at.function_base());
        self.present[index / u64::BITS as usize] & (1 << (index % u64::BITS as usize)) != 0
    }

    /// Whether one exact function is a DMA requester in the sealed snapshot.
    pub fn contains_requester(&self, at: Address) -> bool {
        if !self.sealed {
            return false;
        }
        let index = function_index(at.function_base());
        self.requesters[index / u64::BITS as usize] & (1 << (index % u64::BITS as usize)) != 0
    }

    /// The sealed routing range of one exact bridge function.
    pub fn bridge_range(&self, at: Address) -> Option<BridgeRange> {
        if !self.sealed {
            return None;
        }
        let at = at.function_base();
        for secondary in 1u16..=u16::from(u8::MAX) {
            let secondary = secondary as u8;
            if self.parent(secondary) == Some(at) {
                return Some(BridgeRange {
                    secondary,
                    subordinate: self.subordinate[usize::from(secondary)],
                });
            }
        }
        None
    }

    /// Whether `target` is `root` or lies below that bridge in the sealed tree.
    pub fn hierarchy_contains(&self, root: Address, target: Address) -> bool {
        if !self.sealed {
            return false;
        }
        let root = root.function_base();
        let target = target.function_base();
        if root == target {
            return self.contains_function(root);
        }
        self.bridge_range(root).is_some()
            && self.contains_function(target)
            && self.bridge_is_ancestor(root, target)
    }

    fn parent(&self, bus: u8) -> Option<Address> {
        let bits = self.parent[usize::from(bus)];
        (bits != 0).then_some(Address(bits))
    }

    fn secondary_for(&self, bridge: Address) -> u8 {
        for secondary in 1u16..=u16::from(u8::MAX) {
            let secondary = secondary as u8;
            if self.parent(secondary) == Some(bridge) {
                return secondary;
            }
        }
        0
    }

    fn bus_reaches_root(&self, mut bus: u8) -> bool {
        for _ in 0..BUSES_BELOW_ROOT {
            let Some(parent) = self.parent(bus) else {
                return false;
            };
            bus = parent.bus();
            if bus == 0 {
                return true;
            }
        }
        false
    }

    fn bridge_is_ancestor(&self, ancestor: Address, descendant: Address) -> bool {
        let ancestor = ancestor.function_base();
        let mut bus = descendant.bus();
        for _ in 0..BUSES_BELOW_ROOT {
            let Some(parent) = self.parent(bus) else {
                return false;
            };
            if parent == ancestor {
                return true;
            }
            bus = parent.bus();
            if bus == 0 {
                return false;
            }
        }
        false
    }
}

impl Default for PciTopology {
    fn default() -> Self {
        Self::new()
    }
}

/// Non-root buses in one segment; parent walks must strictly approach bus zero.
const BUSES_BELOW_ROOT: usize = BUSES - 1;

fn function_index(at: Address) -> usize {
    (usize::from(at.bus()) * usize::from(DEVICES) + usize::from(at.device()))
        * usize::from(FUNCTIONS)
        + usize::from(at.function())
}

fn address_for_index(index: usize) -> Address {
    let bus = index / (usize::from(DEVICES) * usize::from(FUNCTIONS));
    let on_bus = index % (usize::from(DEVICES) * usize::from(FUNCTIONS));
    let device = on_bus / usize::from(FUNCTIONS);
    let function = on_bus % usize::from(FUNCTIONS);
    Address(
        Address::ENABLE | ((bus as u32) << 16) | ((device as u32) << 11) | ((function as u32) << 8),
    )
}
