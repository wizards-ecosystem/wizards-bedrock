//! Pure admission rules for paired-root page-table isolation.

use core::fmt;

use crate::{Entry, Flags, MAX_PHYSICAL, PAGE_SIZE, page_identity_matches};

const HIGHER_HALF_START: u64 = 0xffff_8000_0000_0000;

/// The physical roots of one inseparable full/stripped address-space pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RootPair {
    full: u64,
    stripped: u64,
}

impl RootPair {
    /// Admit two distinct, aligned and encodable page-table roots.
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
        Ok(Self { full, stripped })
    }

    /// The root under which ordinary kernel code may execute.
    pub const fn full(self) -> u64 {
        self.full
    }

    /// The root installed only for execution at CPL 3 and enclave assembly.
    pub const fn stripped(self) -> u64 {
        self.stripped
    }
}

/// Why a full/stripped root pair cannot be admitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum RootPairError {
    /// The full root is not page-aligned.
    MisalignedFull(u64),
    /// The stripped root is not page-aligned.
    MisalignedStripped(u64),
    /// The full root cannot be encoded in a page-table entry.
    FullOutOfRange(u64),
    /// The stripped root cannot be encoded in a page-table entry.
    StrippedOutOfRange(u64),
    /// Both roles name the same root.
    SameRoot(u64),
}

impl fmt::Display for RootPairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MisalignedFull(root) => {
                write!(f, "full root {root:#x} is not page-aligned")
            }
            Self::MisalignedStripped(root) => {
                write!(f, "stripped root {root:#x} is not page-aligned")
            }
            Self::FullOutOfRange(root) => {
                write!(f, "full root {root:#x} does not fit 52 bits")
            }
            Self::StrippedOutOfRange(root) => {
                write!(f, "stripped root {root:#x} does not fit 52 bits")
            }
            Self::SameRoot(root) => write!(f, "both roots name {root:#x}"),
        }
    }
}

impl core::error::Error for RootPairError {}

/// Frame accounting while a pair is built, so a refusal after one root returns that one frame.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct PairConstructionLedger {
    root_frames: u8,
}

impl PairConstructionLedger {
    /// Begin construction before either root frame has been acquired.
    pub const fn new() -> Self {
        Self { root_frames: 0 }
    }

    /// Record one newly acquired root frame, refusing a third.
    pub const fn acquire_root(self) -> Option<Self> {
        if self.root_frames >= 2 {
            return None;
        }
        Some(Self {
            root_frames: self.root_frames + 1,
        })
    }

    /// Publish complete pair-frame authority after both roots exist.
    pub const fn publish(self) -> Option<PairFrameAuthority> {
        if self.root_frames != 2 {
            return None;
        }
        Some(PairFrameAuthority { table_frames: 0 })
    }

    #[cfg(test)]
    const fn refusal_returned_all(self, returned_frames: usize) -> bool {
        returned_frames == self.root_frames as usize
    }
}

/// Complete frame authority for one inseparable full/stripped root pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PairFrameAuthority {
    table_frames: usize,
}

impl PairFrameAuthority {
    const ROOT_FRAMES: usize = 2;

    /// Extend the authority by one newly acquired intermediate table frame.
    pub const fn acquire_table(self) -> Option<Self> {
        let Some(table_frames) = self.table_frames.checked_add(1) else {
            return None;
        };
        Some(Self { table_frames })
    }

    /// Remove one intermediate table frame after rollback returned it.
    pub const fn release_table(self) -> Option<Self> {
        let Some(table_frames) = self.table_frames.checked_sub(1) else {
            return None;
        };
        Some(Self { table_frames })
    }

    /// The intermediate table frames retained by this pair.
    pub const fn table_frames(self) -> usize {
        self.table_frames
    }

    /// Settle an all-or-nothing cleanup: none retains, all releases, and a partial count fails.
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

    #[cfg(test)]
    fn refusal_returned_all(self, returned_frames: usize) -> bool {
        self.settle(returned_frames) == Ok(PairCleanupState::Released)
    }
}

/// The only valid outcomes of a pair cleanup transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PairCleanupState {
    /// Cleanup refused before returning a frame, so the whole pair remains.
    Retained(PairFrameAuthority),
    /// Both roots and every intermediate table were returned.
    Released,
}

/// Why observed cleanup cannot preserve or release complete pair authority.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PairCleanupError {
    /// The complete pair's frame count cannot be represented.
    CountOverflow,
    /// Cleanup returned some but not all of the pair's frames.
    Incomplete {
        /// The two roots plus every retained intermediate table.
        expected: usize,
        /// The number of frames cleanup reported returning.
        returned: usize,
    },
}

impl fmt::Display for PairCleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::CountOverflow => f.write_str("paired frame count overflows"),
            Self::Incomplete { expected, returned } => {
                write!(f, "paired cleanup returned {returned} of {expected} frames")
            }
        }
    }
}

impl core::error::Error for PairCleanupError {}

/// The live state of `CR4.PCIDE`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PcidState {
    /// Address-space identifiers are disabled.
    Disabled,
    /// Address-space identifiers are enabled.
    Enabled,
}

/// Evidence for the ordinary-`CR3`-load flush policy used by paired roots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FlushPremise {
    leaf_count: usize,
}

impl FlushPremise {
    /// Admit the flush policy over the flags of every present leaf either root reaches.
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
        Ok(Self {
            leaf_count: leaf_domain.len(),
        })
    }
}

/// Why the paired-root flush premise is absent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum FlushPremiseError {
    /// `CR4.PCIDE` is set.
    PcidEnabled,
    /// No present leaf was included in the claimed complete observation.
    EmptyDomain,
    /// One entry in the supplied complete leaf domain is global.
    GlobalLeaf(usize),
}

impl fmt::Display for FlushPremiseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PcidEnabled => f.write_str("PCID is enabled"),
            Self::EmptyDomain => f.write_str("the leaf domain is empty"),
            Self::GlobalLeaf(index) => write!(f, "leaf {index} is global"),
        }
    }
}

impl core::error::Error for FlushPremiseError {}

/// Whether a lower-half top-level slot is absent in both or links one user table identically.
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

/// The only permissions an entry-enclave page may have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnclavePermission {
    /// Read-only executable entry assembly.
    Executable,
    /// Read-only non-executable descriptor or routing data.
    ReadOnly,
    /// Writable non-executable transition- or IST-stack storage.
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

/// One expected page in the exact stripped-root entry enclave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EnclavePage {
    virtual_address: u64,
    physical_address: u64,
    permission: EnclavePermission,
}

impl EnclavePage {
    /// Describe one page-aligned, higher-half enclave page with encodable backing.
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

    /// The virtual page this descriptor admits.
    pub const fn virtual_address(self) -> u64 {
        self.virtual_address
    }

    /// The one physical backing page this descriptor admits.
    pub const fn physical_address(self) -> u64 {
        self.physical_address
    }

    /// The exact stripped-root permission for this page.
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

/// Why an expected entry-enclave page cannot be represented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum EnclavePageError {
    /// The virtual address is not page-aligned.
    MisalignedVirtual(u64),
    /// The virtual address is not in the canonical higher half.
    OutsideHigherHalf(u64),
    /// The physical address is not page-aligned.
    MisalignedPhysical(u64),
    /// The physical address cannot be encoded in a page-table entry.
    PhysicalOutOfRange(u64),
}

impl fmt::Display for EnclavePageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MisalignedVirtual(address) => {
                write!(f, "enclave address {address:#x} is not page-aligned")
            }
            Self::OutsideHigherHalf(address) => {
                write!(f, "enclave address {address:#x} is not in the higher half")
            }
            Self::MisalignedPhysical(address) => {
                write!(f, "enclave backing {address:#x} is not page-aligned")
            }
            Self::PhysicalOutOfRange(address) => {
                write!(f, "enclave backing {address:#x} does not fit 52 bits")
            }
        }
    }
}

impl core::error::Error for EnclavePageError {}

/// The role of one guarded stack reachable before the full-root load.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryStackKind {
    /// The CPU-local stack used for ordinary privilege crossings.
    Transition,
    /// One interrupt-stack-table stack reachable during early entry.
    Ist,
}

/// One page-aligned entry stack with its adjacent lower guard derived.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EntryStackRange {
    kind: EntryStackKind,
    start: u64,
    len: u64,
    lower_guard: u64,
}

impl EntryStackRange {
    /// Admit one non-empty higher-half stack and derive its lower guard page.
    pub const fn new(
        kind: EntryStackKind,
        start: u64,
        len: u64,
    ) -> Result<Self, EntryStackRangeError> {
        if !start.is_multiple_of(PAGE_SIZE) {
            return Err(EntryStackRangeError::MisalignedStart(start));
        }
        if len == 0 {
            return Err(EntryStackRangeError::Empty);
        }
        if !len.is_multiple_of(PAGE_SIZE) {
            return Err(EntryStackRangeError::MisalignedLength(len));
        }
        let Some(lower_guard) = start.checked_sub(PAGE_SIZE) else {
            return Err(EntryStackRangeError::OutsideHigherHalf(start));
        };
        if lower_guard < HIGHER_HALF_START {
            return Err(EntryStackRangeError::OutsideHigherHalf(start));
        }
        let Some(last) = start.checked_add(len - 1) else {
            return Err(EntryStackRangeError::EndOverflow);
        };
        if last < HIGHER_HALF_START {
            return Err(EntryStackRangeError::OutsideHigherHalf(last));
        }
        Ok(Self {
            kind,
            start,
            len,
            lower_guard,
        })
    }

    /// Whether this is the ordinary transition stack or an IST stack.
    pub const fn kind(self) -> EntryStackKind {
        self.kind
    }

    /// The first writable stack page.
    pub const fn start(self) -> u64 {
        self.start
    }

    /// The byte length of the writable stack payload.
    pub const fn len(self) -> u64 {
        self.len
    }

    /// The adjacent page immediately below the writable payload.
    pub const fn lower_guard(self) -> u64 {
        self.lower_guard
    }

    /// Whether the writable stack payload is empty.
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Why an entry-stack range cannot establish lower-guard geometry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum EntryStackRangeError {
    /// The first stack byte is not page-aligned.
    MisalignedStart(u64),
    /// The stack has no writable payload.
    Empty,
    /// The stack length is not page-aligned.
    MisalignedLength(u64),
    /// The payload's exclusive end overflows.
    EndOverflow,
    /// The payload or its lower guard is outside the canonical higher half.
    OutsideHigherHalf(u64),
}

impl fmt::Display for EntryStackRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MisalignedStart(start) => {
                write!(f, "entry stack start {start:#x} is not page-aligned")
            }
            Self::Empty => f.write_str("the entry stack is empty"),
            Self::MisalignedLength(len) => {
                write!(f, "entry stack length {len} is not page-aligned")
            }
            Self::EndOverflow => f.write_str("entry stack end overflows"),
            Self::OutsideHigherHalf(address) => write!(
                f,
                "entry stack address {address:#x} is not in the higher half"
            ),
        }
    }
}

impl core::error::Error for EntryStackRangeError {}

/// One leaf observed while walking the stripped root's higher half.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ObservedPage {
    virtual_address: u64,
    physical_address: u64,
    span: u64,
    flags: Flags,
}

impl ObservedPage {
    /// Record the complete result of one stripped-root leaf walk.
    pub const fn new(virtual_address: u64, physical_address: u64, span: u64, flags: Flags) -> Self {
        Self {
            virtual_address,
            physical_address,
            span,
            flags,
        }
    }
}

/// Whether no stack guard is covered by any leaf of either root; a malformed leaf fails closed.
pub fn stack_guards_are_absent(
    stacks: &[EntryStackRange],
    full_present_pages: &[ObservedPage],
    stripped_present_pages: &[ObservedPage],
) -> bool {
    if stacks.is_empty() {
        return false;
    }
    for observed in full_present_pages
        .iter()
        .chain(stripped_present_pages.iter())
    {
        if !observed.flags.contains(Flags::PRESENT)
            || !observed.virtual_address.is_multiple_of(PAGE_SIZE)
            || !observed.physical_address.is_multiple_of(PAGE_SIZE)
            || observed.span == 0
            || !observed.span.is_multiple_of(PAGE_SIZE)
        {
            return false;
        }
        let Some(end) = observed.virtual_address.checked_add(observed.span) else {
            return false;
        };
        for stack in stacks {
            if observed.virtual_address <= stack.lower_guard && stack.lower_guard < end {
                return false;
            }
        }
    }
    true
}

/// Whether every present stripped higher-half leaf, in any order, is exactly the expected enclave.
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

/// One security-relevant event while entering the kernel from CPL 3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryEvent {
    /// Load the pair's full root.
    LoadFullRoot,
    /// Transfer the saved frame to the ordinary kernel stack.
    TransferToKernelStack,
    /// Clear the transition-stack payload.
    ScrubTransitionStack,
    /// Call the first Rust body.
    CallRust,
}

/// Pure state for validating the CPL-3-to-kernel transition order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EntryTransition {
    stage: u8,
}

impl EntryTransition {
    /// Begin while the stripped root and transition stack are active.
    pub const fn new() -> Self {
        Self { stage: 0 }
    }

    /// Advance by one event, refusing an event that arrives out of order.
    pub const fn advance(self, event: EntryEvent) -> Option<Self> {
        let expected = match self.stage {
            0 => EntryEvent::LoadFullRoot,
            1 => EntryEvent::TransferToKernelStack,
            2 => EntryEvent::ScrubTransitionStack,
            3 => EntryEvent::CallRust,
            _ => return None,
        };
        if event as u8 == expected as u8 {
            Some(Self {
                stage: self.stage + 1,
            })
        } else {
            None
        }
    }

    /// Whether the full ordered transition reached Rust.
    pub const fn is_complete(self) -> bool {
        self.stage == 4
    }
}

impl Default for EntryTransition {
    fn default() -> Self {
        Self::new()
    }
}

/// One security-relevant event while returning from the kernel to CPL 3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitEvent {
    /// Validate every user-visible register and return-frame field.
    ValidateUserState,
    /// Clear the complete transition-stack payload.
    ScrubTransitionStack,
    /// Copy only validated user-visible state to the transition stack.
    CopyUserState,
    /// Perform the final access to potentially secret memory.
    LastSecretAccess,
    /// Load the pair's stripped root.
    LoadStrippedRoot,
    /// Execute the required post-root `VERW` barrier.
    Verw,
    /// Execute the final user return.
    ReturnToUser,
}

/// Pure state for validating the kernel-to-CPL-3 transition order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExitTransition {
    stage: u8,
}

impl ExitTransition {
    /// Begin while the full root and ordinary kernel stack are active.
    pub const fn new() -> Self {
        Self { stage: 0 }
    }

    /// Advance by one event, refusing an event that arrives out of order.
    pub const fn advance(self, event: ExitEvent) -> Option<Self> {
        let expected = match self.stage {
            0 => ExitEvent::ValidateUserState,
            1 => ExitEvent::ScrubTransitionStack,
            2 => ExitEvent::CopyUserState,
            3 => ExitEvent::LastSecretAccess,
            4 => ExitEvent::LoadStrippedRoot,
            5 => ExitEvent::Verw,
            6 => ExitEvent::ReturnToUser,
            _ => return None,
        };
        if event as u8 == expected as u8 {
            Some(Self {
                stage: self.stage + 1,
            })
        } else {
            None
        }
    }

    /// Whether the full ordered transition reached CPL 3.
    pub const fn is_complete(self) -> bool {
        self.stage == 7
    }
}

impl Default for ExitTransition {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
