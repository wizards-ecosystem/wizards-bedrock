//! What is on the bus, and who is allowed to touch it.
//!
//! D-77 left half an authority debt open: a port claim proves *exclusivity* —
//! nobody else drives these ports — and says nothing about *entitlement*, about
//! whether the claimant is the driver for the thing behind them. The port
//! registry cannot answer that, because a port range is not a device.
//!
//! This is the other half. A driver claims a **function** — a bus, device and
//! function triple — and the claim carries the windows that function decodes
//! with it. Two drivers cannot claim one function, and a bare port claim
//! inside a claimed device's I/O window is refused because the device already
//! has it (D-168).
//!
//! Fixed driver claims are permanent, exactly as port claims are and for the
//! same reason (D-138). Storage selection additionally has a transient lease:
//! it can advance only after restoring the command word and returning every
//! dynamic I/O span and registry slot as one retirement (D-199).
//!
//! **Enumeration recurses.** A scan of bus zero is not enough — VMware places
//! functions behind `pciBridge0`–`pciBridge7`, and QEMU's q35 puts a
//! DMI-to-PCI bridge at `00:1e.0` — so a controller can sit on a bus that
//! nothing on bus zero mentions except a bridge (`012` §2). The walk is
//! iterative over a worklist rather than recursive, because kernel stacks are
//! thirty-two kilobytes and a malicious or broken bridge topology is a cycle.

use pci_config::{
    Address, Bar, BarEnableError, Class, CommandState, EnableError, FunctionObservation,
    HeaderType, PciTopology, ProbeError, ProbeRetry, ProbedBars, QualifiedNonRequester,
    QuiescedIsaRequester, QuiescedRequester, TopologyError, Window, header,
};
use wost::arch::current::vtd::{DmaDomain, InterruptIsolation};
use wost::arch::current::{paging, pci};
use wost::sync::SpinLock;

#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::thread::ThreadContext;

/// Base address registers a type 0 header has.
pub(crate) const BARS: usize = pci_config::BARS;

/// Function authorities the platform PCI registry can hold concurrently.
pub(crate) const FUNCTION_AUTHORITY_LIMIT: usize = pci_config::FUNCTION_AUTHORITY_LIMIT;

/// Buses the walk will visit before giving up.
///
/// There are 256 and a correct topology visits each at most once. The bound is
/// here because a bridge that reports itself as its own secondary bus is a
/// cycle, and a kernel that trusts the machine's numbering hangs on it.
const MAX_BUSES: usize = 256;

/// Complete segment-zero function and bridge inventory used by VT-d lookup.
///
/// The roughly ten-kibibyte snapshot stays in static storage so proving a
/// hostile topology never consumes the candidate's fixed boot stack.
static TOPOLOGY: SpinLock<PciTopology> = SpinLock::new(PciTopology::new());

/// One function found on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Function {
    /// Where it lives.
    pub(crate) at: Address,
    /// Who made it.
    pub(crate) vendor: u16,
    /// Which of theirs it is.
    pub(crate) device: u16,
    /// Complete vendor/device dword used by the handoff policy.
    identity: u32,
    /// What it says it is.
    pub(crate) class: Class,
    /// Complete class dword, including silicon revision.
    class_bits: u32,
    /// Command word observed after the entry-time fence.
    command: u16,
    /// Whether the rest of its header is a bridge's, and whether it has
    /// siblings.
    pub(crate) header: HeaderType,
}

/// Walk every bus reachable from bus zero, reporting each function found.
///
/// Iterative over a worklist of buses, with each bus visited at most once: a
/// bridge that names a bus already seen is ignored rather than followed, which
/// is what keeps a broken topology from being a hang.
///
/// Returns what the walk covered, or `None` if configuration space cannot be
/// reached at all.
pub(crate) fn enumerate(mut found: impl FnMut(&Function)) -> Option<Survey> {
    if !pci::is_available() {
        return None;
    }

    let mut seen = [false; MAX_BUSES];
    let mut pending = [0u8; MAX_BUSES];
    let mut queued = 1;
    let mut next = 0;
    let mut survey = Survey::default();

    while next < queued {
        let bus = pending[next];
        next += 1;
        if seen[bus as usize] {
            continue;
        }
        seen[bus as usize] = true;
        survey.buses += 1;
        survey.highest_bus = survey.highest_bus.max(bus);

        for device in 0..pci_config::DEVICES {
            for function in 0..pci_config::FUNCTIONS {
                let Some(entry) = read_function(bus, device, function) else {
                    // Function zero absent means the whole device is; the
                    // specification allows gaps but never a missing first function.
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

                // Both tests: the class says what the function is for and the
                // header type says what its registers mean.
                if entry.class.is_bridge()
                    && entry.header.is_bridge()
                    && let Some(behind) = secondary_of(&entry)
                    && !seen[behind as usize]
                    && queued < MAX_BUSES
                {
                    pending[queued] = behind;
                    queued += 1;
                }

                // A single-function device answers every function with its own
                // header; bit 7 of the header type says how many there are.
                if function == 0 && !entry.header.is_multifunction() {
                    break;
                }
            }
        }
    }

    Some(survey)
}

/// What one walk of the bus covered.
///
/// Buses rather than only functions, and that is the point: a kernel that
/// scans bus zero and stops finds every function q35 has and reports a
/// perfectly plausible number. Only the *bus* count says whether it went
/// through the bridge (D-168).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Survey {
    /// Functions reported.
    pub(crate) functions: usize,
    /// Buses actually visited.
    pub(crate) buses: usize,
    /// Bridges seen, whether or not they led anywhere new.
    pub(crate) bridges: usize,
    /// The highest bus number visited.
    pub(crate) highest_bus: u8,
}

/// Read one function's identity, or `None` if nothing is there.
fn read_function(bus: u8, device: u8, function: u8) -> Option<Function> {
    let at = Address::new(bus, device, function, header::VENDOR)?;
    let identity = pci::read(at)?;
    let vendor = identity as u16;
    if pci_config::is_absent(vendor) {
        return None;
    }

    let class_bits = pci::read(at.at(header::CLASS)?)?;
    let class = Class::from_bits(class_bits);
    let command = pci::read(at.at(header::COMMAND)?)? as u16;
    let header = HeaderType::from_bits((pci::read(at.at(header::HEADER_TYPE)?)? >> 16) as u8);

    Some(Function {
        at,
        vendor,
        device: (identity >> 16) as u16,
        identity,
        class,
        class_bits,
        command,
        header,
    })
}

/// Probe every function number and seal one rooted, immutable PCI topology.
///
/// Unlike ordinary enumeration, this proof never trusts multifunction hints
/// and probes buses which a malformed bridge may have made unreachable.
///
/// # Errors
///
/// Returns [`TopologyError`] when bridge identity or routing cannot form one
/// unambiguous tree rooted at bus zero.
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
                let mut topology = TOPOLOGY.lock_leaf();
                if let Some(authority) =
                    non_requester.take_if(|authority| authority.address() == entry.at)
                {
                    topology.observe_non_requester(
                        authority,
                        FunctionObservation::new(
                            entry.at,
                            entry.identity,
                            entry.class_bits,
                            entry.command,
                            entry.class,
                            entry.header,
                            bus_numbers,
                        ),
                    )?;
                } else if let Some(authority) =
                    quiesced.take_if(|authority| authority.address() == entry.at)
                {
                    topology.observe_quiesced_requester(
                        authority,
                        FunctionObservation::new(
                            entry.at,
                            entry.identity,
                            entry.class_bits,
                            entry.command,
                            entry.class,
                            entry.header,
                            bus_numbers,
                        ),
                    )?;
                } else {
                    topology.observe(entry.at, entry.class, entry.header, bus_numbers)?;
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

/// Whether the complete segment-zero topology passed its rooted-tree proof.
pub(crate) fn topology_is_sealed() -> bool {
    TOPOLOGY.lock_leaf().is_sealed()
}

/// Whether the PCI requester inventory is unavailable, empty, or populated.
///
/// Security decisions must distinguish an empty sealed inventory from a probe
/// that observed functions but failed before it could seal their topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopologyPresence {
    Unsealed,
    Empty,
    Present,
}

/// The sealed bus range behind one exact bridge function.
pub(crate) fn topology_bridge_range(at: Address) -> Option<pci_config::BridgeRange> {
    TOPOLOGY.lock_leaf().bridge_range(at)
}

/// Whether one exact function answered the sealed independent probe.
pub(crate) fn topology_contains_function(at: Address) -> bool {
    TOPOLOGY.lock_leaf().contains_function(at)
}

/// Whether one exact function is a DMA requester in the sealed snapshot.
pub(crate) fn topology_contains_requester(at: Address) -> bool {
    topology_contains_function(at) && TOPOLOGY.lock_leaf().contains_requester(at)
}

/// The first requester in the sealed snapshot, for an interrupt-only boundary.
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

/// Classify the complete segment-zero requester inventory.
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

/// Whether `target` lies below `root` in the sealed parent chain.
pub(crate) fn topology_hierarchy_contains(root: Address, target: Address) -> bool {
    TOPOLOGY.lock_leaf().hierarchy_contains(root, target)
}

/// The bus behind a bridge.
fn secondary_of(entry: &Function) -> Option<u8> {
    let at = entry.at.at(header::BUS_NUMBERS)?;
    Some(pci_config::secondary_bus(pci::read(at)?))
}

/// A committed controller whose command word no longer matched its lease.
static RETAINED_COMMAND: SpinLock<Option<Device>> = SpinLock::new(None);

/// An uncertain BAR restoration retained after its bounded retry refused.
static RETAINED_PROBE: SpinLock<Option<ProbeRecovery>> = SpinLock::new(None);

/// Device leases whose final exact release remained refused.
static RETAINED_RELEASES: SpinLock<[Option<Device>; FUNCTION_AUTHORITY_LIMIT]> =
    SpinLock::new([const { None }; FUNCTION_AUTHORITY_LIMIT]);

/// Bare function claims whose final sublease-free release remained refused.
static RETAINED_FUNCTIONS: SpinLock<[Option<pci::FunctionClaim>; FUNCTION_AUTHORITY_LIMIT]> =
    SpinLock::new([const { None }; FUNCTION_AUTHORITY_LIMIT]);

/// Make the first real DMA admission exercise the post-disable rollback path.
#[cfg(feature = "selftest")]
static FORCED_BUS_MASTER_REFUSALS: AtomicUsize = AtomicUsize::new(1);

/// Exercise cleanup after the function and its BAR shape have been claimed.
#[cfg(feature = "selftest")]
static REFUSE_CLEAN_PREPARATION: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static OBSERVED_BUS_MASTER_REFUSAL: AtomicBool = AtomicBool::new(false);

/// Why a device could not be claimed.
#[derive(Debug)]
pub(crate) enum ClaimError {
    /// Configuration space cannot be reached.
    NoBus,
    /// Nothing is at that address.
    Absent,
    /// Another driver holds it, and still does.
    Taken,
    /// The registry is full, which bounds devices ever claimed.
    NoRoom,
    /// An I/O window the function decodes is already claimed by something that
    /// is not a device — the bare-port path D-77 said proves the wrong thing.
    WindowTaken,
    /// A BAR consumes another header field or reports an impossible window.
    InvalidWindow,
    /// BAR or command restoration remains uncertain under its function lease.
    ProbeRestore(ProbeRecovery),
    /// A restored, otherwise clean function remains quarantined in the registry.
    FunctionRetained,
}

/// A sibling requester held unable to bus-master for a unit-wide VT-d
/// transition.
#[derive(Debug)]
pub(crate) struct DmaFence {
    function: Option<pci::FunctionClaim>,
    command_state: Option<CommandState>,
}

/// Why a sibling requester could not enter or leave the unit fence.
#[derive(Debug, Clone, Copy)]
pub(crate) enum DmaFenceError {
    Claim(pci::ClaimError),
    Command(EnableError),
    Quiesce,
    Function,
}

impl core::fmt::Display for DmaFenceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Claim(reason) => write!(formatter, "function claim: {reason}"),
            Self::Command(reason) => write!(formatter, "command fence: {reason}"),
            Self::Quiesce => core::fmt::Write::write_str(
                formatter,
                "the entry-quiesced ISA requester changed or its 8237 controllers are not quiet",
            ),
            Self::Function => {
                core::fmt::Write::write_str(formatter, "function authority could not be returned")
            }
        }
    }
}

/// The ownership-preserving results of acquiring a sibling DMA fence.
#[derive(Debug)]
pub(crate) enum DmaFenceOutcome {
    Fenced(DmaFence),
    Refused(DmaFenceError),
    Retained {
        reason: DmaFenceError,
        fence: DmaFence,
    },
}

/// A refused sibling-fence release with its complete retry owner.
#[derive(Debug)]
pub(crate) struct DmaFenceRelease {
    pub(crate) reason: DmaFenceError,
    pub(crate) fence: DmaFence,
}

impl DmaFence {
    /// Restore the safe command word and return the exact function claim.
    pub(crate) fn release(mut self) -> Result<(), DmaFenceRelease> {
        if let Some(state) = self.command_state.take() {
            let Some(function) = self.function.as_ref() else {
                panic!("a DMA fence lost its function authority");
            };
            if let Err(failure) = pci::restore_command(function, state) {
                let reason = DmaFenceError::Command(failure.reason());
                self.command_state = Some(failure.into_state());
                return Err(DmaFenceRelease {
                    reason,
                    fence: self,
                });
            }
        }
        let Some(function) = self.function.take() else {
            panic!("a DMA fence lost its function authority");
        };
        if let Err(failure) = pci::release_function(function) {
            self.function = Some(failure.into_function());
            return Err(DmaFenceRelease {
                reason: DmaFenceError::Function,
                fence: self,
            });
        }
        Ok(())
    }
}

/// Claim one declared sibling requester and verify BME clear.
///
/// The one entry-quiesced ISA requester cannot clear BME; it is held instead
/// by its exclusive claim, its unchanged identity and command, and a fresh
/// remask of its 8237 controllers with no request pending (D-426).
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
            function: Some(function),
            command_state: Some(command_state),
        }),
        Err(failure) => {
            let reason = DmaFenceError::Command(failure.reason());
            if let Some(command_state) = failure.into_state() {
                return DmaFenceOutcome::Retained {
                    reason,
                    fence: DmaFence {
                        function: Some(function),
                        command_state: Some(command_state),
                    },
                };
            }
            match pci::release_function(function) {
                Ok(()) => DmaFenceOutcome::Refused(reason),
                Err(failure) => DmaFenceOutcome::Retained {
                    reason: DmaFenceError::Function,
                    fence: DmaFence {
                        function: Some(failure.into_function()),
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
            function: Some(function),
            command_state: None,
        });
    }
    match pci::release_function(function) {
        Ok(()) => DmaFenceOutcome::Refused(DmaFenceError::Quiesce),
        Err(failure) => DmaFenceOutcome::Retained {
            reason: DmaFenceError::Function,
            fence: DmaFence {
                function: Some(failure.into_function()),
                command_state: None,
            },
        },
    }
}

/// One uncertain BAR restoration and the function authority it still owns.
#[derive(Debug)]
pub(crate) struct ProbeRecovery {
    function: pci::FunctionClaim,
    authority: ProbeRecoveryAuthority,
}

#[derive(Debug)]
enum ProbeRecoveryAuthority {
    Probe(ProbeRetry),
    Command(CommandState),
}

/// A refused recovery with the unchanged function and restoration authority.
#[derive(Debug)]
pub(crate) struct ProbeRecoveryFailure {
    reason: ProbeError<()>,
    recovery: ProbeRecovery,
}

/// Result of retrying an uncertain PCI probe restoration.
#[derive(Debug)]
#[must_use = "probe recovery may retain exclusive function authority"]
pub(crate) enum ProbeRecoveryOutcome {
    /// Registers were restored and the function claim was returned.
    Recovered,
    /// Restoration still owns an exact retry.
    Refused(ProbeRecoveryFailure),
    /// Restoration finished, but the function claim remains quarantined.
    FunctionRetained,
}

impl ProbeRecoveryFailure {
    /// Why the restoration remains uncertain.
    pub(crate) const fn reason(&self) -> &ProbeError<()> {
        &self.reason
    }

    /// Recover the complete retry authority.
    pub(crate) fn into_recovery(self) -> ProbeRecovery {
        self.recovery
    }
}

impl ProbeRecovery {
    /// Retry exact BAR and command restoration, then release the function.
    pub(crate) fn retry(self) -> ProbeRecoveryOutcome {
        let Self {
            function,
            authority,
        } = self;
        let restored = match authority {
            ProbeRecoveryAuthority::Probe(retry) => match pci::retry_probe(&function, retry) {
                Ok(()) => Ok(()),
                Err(failure) => {
                    let reason = *failure.reason();
                    let Some(retry) = failure.into_retry() else {
                        retain_failed_function(function);
                        return ProbeRecoveryOutcome::FunctionRetained;
                    };
                    Err((reason, ProbeRecoveryAuthority::Probe(retry)))
                }
            },
            ProbeRecoveryAuthority::Command(state) => {
                match pci::restore_command(&function, state) {
                    Ok(()) => Ok(()),
                    Err(failure) => Err((
                        ProbeError::Unavailable(failure.reason()),
                        ProbeRecoveryAuthority::Command(failure.into_state()),
                    )),
                }
            }
        };
        match restored {
            Ok(()) => match pci::release_function(function) {
                Ok(()) => ProbeRecoveryOutcome::Recovered,
                Err(failure) => {
                    retain_failed_function(failure.into_function());
                    ProbeRecoveryOutcome::FunctionRetained
                }
            },
            Err((reason, authority)) => ProbeRecoveryOutcome::Refused(ProbeRecoveryFailure {
                reason,
                recovery: ProbeRecovery {
                    function,
                    authority,
                },
            }),
        }
    }
}

impl core::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoBus => core::fmt::Write::write_str(f, "configuration space is not reachable"),
            Self::Absent => core::fmt::Write::write_str(f, "nothing is at that address"),
            Self::Taken => core::fmt::Write::write_str(f, "another driver holds it"),
            Self::NoRoom => core::fmt::Write::write_str(f, "no room in the device registry"),
            Self::WindowTaken => {
                core::fmt::Write::write_str(f, "an i/o window it decodes is already claimed")
            }
            Self::InvalidWindow => {
                core::fmt::Write::write_str(f, "a base address window is malformed")
            }
            Self::ProbeRestore(_) => {
                core::fmt::Write::write_str(f, "a base address probe could not be restored")
            }
            Self::FunctionRetained => {
                core::fmt::Write::write_str(f, "the restored function claim could not be returned")
            }
        }
    }
}

/// Why a probed memory BAR could not become page-table authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarAdmissionError {
    /// The requested BAR slot is empty.
    Missing,
    /// The requested BAR is an I/O window.
    NotMemory,
    /// CPU width, range arithmetic, or immutable memory-map policy refused it.
    Range(paging::MapError),
    /// Exact function/BAR binding refused it.
    Bind(pci::BarBindError),
}

impl core::fmt::Display for BarAdmissionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing => core::fmt::Write::write_str(f, "the BAR slot is empty"),
            Self::NotMemory => core::fmt::Write::write_str(f, "the BAR is not memory"),
            Self::Range(reason) => write!(f, "physical range: {reason}"),
            Self::Bind(reason) => write!(f, "function binding: {reason}"),
        }
    }
}

/// A refused memory-decode transition with the inaccessible mapped BAR.
#[derive(Debug)]
pub(crate) struct BarDecodeFailure {
    reason: BarEnableError,
    bar: pci::DisabledBar,
}

/// Result of the function-owned BAR decode transition.
#[derive(Debug)]
#[must_use = "decode refusal retains an inaccessible mapping"]
pub(crate) enum BarDecodeOutcome {
    /// Memory decode is verified and the mapping is accessible.
    Enabled(pci::MappedBar),
    /// Decode was refused with the mapping retained for cleanup.
    Refused(BarDecodeFailure),
}

impl BarDecodeFailure {
    /// Why the BAR did not become accessible.
    pub(crate) const fn reason(&self) -> BarEnableError {
        self.reason
    }

    /// Recover the mapping for exact retirement.
    pub(crate) fn into_bar(self) -> pci::DisabledBar {
        self.bar
    }
}

/// Why a transient function lease could not be retired exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReleaseError {
    /// A configuration write may have taken effect without verified rollback.
    Command,
    /// The dynamic I/O claims no longer matched the unique span handles.
    Ports,
    /// The function registry refused the exact claim after all subleases ended.
    Function,
    /// A permanent driver claim was offered to the transient release door.
    Permanent,
}

impl core::fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Command => {
                core::fmt::Write::write_str(f, "the pci command word could not be restored")
            }
            Self::Ports => {
                core::fmt::Write::write_str(f, "the device's i/o spans could not be returned")
            }
            Self::Function => {
                core::fmt::Write::write_str(f, "the function claim could not be returned")
            }
            Self::Permanent => {
                core::fmt::Write::write_str(f, "a permanent device claim cannot be released")
            }
        }
    }
}

/// Result of retiring a device, retaining the complete lease on refusal.
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

    /// Recover the refusal and unchanged lease for an exact retry.
    pub(crate) fn into_retained(self) -> Option<(ReleaseError, Device)> {
        self.retained
    }
}

/// A claimed function, and the evidence for touching it.
///
/// Not `Clone` and not `Copy`: it *is* the claim, and a copy would be a second
/// driver for one device with nothing to say so. A fixed claim outlives the
/// boot; a selection lease has an explicit all-or-stop release (D-199).
#[derive(Debug)]
pub(crate) struct Device {
    address: Address,
    function: Option<pci::FunctionClaim>,
    vendor: u16,
    device: u16,
    bars: ProbedBars,
    // Keep port and function authority inseparable (D-77).
    ports: [Option<wost::arch::current::port::PortSpan>; BARS],
    /// Whether selection may still retire this function.
    lifetime: ClaimLifetime,
    /// The command word to restore before a transient lease may disappear.
    command_state: Option<CommandState>,
    /// A failed restoration makes reuse unsafe even if later reads look sane.
    uncertain: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimLifetime {
    Selection,
    Permanent,
}

impl Device {
    /// Where it lives.
    pub(crate) const fn at(&self) -> Address {
        self.address
    }

    /// Who made it, and which of theirs it is.
    pub(crate) const fn identity(&self) -> (u16, u16) {
        (self.vendor, self.device)
    }

    /// The window base address register `index` decodes, if it decodes one.
    pub(crate) fn window(&self, index: usize) -> Option<Window> {
        self.bars.window(index)
    }

    /// Bind one probed memory BAR to this CPU and immutable boot map.
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
        pci::bind_memory_bar(self.function(), &self.bars, index, range)
            .map_err(BarAdmissionError::Bind)
    }

    /// Admit bus mastering with fresh proof from this function's mapped BAR,
    /// behind a DMA domain established for this exact function (D-357).
    ///
    /// # Errors
    ///
    /// [`EnableError`] after an exact rollback, or when rollback could not be
    /// verified. The command restoration authority remains in this device.
    pub(crate) fn admit_bus_master(
        &mut self,
        authority: pci::NvmeBusMasterAuthority<'_>,
        domain: DmaDomain<'_>,
        interrupts: InterruptIsolation<'_>,
    ) -> Result<(), EnableError> {
        #[cfg(feature = "selftest")]
        {
            if FORCED_BUS_MASTER_REFUSALS
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                OBSERVED_BUS_MASTER_REFUSAL.store(true, Ordering::Release);
                return Err(EnableError::Refused(0));
            }
        }

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

    /// Verify BME off and make that restriction survive transient release.
    ///
    /// The exact command/function owner stays in `self` on every refusal, so
    /// the caller can retry once or quarantine the complete device beside its
    /// DMA domain.
    pub(crate) fn disable_bus_master_for_retirement(&mut self) -> Result<(), EnableError> {
        let Some(function) = self.function.as_ref() else {
            self.uncertain = true;
            return Err(EnableError::Stale);
        };
        let Some(state) = self.command_state.as_mut() else {
            self.uncertain = true;
            return Err(EnableError::Stale);
        };
        match pci::disable_bus_master_for_retirement(function, state) {
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

    /// Restore the now-safe command word while retaining function authority.
    ///
    /// Retirement calls this before a unit-wide VT-d disable. The command
    /// lease is settled, but the enclosing [`Device`] continues to exclude
    /// every other configuration writer until the unit transition finishes.
    pub(crate) fn settle_bus_master_fence_for_retirement(&mut self) -> Result<(), EnableError> {
        let Some(state) = self.command_state.take() else {
            return if self.uncertain {
                Err(EnableError::Stale)
            } else {
                Ok(())
            };
        };
        if let Err(failure) = pci::restore_command(self.function(), state) {
            let reason = failure.reason();
            self.command_state = Some(failure.into_state());
            self.uncertain = true;
            return Err(reason);
        }
        self.uncertain = false;
        Ok(())
    }

    /// Freshly prove that this exact command owner still has BME clear before
    /// a unit-wide VT-d transition.
    pub(crate) fn verify_bus_master_fence(&self) -> Result<(), EnableError> {
        let Some(function) = self.function.as_ref() else {
            return Err(EnableError::Stale);
        };
        let Some(state) = self.command_state.as_ref() else {
            return Err(EnableError::Stale);
        };
        pci::verify_bus_master_fence(function, state)
    }

    /// Expose one mapped BAR only after its live identity enables memory decode.
    pub(crate) fn enable_memory_bar(&mut self, bar: pci::DisabledBar) -> BarDecodeOutcome {
        let Some(function) = self.function.as_ref() else {
            panic!("a production PCI device has no function authority");
        };
        let Some(state) = self.command_state.as_mut() else {
            return BarDecodeOutcome::Refused(BarDecodeFailure {
                reason: BarEnableError::Command(EnableError::Stale),
                bar,
            });
        };
        match pci::enable_memory_bar(function, bar, state) {
            pci::BarEnableOutcome::Enabled(bar) => BarDecodeOutcome::Enabled(bar),
            pci::BarEnableOutcome::Refused(failure) => {
                let reason = failure.reason();
                let bar = failure.into_bar();
                self.uncertain |= matches!(reason, BarEnableError::Command(EnableError::Restore));
                BarDecodeOutcome::Refused(BarDecodeFailure { reason, bar })
            }
        }
    }

    /// Make a successful selection lease permanent.
    pub(crate) fn commit(&mut self) -> Result<(), EnableError> {
        if let Some(state) = self.command_state.take()
            && let Err(failure) = pci::commit_command(self.function(), state)
        {
            let reason = failure.reason();
            self.command_state = Some(failure.into_state());
            self.uncertain = true;
            return Err(reason);
        }
        self.lifetime = ClaimLifetime::Permanent;
        self.uncertain = false;
        Ok(())
    }

    /// Restore and return every resource held by a transient lease.
    ///
    /// Any uncertainty leaves the function recorded and forbids selection from
    /// advancing. Successful return is the proof that the next candidate may
    /// be touched (D-199).
    pub(crate) fn release(mut self, _context: &mut ThreadContext) -> ReleaseOutcome {
        if self.lifetime == ClaimLifetime::Permanent {
            return ReleaseOutcome::refused(ReleaseError::Permanent, self);
        }
        if let Some(state) = self.command_state.take() {
            if let Err(failure) = pci::restore_command(self.function(), state) {
                self.command_state = Some(failure.into_state());
                return ReleaseOutcome::refused(ReleaseError::Command, self);
            }
            self.uncertain = false;
        }
        if self.uncertain {
            return ReleaseOutcome::refused(ReleaseError::Command, self);
        }

        let ports = core::mem::replace(&mut self.ports, [const { None }; BARS]);
        if let Err(failure) = wost::arch::current::port::release_spans(ports) {
            self.ports = failure.spans;
            return ReleaseOutcome::refused(ReleaseError::Ports, self);
        }
        let Some(function) = self.function.take() else {
            panic!("a production PCI device lost its function authority");
        };
        if let Err(failure) = pci::release_function(function) {
            self.function = Some(failure.into_function());
            return ReleaseOutcome::refused(ReleaseError::Function, self);
        }
        ReleaseOutcome::released()
    }

    fn function(&self) -> &pci::FunctionClaim {
        let Some(function) = self.function.as_ref() else {
            panic!("a production PCI device has no function authority");
        };
        function
    }
}

/// Whether the deterministic post-disable DMA-admission refusal was consumed.
#[cfg(feature = "selftest")]
pub(crate) fn bus_master_refusal_was_observed() -> bool {
    OBSERVED_BUS_MASTER_REFUSAL.load(Ordering::Acquire)
}

/// Refuse the next clean preparation after its BAR probe has settled.
#[cfg(feature = "selftest")]
pub(crate) fn refuse_clean_preparation_for_test() {
    assert!(
        !REFUSE_CLEAN_PREPARATION.swap(true, Ordering::Relaxed),
        "a clean PCI preparation refusal was already armed"
    );
}

/// Keep a command-mismatched device claimed after storage selection stops.
pub(crate) fn retain_failed_commit(device: Device) {
    let mut retained = RETAINED_COMMAND.lock_leaf();
    assert!(retained.is_none(), "two PCI commit failures were retained");
    *retained = Some(device);
}

/// Keep an uncertain BAR restoration owned after its bounded retry refused.
pub(crate) fn retain_failed_probe(recovery: ProbeRecovery) {
    let mut retained = RETAINED_PROBE.lock_leaf();
    assert!(retained.is_none(), "two PCI probe failures were retained");
    *retained = Some(recovery);
}

/// Keep a device lease reachable after its bounded retirement refuses.
pub(crate) fn retain_failed_release(device: Device) {
    let mut retained = RETAINED_RELEASES.lock_leaf();
    let Some(slot) = retained.iter_mut().find(|slot| slot.is_none()) else {
        panic!("retained device leases exceed the twelve-function PCI registry");
    };
    *slot = Some(device);
}

fn retain_failed_function(function: pci::FunctionClaim) {
    let mut retained = RETAINED_FUNCTIONS.lock_leaf();
    let Some(slot) = retained.iter_mut().find(|slot| slot.is_none()) else {
        panic!("retained function claims exceed the twelve-function PCI registry");
    };
    *slot = Some(function);
}

/// Claim the function at `at`, with every window it decodes.
///
/// The claim is what makes touching the device legitimate, and it carries the
/// I/O runs with it — so a later bare port claim inside one of them is refused
/// by the port registry, which is D-77's entitlement half answered by making
/// the device the claimant (D-168).
///
/// # Errors
///
/// See [`ClaimError`]. Clean refusals record nothing. An uncertain BAR restore
/// returns its exclusive retry owner rather than releasing the function.
/// Lease one function while storage selection determines whether to keep it.
pub(crate) fn lease(at: Address) -> Result<Device, ClaimError> {
    claim_as(at, ClaimLifetime::Selection)
}

fn claim_as(at: Address, lifetime: ClaimLifetime) -> Result<Device, ClaimError> {
    let function = pci::claim_function(at).map_err(|reason| match reason {
        pci::ClaimError::Unavailable => ClaimError::NoBus,
        pci::ClaimError::Absent => ClaimError::Absent,
        pci::ClaimError::Taken => ClaimError::Taken,
        pci::ClaimError::NoRoom | pci::ClaimError::Exhausted => ClaimError::NoRoom,
        pci::ClaimError::Header(_) => ClaimError::InvalidWindow,
        _ => ClaimError::NoBus,
    })?;
    let address = function.address();
    let (vendor, device) = function.identity();
    let staged = match pci::probe_bars(&function) {
        Ok(staged) => staged,
        Err(failure) => {
            let invalid = matches!(
                failure.reason(),
                ProbeError::Malformed | ProbeError::InvalidWindow
            );
            if let Some(retry) = failure.into_retry() {
                return Err(ClaimError::ProbeRestore(ProbeRecovery {
                    function,
                    authority: ProbeRecoveryAuthority::Probe(retry),
                }));
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

    #[cfg(feature = "selftest")]
    if REFUSE_CLEAN_PREPARATION.swap(false, Ordering::Relaxed) {
        return Err(restore_staged_function(
            function,
            command_state,
            ClaimError::WindowTaken,
        ));
    }

    let ports = match claim_windows(&bars) {
        Ok(ports) => ports,
        Err(reason) => {
            return Err(restore_staged_function(function, command_state, reason));
        }
    };

    Ok(Device {
        address,
        function: Some(function),
        vendor,
        device,
        bars,
        ports,
        lifetime,
        command_state: Some(command_state),
        uncertain: false,
    })
}

fn claim_windows(
    bars: &ProbedBars,
) -> Result<[Option<wost::arch::current::port::PortSpan>; BARS], ClaimError> {
    let mut requested = [(0u16, 0usize); BARS];
    let mut port_slots = [0usize; BARS];
    let mut port_count = 0;
    for index in 0..BARS {
        if let Some(Window {
            bar: Bar::Io { port },
            size,
        }) = bars.window(index)
        {
            let len = usize::try_from(size).map_err(|_| ClaimError::InvalidWindow)?;
            requested[port_count] = (port, len);
            port_slots[port_count] = index;
            port_count += 1;
        }
    }

    let mut ports = [const { None }; BARS];
    if port_count > 0 {
        let owned = wost::arch::current::port::claim_spans::<BARS>(&requested[..port_count])
            .ok_or(ClaimError::WindowTaken)?;
        for (dense, span) in owned.into_iter().enumerate().take(port_count) {
            ports[port_slots[dense]] = span;
        }
    }
    Ok(ports)
}

fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    if let Err(failure) = pci::release_function(function) {
        retain_failed_function(failure.into_function());
        return ClaimError::FunctionRetained;
    }
    reason
}

fn restore_staged_function(
    function: pci::FunctionClaim,
    state: CommandState,
    reason: ClaimError,
) -> ClaimError {
    match pci::restore_command(&function, state) {
        Ok(()) => release_clean_function(function, reason),
        Err(failure) => ClaimError::ProbeRestore(ProbeRecovery {
            function,
            authority: ProbeRecoveryAuthority::Command(failure.into_state()),
        }),
    }
}

/// Whether the function at `at` is claimed.
#[cfg(feature = "selftest")]
pub(crate) fn is_claimed(at: Address) -> bool {
    pci::is_claimed(at)
}

/// How many functions are claimed.
#[cfg(feature = "selftest")]
pub(crate) fn claims() -> usize {
    pci::claims()
}
