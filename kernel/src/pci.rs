use core::fmt;

use pci_config::{
    Address, Bar, BarEnableError, Class, CommandState, EnableError, FunctionObservation,
    HeaderType, PciTopology, ProbeError, ProbeRetry, ProbedBars, QualifiedNonRequester,
    QuiescedIsaRequester, QuiescedRequester, TopologyError, Window, header,
};
use wost::arch::current::vtd::{DmaDomain, InterruptIsolation};
use wost::arch::current::{paging, pci};
use wost::sync::{Leaf, SpinLock};

pub(crate) const FUNCTION_AUTHORITY_LIMIT: usize = pci_config::FUNCTION_AUTHORITY_LIMIT;

const MAX_BUSES: usize = 256;

// Static, so a topology proof never consumes the candidate's fixed boot stack.
static TOPOLOGY: SpinLock<Leaf, PciTopology> = SpinLock::new(PciTopology::new());

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Function {
    pub(crate) at: Address,
    identity: u32,
    pub(crate) class: Class,
    class_bits: u32,
    command: u16,
    pub(crate) header: HeaderType,
}

// Each bus is queued once, so a bridge naming a queued bus is ignored, not followed.
pub(crate) fn enumerate(mut found: impl FnMut(&Function)) -> Option<Survey> {
    if !pci::is_available() {
        return None;
    }

    let mut worklist = Worklist::new();
    worklist.push(0);
    let mut survey = Survey::default();

    while let Some(bus) = worklist.pop() {
        survey.buses += 1;
        survey.highest_bus = survey.highest_bus.max(bus);

        for device in 0..pci_config::DEVICES {
            for function in 0..pci_config::FUNCTIONS {
                let Some(entry) = read_function(bus, device, function) else {
                    // Function zero absent means the device is; the specification never skips it.
                    if function == 0 {
                        break;
                    }
                    continue;
                };

                found(&entry);
                survey.functions += 1;
                if entry.class.is_bridge() {
                    survey.bridges += 1;
                }

                // A bridge needs its class and header type: one says what it is, one its layout.
                if entry.class.is_bridge()
                    && entry.header.is_bridge()
                    && let Some(behind) = secondary_of(&entry)
                {
                    worklist.push(behind);
                }

                // A single-function device may answer every function with its own header.
                if function == 0 && !entry.header.is_multifunction() {
                    break;
                }
            }
        }
    }

    Some(survey)
}

#[derive(Debug)]
struct Worklist {
    claimed: [bool; MAX_BUSES],
    pending: [u8; MAX_BUSES],
    queued: usize,
    next: usize,
}

impl Worklist {
    const fn new() -> Self {
        Self {
            claimed: [false; MAX_BUSES],
            pending: [0; MAX_BUSES],
            queued: 0,
            next: 0,
        }
    }

    fn push(&mut self, bus: u8) {
        if self.claimed[usize::from(bus)] || self.queued == MAX_BUSES {
            return;
        }
        self.claimed[usize::from(bus)] = true;
        self.pending[self.queued] = bus;
        self.queued += 1;
    }

    fn pop(&mut self) -> Option<u8> {
        let bus = *self.pending[..self.queued].get(self.next)?;
        self.next += 1;
        Some(bus)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct Survey {
    pub(crate) functions: usize,
    pub(crate) buses: usize,
    pub(crate) bridges: usize,
    pub(crate) highest_bus: u8,
}

fn read_function(bus: u8, device: u8, function: u8) -> Option<Function> {
    let at = Address::new(bus, device, function, header::VENDOR)?;
    let identity = pci::read(at)?;
    if pci_config::is_absent(identity as u16) {
        return None;
    }

    let class_bits = pci::read(at.at(header::CLASS)?)?;
    let class = Class::from_bits(class_bits);
    let command = pci::read(at.at(header::COMMAND)?)? as u16;
    let header = HeaderType::from_bits((pci::read(at.at(header::HEADER_TYPE)?)? >> 16) as u8);

    Some(Function {
        at,
        identity,
        class,
        class_bits,
        command,
        header,
    })
}

// Probes buses a malformed bridge may have made unreachable, so a hidden requester is still seen.
pub(crate) fn snapshot_topology(
    mut non_requester: Option<QualifiedNonRequester>,
    mut quiesced: Option<QuiescedIsaRequester>,
) -> Result<(), TopologyError> {
    TOPOLOGY.lock_leaf().reset();
    for bus in 0..=u8::MAX {
        for device in 0..pci_config::DEVICES {
            for function in 0..pci_config::FUNCTIONS {
                let Some(entry) = read_function(bus, device, function) else {
                    continue;
                };
                let bus_numbers = if entry.class.is_bridge() && entry.header.is_bridge() {
                    entry.at.at(header::BUS_NUMBERS).and_then(pci::read)
                } else {
                    None
                };
                let observation = FunctionObservation {
                    at: entry.at,
                    identity: entry.identity,
                    class_bits: entry.class_bits,
                    command: entry.command,
                    class: entry.class,
                    header: entry.header,
                    bus_numbers,
                };
                let mut topology = TOPOLOGY.lock_leaf();
                if let Some(authority) =
                    non_requester.take_if(|authority| authority.address() == entry.at)
                {
                    topology.observe_non_requester(authority, observation)?;
                } else if let Some(authority) =
                    quiesced.take_if(|authority| authority.address() == entry.at)
                {
                    topology.observe_quiesced_requester(authority, observation)?;
                } else {
                    topology.observe(entry.at, entry.class, entry.header, bus_numbers)?;
                }
                if function == 0 && !entry.header.is_multifunction() {
                    break;
                }
            }
        }
    }
    if let Some(authority) = non_requester {
        return Err(TopologyError::QualifiedNonRequesterMissing {
            at: authority.address(),
        });
    }
    if let Some(authority) = quiesced {
        return Err(TopologyError::QuiescedRequesterMissing {
            at: authority.address(),
        });
    }
    TOPOLOGY.lock_leaf().seal()
}

pub(crate) fn topology_is_sealed() -> bool {
    TOPOLOGY.lock_leaf().is_sealed()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TopologyPresence {
    Unsealed,
    Empty,
    Present,
}

pub(crate) fn topology_bridge_range(at: Address) -> Option<pci_config::BridgeRange> {
    TOPOLOGY.lock_leaf().bridge_range(at)
}

pub(crate) fn topology_contains_function(at: Address) -> bool {
    TOPOLOGY.lock_leaf().contains_function(at)
}

pub(crate) fn topology_contains_requester(at: Address) -> bool {
    topology_contains_function(at) && TOPOLOGY.lock_leaf().contains_requester(at)
}

pub(crate) fn first_topology_function() -> Option<Address> {
    if !topology_is_sealed() {
        return None;
    }
    for bus in 0..=u8::MAX {
        for device in 0..pci_config::DEVICES {
            for function in 0..pci_config::FUNCTIONS {
                let at = Address::new(bus, device, function, 0)?;
                if topology_contains_requester(at) {
                    return Some(at);
                }
            }
        }
    }
    None
}

pub(crate) fn topology_presence() -> TopologyPresence {
    if !topology_is_sealed() {
        return TopologyPresence::Unsealed;
    }
    if first_topology_function().is_some() {
        TopologyPresence::Present
    } else {
        TopologyPresence::Empty
    }
}

pub(crate) fn topology_hierarchy_contains(root: Address, target: Address) -> bool {
    TOPOLOGY.lock_leaf().hierarchy_contains(root, target)
}

fn secondary_of(entry: &Function) -> Option<u8> {
    let at = entry.at.at(header::BUS_NUMBERS)?;
    Some(pci_config::secondary_bus(pci::read(at)?))
}

static RETAINED_COMMAND: SpinLock<Leaf, Option<Device>> = SpinLock::new(None);

static RETAINED_PROBE: SpinLock<Leaf, Option<ProbeRecovery>> = SpinLock::new(None);

static RETAINED_RELEASES: SpinLock<Leaf, [Option<Device>; FUNCTION_AUTHORITY_LIMIT]> =
    SpinLock::new([const { None }; FUNCTION_AUTHORITY_LIMIT]);

static RETAINED_FUNCTIONS: SpinLock<Leaf, [Option<pci::FunctionClaim>; FUNCTION_AUTHORITY_LIMIT]> =
    SpinLock::new([const { None }; FUNCTION_AUTHORITY_LIMIT]);

#[derive(Debug)]
pub(crate) enum ClaimError {
    NoBus,
    Absent,
    Taken,
    NoRoom,
    InvalidWindow,
    ProbeRestore(ProbeRecovery),
    FunctionRetained,
}

#[derive(Debug)]
pub(crate) struct DmaFence {
    function: pci::FunctionClaim,
    command_state: Option<CommandState>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DmaFenceError {
    Claim(pci::ClaimError),
    Command(EnableError),
    Quiesce,
    Function,
}

impl fmt::Display for DmaFenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claim(reason) => write!(formatter, "function claim: {reason}"),
            Self::Command(reason) => write!(formatter, "command fence: {reason}"),
            Self::Quiesce => formatter.write_str(
                "the entry-quiesced ISA requester changed or its 8237 controllers are not quiet",
            ),
            Self::Function => formatter.write_str("function authority could not be returned"),
        }
    }
}

impl core::error::Error for DmaFenceError {}

#[derive(Debug)]
pub(crate) enum DmaFenceOutcome {
    Fenced(DmaFence),
    Refused(DmaFenceError),
    Retained {
        reason: DmaFenceError,
        fence: DmaFence,
    },
}

#[derive(Debug)]
pub(crate) struct DmaFenceRelease {
    pub(crate) reason: DmaFenceError,
    pub(crate) fence: DmaFence,
}

impl DmaFence {
    pub(crate) const fn address(&self) -> Address {
        self.function.address()
    }

    pub(crate) fn admit_bridge_bus_master(
        &mut self,
        domain: &DmaDomain<'_>,
        interrupts: &InterruptIsolation<'_>,
    ) -> Result<(), EnableError> {
        let Some(state) = self.command_state.as_mut() else {
            return Err(EnableError::Stale);
        };
        pci::admit_bridge_bus_master(&self.function, domain, interrupts, state)
    }

    pub(crate) fn release(mut self) -> Result<(), DmaFenceRelease> {
        if let Some(state) = self.command_state.take()
            && let Err(failure) = pci::restore_command(&self.function, state)
        {
            let reason = DmaFenceError::Command(failure.reason());
            self.command_state = Some(failure.into_state());
            return Err(DmaFenceRelease {
                reason,
                fence: self,
            });
        }
        pci::release_function(self.function).map_err(|failure| DmaFenceRelease {
            reason: DmaFenceError::Function,
            fence: Self {
                function: failure.into_function(),
                command_state: None,
            },
        })
    }
}

// The entry-quiesced ISA requester cannot clear BME; its claim and 8237 mask hold it.
pub(crate) fn fence_dma_requester(at: Address) -> DmaFenceOutcome {
    let function = match pci::claim_function(at) {
        Ok(function) => function,
        Err(reason) => return DmaFenceOutcome::Refused(DmaFenceError::Claim(reason)),
    };
    let quiesced = TOPOLOGY.lock_leaf().quiesced_requester(at);
    if let Some(record) = quiesced {
        return fence_quiesced_requester(function, record);
    }
    match pci::fence_bus_master(&function) {
        Ok(command_state) => DmaFenceOutcome::Fenced(DmaFence {
            function,
            command_state: Some(command_state),
        }),
        Err(failure) => {
            let reason = DmaFenceError::Command(failure.reason());
            if let Some(command_state) = failure.into_state() {
                return DmaFenceOutcome::Retained {
                    reason,
                    fence: DmaFence {
                        function,
                        command_state: Some(command_state),
                    },
                };
            }
            match pci::release_function(function) {
                Ok(()) => DmaFenceOutcome::Refused(reason),
                Err(failure) => DmaFenceOutcome::Retained {
                    reason: DmaFenceError::Function,
                    fence: DmaFence {
                        function: failure.into_function(),
                        command_state: None,
                    },
                },
            }
        }
    }
}

fn fence_quiesced_requester(
    function: pci::FunctionClaim,
    record: QuiescedRequester,
) -> DmaFenceOutcome {
    let at = record.address();
    let identity = at.at(header::VENDOR).and_then(pci::read);
    let command = at
        .at(header::COMMAND)
        .and_then(pci::read)
        .map(|bits| bits as u16);
    if identity == Some(record.identity())
        && command == Some(record.command())
        && pci::isa_dma_quiet()
    {
        return DmaFenceOutcome::Fenced(DmaFence {
            function,
            command_state: None,
        });
    }
    match pci::release_function(function) {
        Ok(()) => DmaFenceOutcome::Refused(DmaFenceError::Quiesce),
        Err(failure) => DmaFenceOutcome::Retained {
            reason: DmaFenceError::Function,
            fence: DmaFence {
                function: failure.into_function(),
                command_state: None,
            },
        },
    }
}

#[derive(Debug)]
pub(crate) struct ProbeRecovery {
    function: pci::FunctionClaim,
    retry: ProbeRetry,
}

#[derive(Debug)]
pub(crate) struct ProbeRecoveryFailure {
    reason: ProbeError<()>,
    recovery: ProbeRecovery,
}

#[derive(Debug)]
#[must_use = "probe recovery may retain exclusive function authority"]
pub(crate) enum ProbeRecoveryOutcome {
    Recovered,
    Refused(ProbeRecoveryFailure),
    FunctionRetained,
}

impl ProbeRecoveryFailure {
    pub(crate) const fn reason(&self) -> &ProbeError<()> {
        &self.reason
    }

    pub(crate) fn into_recovery(self) -> ProbeRecovery {
        self.recovery
    }
}

impl ProbeRecovery {
    pub(crate) fn retry(self) -> ProbeRecoveryOutcome {
        let Self { function, retry } = self;
        if let Err(failure) = pci::retry_probe(&function, retry) {
            let reason = *failure.reason();
            let Some(retry) = failure.into_retry() else {
                retain_failed_function(function);
                return ProbeRecoveryOutcome::FunctionRetained;
            };
            return ProbeRecoveryOutcome::Refused(ProbeRecoveryFailure {
                reason,
                recovery: Self { function, retry },
            });
        }
        match pci::release_function(function) {
            Ok(()) => ProbeRecoveryOutcome::Recovered,
            Err(failure) => {
                retain_failed_function(failure.into_function());
                ProbeRecoveryOutcome::FunctionRetained
            }
        }
    }
}

impl fmt::Display for ClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBus => formatter.write_str("configuration space is not reachable"),
            Self::Absent => formatter.write_str("nothing is at that address"),
            Self::Taken => formatter.write_str("another driver holds it"),
            Self::NoRoom => formatter.write_str("no room in the device registry"),
            Self::InvalidWindow => formatter.write_str("a base address window is malformed"),
            Self::ProbeRestore(_) => {
                formatter.write_str("a base address probe could not be restored")
            }
            Self::FunctionRetained => {
                formatter.write_str("the restored function claim could not be returned")
            }
        }
    }
}

impl core::error::Error for ClaimError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BarAdmissionError {
    Missing,
    NotMemory,
    Range(paging::MapError),
    Bind(pci::BarBindError),
}

impl fmt::Display for BarAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("the BAR slot is empty"),
            Self::NotMemory => formatter.write_str("the BAR is not memory"),
            Self::Range(reason) => write!(formatter, "physical range: {reason}"),
            Self::Bind(reason) => write!(formatter, "function binding: {reason}"),
        }
    }
}

impl core::error::Error for BarAdmissionError {}

#[derive(Debug)]
pub(crate) struct BarDecodeFailure {
    reason: BarEnableError,
    bar: pci::DisabledBar,
}

#[derive(Debug)]
#[must_use = "decode refusal retains an inaccessible mapping"]
pub(crate) enum BarDecodeOutcome {
    Enabled(pci::MappedBar),
    Refused(BarDecodeFailure),
}

impl BarDecodeFailure {
    pub(crate) const fn reason(&self) -> BarEnableError {
        self.reason
    }

    pub(crate) fn into_bar(self) -> pci::DisabledBar {
        self.bar
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReleaseError {
    Command,
    Function,
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command => formatter.write_str("the pci command word could not be restored"),
            Self::Function => formatter.write_str("the function claim could not be returned"),
        }
    }
}

impl core::error::Error for ReleaseError {}

#[must_use = "a retained PCI lease must be retried or the kernel must stop"]
#[derive(Debug)]
pub(crate) struct ReleaseOutcome {
    retained: Option<(ReleaseError, Device)>,
}

impl ReleaseOutcome {
    const fn released() -> Self {
        Self { retained: None }
    }

    fn refused(reason: ReleaseError, device: Device) -> Self {
        Self {
            retained: Some((reason, device)),
        }
    }

    pub(crate) fn into_retained(self) -> Option<(ReleaseError, Device)> {
        self.retained
    }
}

#[derive(Debug)]
pub(crate) struct Device {
    function: pci::FunctionClaim,
    bars: ProbedBars,
    command_state: Option<CommandState>,
    // A failed restoration blocks release until a later restoration or fence proof succeeds.
    uncertain: bool,
}

impl Device {
    pub(crate) const fn at(&self) -> Address {
        self.function.address()
    }

    pub(crate) const fn identity(&self) -> (u16, u16) {
        self.function.identity()
    }

    pub(crate) fn window(&self, index: usize) -> Option<Window> {
        self.bars.window(index)
    }

    pub(crate) fn admit_memory_bar(
        &self,
        index: usize,
    ) -> Result<pci::AdmittedBar, BarAdmissionError> {
        let Some(window) = self.window(index) else {
            return Err(BarAdmissionError::Missing);
        };
        let Bar::Memory { address, .. } = window.bar else {
            return Err(BarAdmissionError::NotMemory);
        };
        let range =
            paging::admit_device_range(address, window.size).map_err(BarAdmissionError::Range)?;
        pci::bind_memory_bar(&self.function, &self.bars, index, range)
            .map_err(BarAdmissionError::Bind)
    }

    pub(crate) fn admit_bus_master(
        &mut self,
        authority: pci::NvmeBusMasterAuthority<'_>,
        domain: DmaDomain<'_>,
        interrupts: InterruptIsolation<'_>,
    ) -> Result<(), EnableError> {
        let Some(state) = self.command_state.as_mut() else {
            return Err(EnableError::Stale);
        };
        let result = pci::admit_bus_master(authority, domain, interrupts, state);
        if let Err(error) = result {
            self.uncertain |= matches!(error, EnableError::Restore);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn disable_bus_master_for_retirement(&mut self) -> Result<(), EnableError> {
        let Some(state) = self.command_state.as_mut() else {
            self.uncertain = true;
            return Err(EnableError::Stale);
        };
        match pci::disable_bus_master_for_retirement(&self.function, state) {
            Ok(()) => {
                self.uncertain = false;
                Ok(())
            }
            Err(reason) => {
                self.uncertain = true;
                Err(reason)
            }
        }
    }

    // Keep the device until the VT-d transition ends: it excludes every other writer.
    pub(crate) fn settle_bus_master_fence_for_retirement(&mut self) -> Result<(), EnableError> {
        let Some(state) = self.command_state.take() else {
            return if self.uncertain {
                Err(EnableError::Stale)
            } else {
                Ok(())
            };
        };
        if let Err(failure) = pci::restore_command(&self.function, state) {
            let reason = failure.reason();
            self.command_state = Some(failure.into_state());
            self.uncertain = true;
            return Err(reason);
        }
        self.uncertain = false;
        Ok(())
    }

    pub(crate) fn verify_bus_master_fence(&self) -> Result<(), EnableError> {
        let Some(state) = self.command_state.as_ref() else {
            return Err(EnableError::Stale);
        };
        pci::verify_bus_master_fence(&self.function, state)
    }

    pub(crate) fn enable_memory_bar(&mut self, bar: pci::DisabledBar) -> BarDecodeOutcome {
        let Some(state) = self.command_state.as_mut() else {
            return BarDecodeOutcome::Refused(BarDecodeFailure {
                reason: BarEnableError::Command(EnableError::Stale),
                bar,
            });
        };
        match pci::enable_memory_bar(&self.function, bar, state) {
            pci::BarEnableOutcome::Enabled(bar) => BarDecodeOutcome::Enabled(bar),
            pci::BarEnableOutcome::Refused(failure) => {
                let reason = failure.reason();
                let bar = failure.into_bar();
                self.uncertain |= matches!(reason, BarEnableError::Command(EnableError::Restore));
                BarDecodeOutcome::Refused(BarDecodeFailure { reason, bar })
            }
        }
    }

    pub(crate) fn commit(&mut self) -> Result<(), EnableError> {
        if let Some(state) = self.command_state.take()
            && let Err(failure) = pci::commit_command(&self.function, state)
        {
            let reason = failure.reason();
            self.command_state = Some(failure.into_state());
            self.uncertain = true;
            return Err(reason);
        }
        self.uncertain = false;
        Ok(())
    }

    // Selection may touch the next candidate only after this returns released.
    pub(crate) fn release(mut self) -> ReleaseOutcome {
        if let Some(state) = self.command_state.take() {
            if let Err(failure) = pci::restore_command(&self.function, state) {
                self.command_state = Some(failure.into_state());
                return ReleaseOutcome::refused(ReleaseError::Command, self);
            }
            self.uncertain = false;
        }
        if self.uncertain {
            return ReleaseOutcome::refused(ReleaseError::Command, self);
        }
        let Self { function, bars, .. } = self;
        match pci::release_function(function) {
            Ok(()) => ReleaseOutcome::released(),
            Err(failure) => ReleaseOutcome::refused(
                ReleaseError::Function,
                Self {
                    function: failure.into_function(),
                    bars,
                    command_state: None,
                    uncertain: false,
                },
            ),
        }
    }
}

pub(crate) fn retain_failed_commit(device: Device) {
    let mut retained = RETAINED_COMMAND.lock_leaf();
    assert!(retained.is_none(), "two PCI commit failures were retained");
    *retained = Some(device);
}

pub(crate) fn retain_failed_probe(recovery: ProbeRecovery) {
    let mut retained = RETAINED_PROBE.lock_leaf();
    assert!(retained.is_none(), "two PCI probe failures were retained");
    *retained = Some(recovery);
}

pub(crate) fn retain_failed_release(device: Device) {
    let mut retained = RETAINED_RELEASES.lock_leaf();
    let Some(slot) = retained.iter_mut().find(|slot| slot.is_none()) else {
        panic!("retained device leases exceed FUNCTION_AUTHORITY_LIMIT");
    };
    *slot = Some(device);
}

fn retain_failed_function(function: pci::FunctionClaim) {
    let mut retained = RETAINED_FUNCTIONS.lock_leaf();
    let Some(slot) = retained.iter_mut().find(|slot| slot.is_none()) else {
        panic!("retained function claims exceed FUNCTION_AUTHORITY_LIMIT");
    };
    *slot = Some(function);
}

static RETAINED_BARS: SpinLock<
    Leaf,
    [Option<BarQuarantine>; FUNCTION_AUTHORITY_LIMIT * pci_config::BARS],
> = SpinLock::new([const { None }; FUNCTION_AUTHORITY_LIMIT * pci_config::BARS]);

#[derive(Debug)]
pub(crate) enum BarQuarantine {
    Mapping {
        _mapping: pci::BarMapping,
    },
    Releasable {
        _bar: pci::ReleasableBar,
    },
    Authority {
        _failure: pci_config::BarAuthorityFailure,
    },
}

pub(crate) fn retain_bar(quarantine: BarQuarantine) {
    let mut retained = RETAINED_BARS.lock_leaf();
    let Some(slot) = retained.iter_mut().find(|slot| slot.is_none()) else {
        panic!("retained BAR authority exceeds every BAR of the PCI function registry");
    };
    *slot = Some(quarantine);
}

pub(crate) fn retain_bar_mapping(mapping: pci::BarMapping) {
    retain_bar(BarQuarantine::Mapping { _mapping: mapping });
}

pub(crate) fn retain_bar_authority(failure: pci_config::BarAuthorityFailure) {
    retain_bar(BarQuarantine::Authority { _failure: failure });
}

pub(crate) fn lease(at: Address) -> Result<Device, ClaimError> {
    let function = pci::claim_function(at).map_err(|reason| match reason {
        pci::ClaimError::Unavailable => ClaimError::NoBus,
        pci::ClaimError::Absent => ClaimError::Absent,
        pci::ClaimError::Taken => ClaimError::Taken,
        pci::ClaimError::NoRoom | pci::ClaimError::Exhausted => ClaimError::NoRoom,
        pci::ClaimError::Header(_) => ClaimError::InvalidWindow,
    })?;
    let staged = match pci::probe_bars(&function) {
        Ok(staged) => staged,
        Err(failure) => {
            let invalid = matches!(
                failure.reason(),
                ProbeError::Malformed | ProbeError::InvalidWindow
            );
            if let Some(retry) = failure.into_retry() {
                return Err(ClaimError::ProbeRestore(ProbeRecovery { function, retry }));
            }
            let reason = if invalid {
                ClaimError::InvalidWindow
            } else {
                ClaimError::NoBus
            };
            return Err(release_clean_function(function, reason));
        }
    };
    let (bars, command_state) = staged.into_parts();
    Ok(Device {
        function,
        bars,
        command_state: Some(command_state),
        uncertain: false,
    })
}

fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    if let Err(failure) = pci::release_function(function) {
        retain_failed_function(failure.into_function());
        return ClaimError::FunctionRetained;
    }
    reason
}
