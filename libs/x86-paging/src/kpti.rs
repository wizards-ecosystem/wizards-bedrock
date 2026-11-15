use core::fmt;

use crate::{Entry, Flags, MAX_PHYSICAL, PAGE_SIZE, page_identity_matches};

const HIGHER_HALF_START: u64 = 0xffff_8000_0000_0000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RootPair(());

impl RootPair {
    pub const fn new(full: u64, stripped: u64) -> Result<Self, RootPairError> {
        if !full.is_multiple_of(PAGE_SIZE) {
            return Err(RootPairError::MisalignedFull(full));
        }
        if !stripped.is_multiple_of(PAGE_SIZE) {
            return Err(RootPairError::MisalignedStripped(stripped));
        }
        if full >= MAX_PHYSICAL {
            return Err(RootPairError::FullOutOfRange(full));
        }
        if stripped >= MAX_PHYSICAL {
            return Err(RootPairError::StrippedOutOfRange(stripped));
        }
        if full == stripped {
            return Err(RootPairError::SameRoot(full));
        }
        Ok(Self(()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RootPairError {
    MisalignedFull(u64),
    MisalignedStripped(u64),
    FullOutOfRange(u64),
    StrippedOutOfRange(u64),
    SameRoot(u64),
}

impl fmt::Display for RootPairError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MisalignedFull(root) => {
                write!(formatter, "full root {root:#x} is not page-aligned")
            }
            Self::MisalignedStripped(root) => {
                write!(formatter, "stripped root {root:#x} is not page-aligned")
            }
            Self::FullOutOfRange(root) => {
                write!(formatter, "full root {root:#x} does not fit 52 bits")
            }
            Self::StrippedOutOfRange(root) => {
                write!(formatter, "stripped root {root:#x} does not fit 52 bits")
            }
            Self::SameRoot(root) => write!(formatter, "both roots name {root:#x}"),
        }
    }
}

impl core::error::Error for RootPairError {}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct PairConstructionLedger {
    root_frames: u8,
}

impl PairConstructionLedger {
    pub const fn new() -> Self {
        Self { root_frames: 0 }
    }

    pub const fn acquire_root(self) -> Option<Self> {
        if self.root_frames >= 2 {
            return None;
        }
        Some(Self {
            root_frames: self.root_frames + 1,
        })
    }

    pub const fn publish(self) -> Option<PairFrameCount> {
        if self.root_frames != 2 {
            return None;
        }
        Some(PairFrameCount { table_frames: 0 })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PairFrameCount {
    table_frames: usize,
}

impl PairFrameCount {
    const ROOT_FRAMES: usize = 2;

    pub const fn acquire_table(self) -> Option<Self> {
        let Some(table_frames) = self.table_frames.checked_add(1) else {
            return None;
        };
        Some(Self { table_frames })
    }

    pub const fn release_table(self) -> Option<Self> {
        let Some(table_frames) = self.table_frames.checked_sub(1) else {
            return None;
        };
        Some(Self { table_frames })
    }

    pub const fn table_frames(self) -> usize {
        self.table_frames
    }

    pub const fn settle(
        self,
        returned_frames: usize,
    ) -> Result<PairCleanupState, PairCleanupError> {
        let Some(expected) = Self::ROOT_FRAMES.checked_add(self.table_frames) else {
            return Err(PairCleanupError::CountOverflow);
        };
        if returned_frames == 0 {
            return Ok(PairCleanupState::Retained(self));
        }
        if returned_frames == expected {
            return Ok(PairCleanupState::Released);
        }
        Err(PairCleanupError::Incomplete {
            expected,
            returned: returned_frames,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairCleanupState {
    Retained(PairFrameCount),
    Released,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairCleanupError {
    CountOverflow,
    Incomplete { expected: usize, returned: usize },
}

impl fmt::Display for PairCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::CountOverflow => formatter.write_str("paired frame count overflows"),
            Self::Incomplete { expected, returned } => {
                write!(
                    formatter,
                    "paired cleanup returned {returned} of {expected} frames"
                )
            }
        }
    }
}

impl core::error::Error for PairCleanupError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PcidState {
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlushPremise(());

impl FlushPremise {
    // `leaf_domain` must hold the flags of every present leaf reachable through either root.
    pub fn new(pcid: PcidState, leaf_domain: &[Flags]) -> Result<Self, FlushPremiseError> {
        if matches!(pcid, PcidState::Enabled) {
            return Err(FlushPremiseError::PcidEnabled);
        }
        if leaf_domain.is_empty() {
            return Err(FlushPremiseError::EmptyDomain);
        }
        for (index, flags) in leaf_domain.iter().enumerate() {
            if flags.contains(Flags::GLOBAL) {
                return Err(FlushPremiseError::GlobalLeaf(index));
            }
        }
        Ok(Self(()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlushPremiseError {
    PcidEnabled,
    EmptyDomain,
    GlobalLeaf(usize),
}

impl fmt::Display for FlushPremiseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PcidEnabled => formatter.write_str("PCID is enabled"),
            Self::EmptyDomain => formatter.write_str("the leaf domain is empty"),
            Self::GlobalLeaf(index) => write!(formatter, "leaf {index} is global"),
        }
    }
}

impl core::error::Error for FlushPremiseError {}

pub const fn shared_user_link_matches(full: Entry, stripped: Entry) -> bool {
    if !full.is_present() || !stripped.is_present() {
        return full.bits() == Entry::ABSENT.bits() && stripped.bits() == Entry::ABSENT.bits();
    }

    let permitted = Flags::PRESENT
        .union(Flags::WRITABLE)
        .union(Flags::USER)
        .union(Flags::ACCESSED);
    full.table_identity_matches(stripped)
        && full.flags().difference(permitted).bits() == 0
        && full.flags().contains(Flags::PRESENT)
        && full.flags().contains(Flags::WRITABLE)
        && full.flags().contains(Flags::USER)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnclavePermission {
    Executable,
    ReadOnly,
    Writable,
}

impl EnclavePermission {
    const fn flags(self) -> Flags {
        match self {
            Self::Executable => Flags::PRESENT,
            Self::ReadOnly => Flags::PRESENT.union(Flags::NO_EXECUTE),
            Self::Writable => Flags::PRESENT
                .union(Flags::WRITABLE)
                .union(Flags::NO_EXECUTE),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EnclavePage {
    virtual_address: u64,
    physical_address: u64,
    permission: EnclavePermission,
}

impl EnclavePage {
    pub const fn new(
        virtual_address: u64,
        physical_address: u64,
        permission: EnclavePermission,
    ) -> Result<Self, EnclavePageError> {
        if !virtual_address.is_multiple_of(PAGE_SIZE) {
            return Err(EnclavePageError::MisalignedVirtual(virtual_address));
        }
        if virtual_address < HIGHER_HALF_START {
            return Err(EnclavePageError::OutsideHigherHalf(virtual_address));
        }
        if !physical_address.is_multiple_of(PAGE_SIZE) {
            return Err(EnclavePageError::MisalignedPhysical(physical_address));
        }
        if physical_address >= MAX_PHYSICAL {
            return Err(EnclavePageError::PhysicalOutOfRange(physical_address));
        }
        Ok(Self {
            virtual_address,
            physical_address,
            permission,
        })
    }

    pub const fn virtual_address(self) -> u64 {
        self.virtual_address
    }

    pub const fn physical_address(self) -> u64 {
        self.physical_address
    }

    pub const fn permission(self) -> EnclavePermission {
        self.permission
    }

    fn matches(self, observed: ObservedPage) -> bool {
        self.virtual_address == observed.virtual_address
            && page_identity_matches(
                observed.physical_address,
                observed.flags,
                observed.span,
                self.physical_address,
                self.permission.flags(),
            )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnclavePageError {
    MisalignedVirtual(u64),
    OutsideHigherHalf(u64),
    MisalignedPhysical(u64),
    PhysicalOutOfRange(u64),
}

impl fmt::Display for EnclavePageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MisalignedVirtual(address) => {
                write!(
                    formatter,
                    "enclave address {address:#x} is not page-aligned"
                )
            }
            Self::OutsideHigherHalf(address) => {
                write!(
                    formatter,
                    "enclave address {address:#x} is not in the higher half"
                )
            }
            Self::MisalignedPhysical(address) => {
                write!(
                    formatter,
                    "enclave backing {address:#x} is not page-aligned"
                )
            }
            Self::PhysicalOutOfRange(address) => {
                write!(
                    formatter,
                    "enclave backing {address:#x} does not fit 52 bits"
                )
            }
        }
    }
}

impl core::error::Error for EnclavePageError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObservedPage {
    virtual_address: u64,
    physical_address: u64,
    span: u64,
    flags: Flags,
}

impl ObservedPage {
    pub const fn new(virtual_address: u64, physical_address: u64, span: u64, flags: Flags) -> Self {
        Self {
            virtual_address,
            physical_address,
            span,
            flags,
        }
    }
}

// `observed` must hold every present stripped higher-half leaf: an omitted one reads as absent.
pub fn enclave_matches(expected: &[EnclavePage], observed: &[ObservedPage]) -> bool {
    if expected.is_empty() || expected.len() != observed.len() {
        return false;
    }

    for (index, page) in expected.iter().enumerate() {
        if expected[..index].iter().any(|prior| {
            prior.virtual_address == page.virtual_address
                || prior.physical_address == page.physical_address
        }) {
            return false;
        }
        let mut match_count = 0;
        for candidate in observed {
            if candidate.virtual_address == page.virtual_address {
                match_count += 1;
                if !page.matches(*candidate) {
                    return false;
                }
            }
        }
        if match_count != 1 {
            return false;
        }
    }
    true
}
