//! Pure admission rules for paired-root page-table isolation.

use core::fmt;

use crate::{Entry, Flags, MAX_PHYSICAL, PAGE_SIZE, page_identity_matches};

const HIGHER_HALF_START: u64 = 0xffff_8000_0000_0000;

/// The physical roots of one inseparable full/stripped address-space pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootPair {
    full: u64,
    stripped: u64,
}

impl RootPair {
    /// Admit two distinct, encodable page-table roots.
    ///
    /// # Errors
    ///
    /// Refuses either unaligned or unencodable root, or a pair that names the
    /// same root twice.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Frame accounting while a full/stripped root pair is being constructed.
///
/// Root acquisition is deliberately separate from published pair authority:
/// a refusal after the first root must return that one frame, while a complete
/// pair can move into [`PairFrameAuthority`] only after both roots exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PairConstructionLedger {
    root_frames: u8,
}

impl PairConstructionLedger {
    /// Begin construction before either root frame has been acquired.
    pub const fn new() -> Self {
        Self { root_frames: 0 }
    }

    /// Record one newly acquired root frame.
    ///
    /// Returns `None` rather than permitting a third root to enter the pair.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Settle an all-or-nothing cleanup observation.
    ///
    /// Returning no frames preserves the complete authority for retry. An
    /// exact return releases it. Any partial count is rejected because it
    /// would describe a half-reclaimed pair.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairCleanupState {
    /// Cleanup refused before returning a frame, so the whole pair remains.
    Retained(PairFrameAuthority),
    /// Both roots and every intermediate table were returned.
    Released,
}

/// Why observed cleanup cannot preserve or release complete pair authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::CountOverflow => {
                core::fmt::Write::write_str(formatter, "paired frame count overflows")
            }
            Self::Incomplete { expected, returned } => write!(
                formatter,
                "paired cleanup returned {returned} of {expected} frames"
            ),
        }
    }
}

impl core::error::Error for PairCleanupError {}

/// The live state of `CR4.PCIDE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcidState {
    /// Address-space identifiers are disabled.
    Disabled,
    /// Address-space identifiers are enabled.
    Enabled,
}

/// Evidence for the ordinary-`CR3`-load flush policy used by paired roots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlushPremise {
    leaf_count: usize,
}

impl FlushPremise {
    /// Admit the flush policy over one complete leaf-domain observation.
    ///
    /// `leaf_domain` must contain the flags of every present leaf reachable
    /// through either member of the pair. The runtime owns that exhaustive
    /// walk; this admission makes the no-PCID and all-non-global predicates
    /// inseparable once the observation has been supplied.
    ///
    /// # Errors
    ///
    /// Refuses enabled PCID or the first observed global leaf.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PcidEnabled => core::fmt::Write::write_str(formatter, "PCID is enabled"),
            Self::EmptyDomain => core::fmt::Write::write_str(formatter, "the leaf domain is empty"),
            Self::GlobalLeaf(index) => write!(formatter, "leaf {index} is global"),
        }
    }
}

impl core::error::Error for FlushPremiseError {}

/// Whether one lower-half top-level slot has the same shared user link.
///
/// Two absent slots match. Present slots must point to one identical user
/// table with exactly the ordinary table-link policy; hardware `ACCESSED`
/// drift does not change their identity.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnclavePage {
    virtual_address: u64,
    physical_address: u64,
    permission: EnclavePermission,
}

impl EnclavePage {
    /// Describe one page-aligned, higher-half enclave page.
    ///
    /// # Errors
    ///
    /// Refuses an unaligned virtual or physical address, a virtual address
    /// outside the canonical higher half, or an unencodable physical address.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::MisalignedPhysical(address) => write!(
                formatter,
                "enclave backing {address:#x} is not page-aligned"
            ),
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

/// The role of one guarded stack reachable before the full-root load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryStackKind {
    /// The CPU-local stack used for ordinary privilege crossings.
    Transition,
    /// One interrupt-stack-table stack reachable during early entry.
    Ist,
}

/// One page-aligned entry stack with its adjacent lower guard derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryStackRange {
    kind: EntryStackKind,
    start: u64,
    len: u64,
    lower_guard: u64,
}

impl EntryStackRange {
    /// Admit one non-empty higher-half stack and derive its lower guard page.
    ///
    /// # Errors
    ///
    /// Refuses an unaligned or empty range, arithmetic overflow, or a range
    /// whose payload or adjacent lower guard leaves the canonical higher half.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::MisalignedStart(start) => {
                write!(
                    formatter,
                    "entry stack start {start:#x} is not page-aligned"
                )
            }
            Self::Empty => core::fmt::Write::write_str(formatter, "the entry stack is empty"),
            Self::MisalignedLength(len) => {
                write!(formatter, "entry stack length {len} is not page-aligned")
            }
            Self::EndOverflow => {
                core::fmt::Write::write_str(formatter, "entry stack end overflows")
            }
            Self::OutsideHigherHalf(address) => write!(
                formatter,
                "entry stack address {address:#x} is not in the higher half"
            ),
        }
    }
}

impl core::error::Error for EntryStackRangeError {}

/// One leaf observed while walking the stripped root's higher half.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedPage {
    virtual_address: u64,
    physical_address: u64,
    span: u64,
    flags: Flags,
}

/// Whether every supplied entry-stack guard is absent from both roots.
///
/// The two observations must enumerate every present leaf that could cover a
/// supplied guard. Invalid observations fail closed, including an absent leaf
/// or a range that is empty, unaligned, or overflowing.
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

/// Whether the stripped higher half contains exactly the expected enclave.
///
/// `observed` must contain every present leaf found in the stripped root's
/// higher half. The comparison is order-independent and rejects an empty
/// enclave, duplicate virtual pages, a missing or extra leaf, a large page,
/// different backing, or any permission outside the expected policy.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
mod tests {
    use super::*;

    const CODE: u64 = HIGHER_HALF_START + 0x1000;
    const DATA: u64 = HIGHER_HALF_START + 0x2000;
    const STACK: u64 = HIGHER_HALF_START + 0x3000;

    fn expected_enclave() -> [EnclavePage; 3] {
        [
            EnclavePage::new(CODE, 0x4000, EnclavePermission::Executable).unwrap(),
            EnclavePage::new(DATA, 0x5000, EnclavePermission::ReadOnly).unwrap(),
            EnclavePage::new(STACK, 0x6000, EnclavePermission::Writable).unwrap(),
        ]
    }

    fn observed_enclave() -> [ObservedPage; 3] {
        [
            ObservedPage::new(
                STACK,
                0x6000,
                PAGE_SIZE,
                Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE | Flags::DIRTY,
            ),
            ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::ACCESSED),
            ObservedPage::new(DATA, 0x5000, PAGE_SIZE, Flags::PRESENT | Flags::NO_EXECUTE),
        ]
    }

    #[test]
    fn a_root_pair_is_aligned_encodable_and_distinct() {
        let pair = RootPair::new(0x1000, 0x2000).unwrap();
        assert_eq!(pair.full(), 0x1000);
        assert_eq!(pair.stripped(), 0x2000);

        assert_eq!(
            RootPair::new(1, 0x2000),
            Err(RootPairError::MisalignedFull(1))
        );
        assert_eq!(
            RootPair::new(0x1000, 0x2001),
            Err(RootPairError::MisalignedStripped(0x2001))
        );
        assert_eq!(
            RootPair::new(MAX_PHYSICAL, 0x2000),
            Err(RootPairError::FullOutOfRange(MAX_PHYSICAL))
        );
        assert_eq!(
            RootPair::new(0x1000, MAX_PHYSICAL),
            Err(RootPairError::StrippedOutOfRange(MAX_PHYSICAL))
        );
        assert_eq!(
            RootPair::new(0x1000, 0x1000),
            Err(RootPairError::SameRoot(0x1000))
        );
    }

    #[test]
    fn paired_flushes_require_pcid_off_and_non_global_leaves() {
        assert_eq!(
            FlushPremise::new(PcidState::Enabled, &[Flags::PRESENT]),
            Err(FlushPremiseError::PcidEnabled)
        );
        assert_eq!(
            FlushPremise::new(PcidState::Disabled, &[]),
            Err(FlushPremiseError::EmptyDomain)
        );
        assert_eq!(
            FlushPremise::new(
                PcidState::Disabled,
                &[Flags::PRESENT | Flags::USER, Flags::PRESENT | Flags::GLOBAL,]
            ),
            Err(FlushPremiseError::GlobalLeaf(1))
        );
        let premise = FlushPremise::new(
            PcidState::Disabled,
            &[
                Flags::PRESENT | Flags::USER,
                Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
            ],
        )
        .unwrap();
        assert_eq!(premise.leaf_count, 2);
    }

    #[test]
    fn a_kpti_pair_shares_one_identical_user_subtree() {
        assert!(shared_user_link_matches(Entry::ABSENT, Entry::ABSENT));
        assert!(!shared_user_link_matches(
            Entry::from_bits(Flags::WRITABLE.bits()),
            Entry::from_bits(Flags::WRITABLE.bits())
        ));

        let full = Entry::table(0x3000, crate::TablePrivilege::User).unwrap();
        let accessed = Entry::from_bits(full.bits() | Flags::ACCESSED.bits());
        assert!(shared_user_link_matches(full, accessed));

        let other = Entry::table(0x4000, crate::TablePrivilege::User).unwrap();
        let supervisor = Entry::table(0x3000, crate::TablePrivilege::Supervisor).unwrap();
        let global = Entry::from_bits(full.bits() | Flags::GLOBAL.bits());
        assert!(!shared_user_link_matches(full, Entry::ABSENT));
        assert!(!shared_user_link_matches(full, other));
        assert!(!shared_user_link_matches(full, supervisor));
        assert!(!shared_user_link_matches(full, global));
    }

    #[test]
    fn enclave_descriptors_are_aligned_high_half_pages() {
        let page = EnclavePage::new(CODE, 0x4000, EnclavePermission::Executable).unwrap();
        assert_eq!(page.virtual_address(), CODE);
        assert_eq!(page.physical_address(), 0x4000);
        assert_eq!(page.permission(), EnclavePermission::Executable);

        assert_eq!(
            EnclavePage::new(CODE + 1, 0x4000, EnclavePermission::Executable),
            Err(EnclavePageError::MisalignedVirtual(CODE + 1))
        );
        assert_eq!(
            EnclavePage::new(0x1000, 0x4000, EnclavePermission::Executable),
            Err(EnclavePageError::OutsideHigherHalf(0x1000))
        );
        assert_eq!(
            EnclavePage::new(CODE, 0x4001, EnclavePermission::Executable),
            Err(EnclavePageError::MisalignedPhysical(0x4001))
        );
        assert_eq!(
            EnclavePage::new(CODE, MAX_PHYSICAL, EnclavePermission::Executable),
            Err(EnclavePageError::PhysicalOutOfRange(MAX_PHYSICAL))
        );
    }

    #[test]
    fn a_kpti_user_root_contains_only_the_entry_enclave() {
        let expected = expected_enclave();
        let observed = observed_enclave();
        assert!(enclave_matches(&expected, &observed));
        assert!(!enclave_matches(&[], &[]));
        assert!(!enclave_matches(&expected, &observed[..2]));

        let mut extra = [observed[0], observed[1], observed[2], observed[2]];
        extra[3] = ObservedPage::new(
            HIGHER_HALF_START + 0x4000,
            0x7000,
            PAGE_SIZE,
            Flags::PRESENT | Flags::NO_EXECUTE,
        );
        assert!(!enclave_matches(&expected, &extra));

        let duplicate_expected = [expected[0], expected[0], expected[2]];
        assert!(!enclave_matches(&duplicate_expected, &observed));

        let aliased_expected = [
            expected[0],
            EnclavePage::new(
                DATA,
                expected[0].physical_address,
                EnclavePermission::ReadOnly,
            )
            .unwrap(),
            expected[2],
        ];
        assert!(!enclave_matches(&aliased_expected, &observed));
    }

    #[test]
    fn a_kpti_pair_refusal_returns_every_frame() {
        let empty = PairConstructionLedger::new();
        assert!(empty.refusal_returned_all(0));
        assert!(!empty.refusal_returned_all(1));

        let one_root = empty.acquire_root().unwrap();
        assert!(one_root.refusal_returned_all(1));
        assert!(!one_root.refusal_returned_all(0));
        assert!(!one_root.refusal_returned_all(2));
        assert_eq!(one_root.publish(), None);

        let roots = one_root.acquire_root().unwrap();
        assert!(roots.refusal_returned_all(2));
        assert_eq!(roots.acquire_root(), None);
        let mut authority = roots.publish().unwrap();
        assert!(authority.refusal_returned_all(2));

        for table_frames in 1..=8 {
            authority = authority.acquire_table().unwrap();
            let expected = 2 + table_frames;
            assert!(authority.refusal_returned_all(expected));
            assert!(!authority.refusal_returned_all(expected - 1));
            assert!(!authority.refusal_returned_all(expected + 1));
        }
    }

    #[test]
    fn a_kpti_pair_cleanup_retains_complete_authority() {
        let roots = PairConstructionLedger::new()
            .acquire_root()
            .unwrap()
            .acquire_root()
            .unwrap();
        let mut authority = roots.publish().unwrap();
        for _ in 0..5 {
            authority = authority.acquire_table().unwrap();
        }

        assert_eq!(
            authority.settle(0),
            Ok(PairCleanupState::Retained(authority))
        );
        for partial in 1..7 {
            assert_eq!(
                authority.settle(partial),
                Err(PairCleanupError::Incomplete {
                    expected: 7,
                    returned: partial,
                })
            );
        }
        assert_eq!(authority.settle(7), Ok(PairCleanupState::Released));

        for remaining in (0..5).rev() {
            authority = authority.release_table().unwrap();
            assert_eq!(authority.table_frames(), remaining);
            assert_eq!(
                authority.settle(0),
                Ok(PairCleanupState::Retained(authority))
            );
        }
        assert_eq!(authority.release_table(), None);
        assert_eq!(authority.settle(2), Ok(PairCleanupState::Released));
    }

    #[test]
    fn enclave_backing_span_and_every_permission_bit_are_exact() {
        let expected = expected_enclave();
        let observed = observed_enclave();
        for (name, replacement) in [
            (
                "backing",
                ObservedPage::new(CODE, 0x7000, PAGE_SIZE, Flags::PRESENT),
            ),
            (
                "span",
                ObservedPage::new(CODE, 0x4000, 2 * PAGE_SIZE, Flags::PRESENT),
            ),
            (
                "user",
                ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::USER),
            ),
            (
                "global",
                ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::GLOBAL),
            ),
            (
                "absent",
                ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::NONE),
            ),
            (
                "cache",
                ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::NO_CACHE),
            ),
            (
                "write-through cache",
                ObservedPage::new(
                    CODE,
                    0x4000,
                    PAGE_SIZE,
                    Flags::PRESENT | Flags::WRITE_THROUGH,
                ),
            ),
            (
                "huge",
                ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::HUGE),
            ),
            (
                "owned",
                ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::OWNED),
            ),
            (
                "writable read-only",
                ObservedPage::new(
                    DATA,
                    0x5000,
                    PAGE_SIZE,
                    Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
                ),
            ),
            (
                "write-execute",
                ObservedPage::new(STACK, 0x6000, PAGE_SIZE, Flags::PRESENT | Flags::WRITABLE),
            ),
        ] {
            let mut changed = observed;
            let index = changed
                .iter()
                .position(|page| page.virtual_address == replacement.virtual_address)
                .unwrap();
            changed[index] = replacement;
            assert!(
                !enclave_matches(&expected, &changed),
                "accepted {name}: {replacement:?}"
            );
        }
    }

    #[test]
    fn transition_and_ist_stacks_have_adjacent_guards_absent_from_both_roots() {
        let transition =
            EntryStackRange::new(EntryStackKind::Transition, STACK, 2 * PAGE_SIZE).unwrap();
        let ist = EntryStackRange::new(EntryStackKind::Ist, HIGHER_HALF_START + 0x9000, PAGE_SIZE)
            .unwrap();
        assert_eq!(transition.kind(), EntryStackKind::Transition);
        assert_eq!(transition.start(), STACK);
        assert_eq!(transition.len(), 2 * PAGE_SIZE);
        assert!(!transition.is_empty());
        assert_eq!(transition.lower_guard(), STACK - PAGE_SIZE);

        let full = [ObservedPage::new(
            transition.start(),
            0x6000,
            PAGE_SIZE,
            Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
        )];
        let stripped = [ObservedPage::new(
            ist.start(),
            0x7000,
            PAGE_SIZE,
            Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
        )];
        assert!(stack_guards_are_absent(
            &[transition, ist],
            &full,
            &stripped
        ));

        for mapped_guard in [transition.lower_guard(), ist.lower_guard()] {
            let guard = [ObservedPage::new(
                mapped_guard,
                0x8000,
                PAGE_SIZE,
                Flags::PRESENT | Flags::NO_EXECUTE,
            )];
            assert!(!stack_guards_are_absent(
                &[transition, ist],
                &guard,
                &stripped
            ));
            assert!(!stack_guards_are_absent(&[transition, ist], &full, &guard));
        }
        assert!(!stack_guards_are_absent(&[], &full, &stripped));
        let invalid_observation = [ObservedPage::new(
            transition.start(),
            0x6001,
            PAGE_SIZE,
            Flags::PRESENT,
        )];
        assert!(!stack_guards_are_absent(
            &[transition, ist],
            &invalid_observation,
            &stripped
        ));
    }

    #[test]
    fn entry_stack_geometry_refuses_partial_or_unrepresentable_ranges() {
        assert_eq!(
            EntryStackRange::new(EntryStackKind::Transition, STACK + 1, PAGE_SIZE),
            Err(EntryStackRangeError::MisalignedStart(STACK + 1))
        );
        assert_eq!(
            EntryStackRange::new(EntryStackKind::Transition, STACK, 0),
            Err(EntryStackRangeError::Empty)
        );
        assert_eq!(
            EntryStackRange::new(EntryStackKind::Transition, STACK, PAGE_SIZE + 1),
            Err(EntryStackRangeError::MisalignedLength(PAGE_SIZE + 1))
        );
        assert_eq!(
            EntryStackRange::new(EntryStackKind::Transition, HIGHER_HALF_START, PAGE_SIZE),
            Err(EntryStackRangeError::OutsideHigherHalf(HIGHER_HALF_START))
        );
        assert_eq!(
            EntryStackRange::new(
                EntryStackKind::Ist,
                u64::MAX - (PAGE_SIZE - 1),
                2 * PAGE_SIZE,
            ),
            Err(EntryStackRangeError::EndOverflow)
        );
    }

    #[test]
    fn entry_transition_reaches_rust_only_in_the_required_order() {
        let ordered = [
            EntryEvent::LoadFullRoot,
            EntryEvent::TransferToKernelStack,
            EntryEvent::ScrubTransitionStack,
            EntryEvent::CallRust,
        ];
        let mut state = EntryTransition::new();
        for event in ordered {
            state = state.advance(event).unwrap();
        }
        assert!(state.is_complete());
        assert_eq!(state.advance(EntryEvent::CallRust), None);

        for forbidden in [
            EntryEvent::TransferToKernelStack,
            EntryEvent::ScrubTransitionStack,
            EntryEvent::CallRust,
        ] {
            assert_eq!(EntryTransition::new().advance(forbidden), None);
        }
    }

    #[test]
    fn exit_transition_validates_then_loads_stripped_and_executes_verw() {
        let ordered = [
            ExitEvent::ValidateUserState,
            ExitEvent::ScrubTransitionStack,
            ExitEvent::CopyUserState,
            ExitEvent::LastSecretAccess,
            ExitEvent::LoadStrippedRoot,
            ExitEvent::Verw,
            ExitEvent::ReturnToUser,
        ];
        let mut state = ExitTransition::new();
        for event in ordered {
            state = state.advance(event).unwrap();
        }
        assert!(state.is_complete());
        assert_eq!(state.advance(ExitEvent::ReturnToUser), None);

        for forbidden in [
            ExitEvent::ScrubTransitionStack,
            ExitEvent::CopyUserState,
            ExitEvent::LastSecretAccess,
            ExitEvent::LoadStrippedRoot,
            ExitEvent::Verw,
            ExitEvent::ReturnToUser,
        ] {
            assert_eq!(ExitTransition::new().advance(forbidden), None);
        }

        let mut before_verw = ExitTransition::new();
        for event in ordered[..5].iter().copied() {
            before_verw = before_verw.advance(event).unwrap();
        }
        assert_eq!(before_verw.advance(ExitEvent::ReturnToUser), None);
        assert!(!before_verw.is_complete());
    }
}
