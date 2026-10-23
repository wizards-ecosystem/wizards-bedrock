//! Legacy CF8/CFC PCI configuration-space access.
//!
//! `pci-config` owns the encoding and decoding. This module owns the ports,
//! claimed atomically and locked across each address/data pair. CF8/CFC is the
//! common mechanism on the QEMU i440fx and VMware 440BX targets (D-18, D-138).

use nvme::{ControllerStateError, DisabledRegisters, register};
use pci_config::{
    Address, Bar, BarAuthority, BarAuthorityError, BarAuthorityFailure, BarEnableAccess,
    BarEnableError as ConfigBarEnableError, CommandAccess, CommandState, EnableError,
    EnableFailure, FunctionAuthority, FunctionClaimError, FunctionClaims, FunctionReleaseError,
    HandoffAccess, HandoffFenceError, HandoffFencePolicy, HandoffFenceReport, HeaderType,
    ProbeAccess, ProbeError, ProbeFailure, ProbeRetry, ProbedBars, RestoreFailure, StagedProbe,
    Window, header,
};

use crate::arch::current::paging::{AdmittedDeviceRange, DeviceMapping, ReleasableDevice};
use crate::arch::current::vtd::{DmaDomain, InterruptIsolation};
use crate::mmio::MmioError;
use crate::sync::SpinLock;

use super::port::{self, Port, PortBlock};

/// Functions that may be held by drivers at once.
const FUNCTION_CLAIMS: usize = pci_config::FUNCTION_AUTHORITY_LIMIT;

struct Configuration {
    ports: Option<(Port<u32>, Port<u32>)>,
    functions: FunctionClaims<FUNCTION_CLAIMS>,
}

impl Configuration {
    const fn new() -> Self {
        Self {
            ports: None,
            functions: FunctionClaims::new(),
        }
    }
}

/// The claimed pair and every live command transition under one ownership lock.
static CONFIGURATION: SpinLock<Configuration> = SpinLock::new(Configuration::new());

/// Reclaim the configuration registry on the terminal panic path.
pub(crate) fn force_unlock(panic: &crate::panic::PanicToken) {
    CONFIGURATION.force_unlock(panic);
    ISA_DMA.force_unlock(panic);
}

/// Claim the two ports, or report that something else already has them.
///
/// Idempotent: a second caller finds the pair already claimed and returns
/// `true` having done nothing, because the question it is really asking is
/// "can configuration space be reached", and it can.
#[must_use = "a refused PCI initialization left configuration space unavailable"]
pub fn init() -> bool {
    if is_available() {
        return true;
    }
    // Claim before taking this module's leaf guard to preserve lock order.
    // The one typed door to configuration space mints the pair (D-351).
    let Some(pair) = port::pci_configuration() else {
        return is_available();
    };

    let mut configuration = CONFIGURATION.lock_leaf();
    if configuration.ports.is_none() {
        configuration.ports = Some(pair);
    }
    true
}

/// Whether configuration space can be reached at all.
pub fn is_available() -> bool {
    CONFIGURATION.lock_leaf().ports.is_some()
}

/// Why the entry-time exhaustive requester fence could not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DmaHandoffFenceError {
    /// CF8/CFC could not be reserved.
    Unavailable,
    /// A function authority already exists, so a raw segment transition is unsafe.
    Busy,
    /// One exhaustive configuration transaction was uncertain.
    Fence(HandoffFenceError),
}

impl core::fmt::Display for DmaHandoffFenceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable => {
                core::fmt::Write::write_str(formatter, "PCI configuration is unavailable")
            }
            Self::Busy => core::fmt::Write::write_str(
                formatter,
                "PCI function authority exists before the handoff fence",
            ),
            Self::Fence(reason) => core::fmt::Display::fmt(reason, formatter),
        }
    }
}

impl core::error::Error for DmaHandoffFenceError {}

/// Permanently clear and verify BME on every present segment-zero function.
///
/// This one-time transition is valid only before any function authority has
/// been minted. Pure enumeration, ordering and readback policy live in
/// `pci-config`; this adapter owns only the serialized port instructions.
///
/// # Errors
///
/// If CF8/CFC is unavailable, a function is already owned, or any exhaustive
/// configuration transaction is uncertain.
pub fn fence_dma_handoff(
    policy: HandoffFencePolicy,
) -> Result<HandoffFenceReport, DmaHandoffFenceError> {
    // Claim before taking this module's leaf guard to preserve lock order. A
    // refused claim leaves the controllers unowned, so a quiesce refuses.
    let mut isa_dma = if policy.quiesces_isa_dma() {
        port::isa_dma_controllers().map(|(first, second)| IsaDma { first, second })
    } else {
        None
    };
    let fenced = {
        let mut configuration = CONFIGURATION.lock_leaf();
        if configuration.functions.len() != 0 {
            Err(DmaHandoffFenceError::Busy)
        } else if let Some((address, data)) = configuration.ports.as_mut() {
            pci_config::fence_dma_handoff(policy, |at, operation| {
                Port::write(address, at.bits());
                match operation {
                    HandoffAccess::ReadIdentity
                    | HandoffAccess::ReadClass
                    | HandoffAccess::ReadCommand => Some(Port::read(data)),
                    HandoffAccess::WriteCommand(value) => {
                        Port::write(data, u32::from(value));
                        Some(0)
                    }
                    HandoffAccess::QuiesceIsaDma => isa_dma.as_mut().map(IsaDma::quiesce),
                }
            })
            .map_err(DmaHandoffFenceError::Fence)
        } else {
            Err(DmaHandoffFenceError::Unavailable)
        }
    };
    if let Some(controllers) = isa_dma {
        *ISA_DMA.lock_leaf() = Some(controllers);
    }
    fenced
}

/// Both 8237 controllers, owned for the rest of the boot once claimed, so no
/// later port owner can unmask a channel behind a quiesced ISA requester.
static ISA_DMA: SpinLock<Option<IsaDma>> = SpinLock::new(None);

/// The permanent claim on DMA1 (`0x00..0x10`) and DMA2 (`0xC0..0xE0`).
struct IsaDma {
    first: PortBlock<16>,
    second: PortBlock<32>,
}

impl IsaDma {
    /// Master-clear and mask both controllers, then read both status words.
    ///
    /// A master clear masks every channel by definition; the explicit
    /// all-mask write follows because the mask registers cannot be read back.
    fn quiesce(&mut self) -> u32 {
        self.first.write::<u8>(0x0D, 0);
        self.second.write::<u8>(0x1A, 0);
        self.reassert()
    }

    /// Mask every channel again and read both status registers.
    fn reassert(&mut self) -> u32 {
        self.first.write::<u8>(0x0F, 0x0F);
        self.second.write::<u8>(0x1E, 0x0F);
        let first = self.first.read::<u8>(0x08);
        let second = self.second.read::<u8>(0x10);
        u32::from(first) | u32::from(second) << 8
    }
}

/// Whether the entry quiesce still holds: both 8237 controllers are owned,
/// remasked, and report no pending request. `false` if none was ever claimed.
pub fn isa_dma_quiet() -> bool {
    ISA_DMA.lock_leaf().as_mut().is_some_and(|controllers| {
        controllers.reassert() & pci_config::ISA_DMA_PENDING_REQUESTS == 0
    })
}

/// Read the dword at `at`.
///
/// `None` only when the ports were never claimed. **An absent function is not
/// an error**: the bus answers all ones for one that is not there, so
/// `0xFFFF_FFFF` is a value and `pci_config::is_absent` is what reads it.
pub fn read(at: Address) -> Option<u32> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let (address, data) = configuration.ports.as_mut()?;
    Port::write(address, at.bits());
    Some(Port::read(data))
}

/// The exclusive right to mutate one function's configuration registers.
///
/// Only [`claim_function`] constructs one, after reading and validating the
/// function's own identity and header through this module's claimed ports.
/// Dropping it retains the global claim rather than making another caller's
/// raw address authoritative.
#[derive(Debug)]
#[must_use = "dropping PCI function authority permanently retains its claim"]
pub struct FunctionClaim {
    authority: FunctionAuthority,
    identity: u32,
}

/// A page-safe physical range bound to one live function and memory BAR.
#[derive(Debug)]
pub struct AdmittedBar {
    pub(super) authority: BarAuthority,
    pub(super) range: AdmittedDeviceRange,
}

impl AdmittedBar {
    /// Rounded physical pages and the runtime width that admitted them.
    pub const fn physical_admission(&self) -> (u64, u64, u8) {
        (
            self.range.page_start(),
            self.range.page_end(),
            self.range.physical_bits(),
        )
    }
}

/// A BAR authority whose pages are installed but not yet exposed as registers.
#[derive(Debug)]
pub struct BarMapping {
    pub(super) authority: BarAuthority,
    pub(super) mapping: DeviceMapping,
}

/// A mapped BAR kept inaccessible until memory decode is verified.
#[derive(Debug)]
pub struct DisabledBar {
    authority: BarAuthority,
    region: ReleasableDevice,
}

/// Register access retaining the exact function generation and BAR identity.
#[derive(Debug)]
pub struct MappedBar {
    authority: BarAuthority,
    region: ReleasableDevice,
}

/// Fresh NVMe quiescence observed through one exact function-bound BAR.
#[derive(Debug)]
#[must_use = "NVMe quiescence authority must be consumed by DMA admission"]
pub struct NvmeBusMasterAuthority<'bar> {
    bar: &'bar MappedBar,
    _observed: DisabledRegisters,
}

/// Why a mapped BAR could not prove that its NVMe controller is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NvmeObservationError {
    /// NVMe controller registers must come from BAR0.
    WrongBar(usize),
    /// A register access was outside or misaligned within the mapped BAR.
    Access(MmioError),
    /// `CC.EN` or `CSTS.RDY` remained set.
    State(ControllerStateError),
}

impl core::fmt::Display for NvmeObservationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongBar(index) => write!(formatter, "NVMe registers came from BAR{index}"),
            Self::Access(reason) => write!(formatter, "NVMe register access: {reason}"),
            Self::State(reason) => core::fmt::Display::fmt(reason, formatter),
        }
    }
}

impl core::error::Error for NvmeObservationError {}

/// A mapped BAR converted back into exact unmap authority.
#[derive(Debug)]
pub struct ReleasableBar {
    pub(super) authority: BarAuthority,
    pub(super) region: ReleasableDevice,
}

impl BarMapping {
    /// Claim inaccessible register storage after its pages become live.
    ///
    /// # Errors
    ///
    /// Returns the complete mapping if the running root no longer contains it.
    #[expect(
        clippy::result_large_err,
        reason = "refusal must return the complete linear BAR authority without allocating"
    )]
    pub fn claim_disabled(self) -> Result<DisabledBar, Self> {
        let Self { authority, mapping } = self;
        match mapping.claim_releasable() {
            Ok(region) => Ok(DisabledBar { authority, region }),
            Err(mapping) => Err(Self { authority, mapping }),
        }
    }
}

impl DisabledBar {
    /// Recover exact unmap authority while register access remains hidden.
    pub fn into_releasable(self) -> ReleasableBar {
        ReleasableBar {
            authority: self.authority,
            region: self.region,
        }
    }
}

impl MappedBar {
    /// Where the registers are mapped in the running address space.
    pub const fn address(&self) -> u64 {
        self.region.address()
    }

    /// Read one register word.
    pub fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        ReleasableDevice::read_u32(&self.region, offset)
    }

    /// Write one register word.
    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        ReleasableDevice::write_u32(&mut self.region, offset, value)
    }

    /// Observe the two live NVMe registers required before DMA admission.
    ///
    /// The returned authority borrows this mapping, preventing another safe
    /// register write before it is consumed by [`admit_bus_master`].
    ///
    /// # Errors
    ///
    /// Refuses a non-BAR0 mapping, either inaccessible register, or a set
    /// `CC.EN` or `CSTS.RDY` bit.
    pub fn observe_nvme_disabled(
        &self,
    ) -> Result<NvmeBusMasterAuthority<'_>, NvmeObservationError> {
        if self.authority.index() != 0 {
            return Err(NvmeObservationError::WrongBar(self.authority.index()));
        }
        let cc = self
            .read_u32(register::CC)
            .map_err(NvmeObservationError::Access)?;
        let csts = self
            .read_u32(register::CSTS)
            .map_err(NvmeObservationError::Access)?;
        let observed =
            DisabledRegisters::from_bits(cc, csts).map_err(NvmeObservationError::State)?;
        Ok(NvmeBusMasterAuthority {
            bar: self,
            _observed: observed,
        })
    }

    /// Recover exact unmap authority for a rejected controller.
    pub fn into_releasable(self) -> ReleasableBar {
        ReleasableBar {
            authority: self.authority,
            region: self.region,
        }
    }
}

impl NvmeBusMasterAuthority<'_> {
    /// The live register values carried by this authority.
    #[cfg(feature = "selftest")]
    pub fn observation_for_test(&self) -> (u32, u32) {
        self._observed.bits()
    }
}

/// Why a CPU-admitted physical range could not bind to one probed BAR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BarBindError {
    /// The physical base or length differs from the probed BAR window.
    Range,
    /// Function generation, BAR kind, or exclusive ownership was refused.
    Authority(BarAuthorityError),
}

impl core::fmt::Display for BarBindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Range => {
                core::fmt::Write::write_str(f, "the admitted range differs from the BAR")
            }
            Self::Authority(reason) => write!(f, "BAR authority: {reason}"),
        }
    }
}

impl core::error::Error for BarBindError {}

/// A refused decode transition retaining the mapping and any command owner.
#[derive(Debug)]
#[must_use = "a refused decode transition retains mapping authority"]
pub struct BarEnableFailure {
    reason: ConfigBarEnableError,
    bar: DisabledBar,
}

/// Result of exposing a mapped BAR through memory decode.
#[derive(Debug)]
#[must_use = "a refused decode transition retains mapping and restoration authority"]
pub enum BarEnableOutcome {
    /// The BAR is accessible and the command transition remains owned.
    Enabled(MappedBar),
    /// Decode was refused with every remaining owner intact.
    Refused(BarEnableFailure),
}

impl BarEnableFailure {
    /// Why memory decode did not become usable.
    pub const fn reason(&self) -> ConfigBarEnableError {
        self.reason
    }

    /// Recover the inaccessible mapping for exact retirement.
    pub fn into_bar(self) -> DisabledBar {
        self.bar
    }
}

impl FunctionClaim {
    /// The claimed bus/device/function identity, with register offset zero.
    pub const fn address(&self) -> Address {
        self.authority.address()
    }

    /// The vendor and device word read while the function was claimed.
    pub const fn identity(&self) -> (u16, u16) {
        (self.identity as u16, (self.identity >> 16) as u16)
    }
}

/// Why exclusive function authority could not be established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClaimError {
    /// Configuration space is unavailable.
    Unavailable,
    /// The function does not exist.
    Absent,
    /// Another live authority already owns the function.
    Taken,
    /// The bounded registry has no free slot.
    NoRoom,
    /// Registry identity or generation space is exhausted.
    Exhausted,
    /// The function reports an unsupported header layout.
    Header(u8),
}

impl core::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable => {
                core::fmt::Write::write_str(f, "configuration space is unavailable")
            }
            Self::Absent => core::fmt::Write::write_str(f, "the function is absent"),
            Self::Taken => core::fmt::Write::write_str(f, "the function is already claimed"),
            Self::NoRoom => core::fmt::Write::write_str(f, "the function registry is full"),
            Self::Exhausted => {
                core::fmt::Write::write_str(f, "function claim generations are exhausted")
            }
            Self::Header(layout) => write!(f, "header layout {layout} is unsupported"),
        }
    }
}

impl core::error::Error for ClaimError {}

/// Claim one present function after verifying its header layout.
///
/// Read-only enumeration remains address based. This is the sole door from an
/// address into configuration-write authority.
///
/// # Errors
///
/// See [`ClaimError`]. Every refusal leaves the function registry unchanged.
pub fn claim_function(at: Address) -> Result<FunctionClaim, ClaimError> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration {
        ports, functions, ..
    } = &mut *configuration;
    let (address, data) = ports.as_mut().ok_or(ClaimError::Unavailable)?;
    let at = at.function_base();
    Port::write(address, at.bits());
    let identity = Port::read(data);
    if pci_config::is_absent(identity as u16) {
        return Err(ClaimError::Absent);
    }
    let header_at = at.at(header::HEADER_TYPE).ok_or(ClaimError::Unavailable)?;
    Port::write(address, header_at.bits());
    let header = HeaderType::from_bits((Port::read(data) >> 16) as u8);
    let authority = functions.claim(at, header).map_err(|reason| match reason {
        FunctionClaimError::Taken => ClaimError::Taken,
        FunctionClaimError::NoRoom => ClaimError::NoRoom,
        FunctionClaimError::Exhausted => ClaimError::Exhausted,
        FunctionClaimError::Header(layout) => ClaimError::Header(layout),
        _ => ClaimError::Unavailable,
    })?;
    Ok(FunctionClaim {
        authority,
        identity,
    })
}

/// A refused function-authority release with the unchanged claim.
#[derive(Debug)]
#[must_use = "a refused function release returns its claim for retry"]
pub struct ReleaseFailure {
    reason: FunctionReleaseError,
    function: FunctionClaim,
}

impl ReleaseFailure {
    /// Recover the exact function authority for retry.
    pub fn into_function(self) -> FunctionClaim {
        self.function
    }
}

impl core::fmt::Display for ReleaseFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.reason {
            FunctionReleaseError::Active => {
                core::fmt::Write::write_str(f, "the function still has a configuration sublease")
            }
            FunctionReleaseError::Stale => {
                core::fmt::Write::write_str(f, "the function authority is foreign or stale")
            }
            _ => core::fmt::Write::write_str(f, "the function authority could not be released"),
        }
    }
}

impl core::error::Error for ReleaseFailure {}

/// Return function authority after every configuration sublease has settled.
///
/// # Errors
///
/// Returns the unchanged claim while command or BAR restoration authority is
/// live, or if the token is no longer this registry's exact generation.
pub fn release_function(function: FunctionClaim) -> Result<(), ReleaseFailure> {
    let FunctionClaim {
        authority,
        identity,
    } = function;
    let mut configuration = CONFIGURATION.lock_leaf();
    let functions = &mut configuration.functions;
    pci_config::release_function(functions, authority).map_err(|failure| ReleaseFailure {
        reason: failure.reason(),
        function: FunctionClaim {
            authority: failure.into_authority(),
            identity,
        },
    })
}

/// Admit DMA through freshly observed, function-bound NVMe registers, behind
/// a remapping domain established for that exact function (D-357).
///
/// # Errors
///
/// Refuses a stale BAR or command owner, a domain for another function, and
/// every configuration access or exact-readback failure without consuming the
/// command state.
pub fn admit_bus_master(
    authority: NvmeBusMasterAuthority<'_>,
    domain: DmaDomain,
    interrupts: InterruptIsolation,
    state: &mut CommandState,
) -> Result<(), EnableError> {
    let NvmeBusMasterAuthority { bar, _observed: _ } = authority;
    if interrupts.function() != domain.function() {
        return Err(EnableError::Stale);
    }
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    pci_config::admit_mapped_bus_master(
        functions,
        &bar.authority,
        domain.function(),
        state,
        |at, access| {
            let (address, data) = ports.as_mut()?;
            Port::write(address, at.bits());
            match access {
                CommandAccess::Read => Some(Port::read(data)),
                CommandAccess::Write(value) => {
                    Port::write(data, u32::from(value));
                    Some(0)
                }
            }
        },
    )
}

/// Disable bus mastering for a rejected DMA lease and remove BME from its
/// eventual restoration word only after exact readback.
pub fn disable_bus_master_for_retirement(
    function: &FunctionClaim,
    state: &mut CommandState,
) -> Result<(), EnableError> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    if !functions.holds(&function.authority) {
        return pci_config::disable_bus_master_for_retirement(
            functions,
            &function.authority,
            state,
            |_, _| None,
        );
    }
    pci_config::disable_bus_master_for_retirement(
        functions,
        &function.authority,
        state,
        |at, access| {
            let (address, data) = ports.as_mut()?;
            Port::write(address, at.bits());
            match access {
                CommandAccess::Read => Some(Port::read(data)),
                CommandAccess::Write(value) => {
                    Port::write(data, u32::from(value));
                    Some(0)
                }
            }
        },
    )
}

/// Claim a command sublease and keep one unit sibling unable to bus-master.
///
/// Successful restoration of the returned state preserves every captured
/// command bit except BME.
pub fn fence_bus_master(function: &FunctionClaim) -> Result<CommandState, EnableFailure> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    if !functions.holds(&function.authority) {
        return pci_config::fence_bus_master(functions, &function.authority, |_, _| None);
    }
    pci_config::fence_bus_master(functions, &function.authority, |at, access| {
        let (address, data) = ports.as_mut()?;
        Port::write(address, at.bits());
        match access {
            CommandAccess::Read => Some(Port::read(data)),
            CommandAccess::Write(value) => {
                Port::write(data, u32::from(value));
                Some(0)
            }
        }
    })
}

/// Re-read one exact command owner immediately before a unit-wide transition.
pub fn verify_bus_master_fence(
    function: &FunctionClaim,
    state: &CommandState,
) -> Result<(), EnableError> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    if !functions.holds(&function.authority) {
        return pci_config::verify_bus_master_fence(
            functions,
            &function.authority,
            state,
            |_, _| None,
        );
    }
    pci_config::verify_bus_master_fence(functions, &function.authority, state, |at, access| {
        let (address, data) = ports.as_mut()?;
        Port::write(address, at.bits());
        match access {
            CommandAccess::Read => Some(Port::read(data)),
            CommandAccess::Write(value) => {
                Port::write(data, u32::from(value));
                Some(0)
            }
        }
    })
}

/// Restore a captured command word as one verified transaction.
pub fn restore_command(
    function: &FunctionClaim,
    state: CommandState,
) -> Result<(), RestoreFailure> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    if !functions.holds(&function.authority) {
        return pci_config::restore_command(functions, &function.authority, state, |_, _| None);
    }
    pci_config::restore_command(functions, &function.authority, state, |at, access| {
        let (address, data) = ports.as_mut()?;
        Port::write(address, at.bits());
        match access {
            CommandAccess::Read => Some(Port::read(data)),
            CommandAccess::Write(value) => {
                Port::write(data, u32::from(value));
                Some(0)
            }
        }
    })
}

/// Finish a permanent command transition after exact readback.
pub fn commit_command(function: &FunctionClaim, state: CommandState) -> Result<(), RestoreFailure> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    if !functions.holds(&function.authority) {
        return pci_config::commit_command(functions, &function.authority, state, |_, _| None);
    }
    pci_config::commit_command(functions, &function.authority, state, |at, access| {
        let (address, data) = ports.as_mut()?;
        Port::write(address, at.bits());
        match access {
            CommandAccess::Read => Some(Port::read(data)),
            CommandAccess::Write(value) => {
                Port::write(data, u32::from(value));
                Some(0)
            }
        }
    })
}

/// Probe every BAR with decode and DMA off, retaining exact command restoration.
///
/// `pci-config` supplies the host-tested ordering and rollback; this adapter
/// only performs each requested cycle. The whole algorithm runs under one
/// hold of the pair, so nobody can see a probe value or replace CF8 between a
/// cycle's address and data instructions.
pub fn probe_bars(function: &FunctionClaim) -> Result<StagedProbe, ProbeFailure<()>> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    if !functions.holds(&function.authority) {
        return Err(ProbeFailure::unavailable(EnableError::Stale));
    }
    pci_config::probe_bars(functions, &function.authority, |at, access| {
        configuration_cycle(ports, at, access)
    })
}

/// Bind one CPU-admitted physical range to its exact probed memory BAR.
///
/// # Errors
///
/// Refuses a range whose base or length differs, a probe from another function
/// generation, or a BAR that already has a live mapping. The admitted range is
/// a proof this CPU can re-derive, so a refusal keeps nothing.
pub fn bind_memory_bar(
    function: &FunctionClaim,
    bars: &ProbedBars,
    index: usize,
    range: AdmittedDeviceRange,
) -> Result<AdmittedBar, BarBindError> {
    let matches = bars.window(index).is_some_and(|window| {
        matches!(
            window,
            Window {
                bar: Bar::Memory { address, .. },
                size,
            } if address == range.physical() && usize::try_from(size).ok() == Some(range.len())
        )
    });
    if !matches {
        return Err(BarBindError::Range);
    }

    let mut configuration = CONFIGURATION.lock_leaf();
    let authority = match pci_config::bind_memory_bar(
        &mut configuration.functions,
        &function.authority,
        bars,
        index,
    ) {
        Ok(authority) => authority,
        Err(reason) => return Err(BarBindError::Authority(reason)),
    };
    Ok(AdmittedBar { authority, range })
}

/// Enable memory decode for an already mapped, freshly unchanged BAR.
///
/// Returns the inaccessible mapping and any exact command restoration owner
/// after identity or command refusal.
pub fn enable_memory_bar(
    function: &FunctionClaim,
    bar: DisabledBar,
    state: &mut CommandState,
) -> BarEnableOutcome {
    let DisabledBar { authority, region } = bar;
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    let result = pci_config::enable_memory_bar(
        functions,
        &function.authority,
        &authority,
        state,
        |at, access| {
            let (address, data) = ports.as_mut()?;
            Port::write(address, at.bits());
            match access {
                BarEnableAccess::ReadBar(_) | BarEnableAccess::Command(CommandAccess::Read) => {
                    Some(Port::read(data))
                }
                BarEnableAccess::Command(CommandAccess::Write(value)) => {
                    Port::write(data, u32::from(value));
                    Some(0)
                }
            }
        },
    );
    match result {
        Ok(()) => BarEnableOutcome::Enabled(MappedBar { authority, region }),
        Err(reason) => BarEnableOutcome::Refused(BarEnableFailure {
            reason,
            bar: DisabledBar { authority, region },
        }),
    }
}

/// Return a BAR authority only after its device pages have been unmapped.
///
/// # Errors
///
/// Returns the unchanged authority if its function generation or BAR slot is
/// no longer live.
pub fn release_memory_bar(authority: BarAuthority) -> Result<(), BarAuthorityFailure> {
    pci_config::release_memory_bar(&mut CONFIGURATION.lock_leaf().functions, authority)
}

/// Return a BAR authority whose page-table mapping never began.
///
/// # Errors
///
/// Returns the authority refusal if its function generation is no longer live.
pub fn release_admitted_bar(admitted: AdmittedBar) -> Result<(), BarAuthorityFailure> {
    release_memory_bar(admitted.authority)
}

/// Retry an uncertain BAR restoration without releasing its function lease.
pub fn retry_probe(function: &FunctionClaim, retry: ProbeRetry) -> Result<(), ProbeFailure<()>> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    if !functions.holds(&function.authority) {
        return Err(ProbeFailure::with_retry(
            ProbeError::Unavailable(EnableError::Stale),
            retry,
        ));
    }
    pci_config::retry_probe(functions, &function.authority, retry, |at, access| {
        configuration_cycle(ports, at, access)
    })
}

/// Whether any offset of `at` is claimed by a live function authority.
#[cfg(feature = "selftest")]
pub fn is_claimed(at: Address) -> bool {
    CONFIGURATION.lock_leaf().functions.is_claimed(at)
}

/// Function authorities retained at this instant.
#[cfg(feature = "selftest")]
pub fn claims() -> usize {
    CONFIGURATION.lock_leaf().functions.len()
}

fn configuration_cycle(
    ports: &mut Option<(Port<u32>, Port<u32>)>,
    function: Address,
    access: ProbeAccess,
) -> Result<u32, ()> {
    let (address, data) = ports.as_mut().ok_or(())?;
    let (offset, write) = match access {
        ProbeAccess::ReadCommand => (header::COMMAND, None),
        ProbeAccess::WriteCommand(value) => (header::COMMAND, Some(u32::from(value))),
        ProbeAccess::ReadBar(index) => (header::BAR0 + index as u16 * 4, None),
        ProbeAccess::WriteBar(index, value) => (header::BAR0 + index as u16 * 4, Some(value)),
    };
    let register = function.at(offset).ok_or(())?;
    Port::write(address, register.bits());
    match write {
        Some(value) => {
            Port::write(data, value);
            Ok(0)
        }
        None => Ok(Port::read(data)),
    }
}
