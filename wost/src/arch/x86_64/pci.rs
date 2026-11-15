use nvme::{ControllerStateError, DisabledRegisters, register};
use pci_config::{
    Address, Bar, BarAuthority, BarAuthorityError, BarAuthorityFailure, BarEnableAccess,
    BarEnableError as ConfigBarEnableError, CommandAccess, CommandState, EnableError,
    EnableFailure, FunctionAuthority, FunctionClaimError, FunctionClaims, FunctionReleaseError,
    HandoffAccess, HandoffFenceError, HandoffFencePolicy, HandoffFenceReport, HeaderType,
    ProbeAccess, ProbeError, ProbeFailure, ProbeRetry, ProbedBars, RestoreFailure, StagedProbe,
    Window, header,
};
use register_bits::u16_halves;

use crate::arch::current::paging::{AdmittedDeviceRange, DeviceMapping, ReleasableDevice};
use crate::arch::current::vtd::{DmaDomain, InterruptIsolation};
use crate::mmio::MmioError;
use crate::sync::SpinLock;

use super::port::{self, Port, PortBlock};

const FUNCTION_CLAIMS: usize = pci_config::FUNCTION_AUTHORITY_LIMIT;

#[derive(Debug)]
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

static CONFIGURATION: SpinLock<Configuration> = SpinLock::new(Configuration::new());

pub(crate) fn force_unlock(panic: &crate::panic::PanicToken) {
    CONFIGURATION.force_unlock(panic);
    ISA_DMA.force_unlock(panic);
}

#[must_use = "a refused PCI initialization left configuration space unavailable"]
pub fn init() -> bool {
    if is_available() {
        return true;
    }
    // Claim before taking this module's leaf guard to preserve lock order.
    let Some(pair) = port::pci_configuration() else {
        return is_available();
    };

    let mut configuration = CONFIGURATION.lock_leaf();
    if configuration.ports.is_none() {
        configuration.ports = Some(pair);
    }
    true
}

pub fn is_available() -> bool {
    CONFIGURATION.lock_leaf().ports.is_some()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DmaHandoffFenceError {
    Unavailable,
    Busy,
    Fence(HandoffFenceError),
}

impl core::fmt::Display for DmaHandoffFenceError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("PCI configuration is unavailable"),
            Self::Busy => {
                formatter.write_str("PCI function authority exists before the handoff fence")
            }
            Self::Fence(reason) => write!(formatter, "{reason}"),
        }
    }
}

impl core::error::Error for DmaHandoffFenceError {}

pub fn fence_dma_handoff(
    policy: HandoffFencePolicy,
) -> Result<HandoffFenceReport, DmaHandoffFenceError> {
    // Claim before taking this module's leaf guard to preserve lock order; unowned 8237s refuse.
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

// Owned for the boot once claimed, so no later owner can unmask a channel.
static ISA_DMA: SpinLock<Option<IsaDma>> = SpinLock::new(None);

#[derive(Debug)]
struct IsaDma {
    first: PortBlock<16>,
    second: PortBlock<32>,
}

// The second 8237 decodes its registers at even ports, so its offsets are doubled.
mod isa_dma {
    pub(super) const FIRST_STATUS: usize = 0x08;
    pub(super) const FIRST_MASTER_CLEAR: usize = 0x0D;
    pub(super) const FIRST_MASK_ALL: usize = 0x0F;
    pub(super) const SECOND_STATUS: usize = 0x10;
    pub(super) const SECOND_MASTER_CLEAR: usize = 0x1A;
    pub(super) const SECOND_MASK_ALL: usize = 0x1E;
    pub(super) const EVERY_CHANNEL: u8 = 0x0F;
}

impl IsaDma {
    // A master clear masks every channel; the explicit mask follows as masks cannot be read.
    fn quiesce(&mut self) -> u32 {
        self.first.write::<u8>(isa_dma::FIRST_MASTER_CLEAR, 0);
        self.second.write::<u8>(isa_dma::SECOND_MASTER_CLEAR, 0);
        self.reassert()
    }

    fn reassert(&mut self) -> u32 {
        self.first
            .write::<u8>(isa_dma::FIRST_MASK_ALL, isa_dma::EVERY_CHANNEL);
        self.second
            .write::<u8>(isa_dma::SECOND_MASK_ALL, isa_dma::EVERY_CHANNEL);
        let first = self.first.read::<u8>(isa_dma::FIRST_STATUS);
        let second = self.second.read::<u8>(isa_dma::SECOND_STATUS);
        u32::from(first) | u32::from(second) << 8
    }
}

pub fn isa_dma_quiet() -> bool {
    ISA_DMA.lock_leaf().as_mut().is_some_and(|controllers| {
        controllers.reassert() & pci_config::ISA_DMA_PENDING_REQUESTS == 0
    })
}

// An absent function answers all ones, which `pci_config::is_absent` reads; never an error.
pub fn read(at: Address) -> Option<u32> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let (address, data) = configuration.ports.as_mut()?;
    Port::write(address, at.bits());
    Some(Port::read(data))
}

#[derive(Debug)]
#[must_use = "dropping PCI function authority permanently retains its claim"]
pub struct FunctionClaim {
    authority: FunctionAuthority,
    identity: u32,
}

#[derive(Debug)]
pub struct AdmittedBar {
    pub(super) authority: BarAuthority,
    pub(super) range: AdmittedDeviceRange,
}

impl AdmittedBar {
    pub const fn physical_admission(&self) -> (u64, u64, u8) {
        (
            self.range.page_start(),
            self.range.page_end(),
            self.range.physical_bits(),
        )
    }
}

#[derive(Debug)]
pub struct BarMapping {
    pub(super) authority: BarAuthority,
    pub(super) mapping: DeviceMapping,
}

#[derive(Debug)]
pub struct DisabledBar {
    authority: BarAuthority,
    region: ReleasableDevice,
}

#[derive(Debug)]
pub struct MappedBar {
    authority: BarAuthority,
    region: ReleasableDevice,
}

#[derive(Debug)]
#[must_use = "NVMe quiescence authority must be consumed by DMA admission"]
pub struct NvmeBusMasterAuthority<'bar> {
    bar: &'bar MappedBar,
    _observed: DisabledRegisters,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NvmeObservationError {
    WrongBar(usize),
    Access(MmioError),
    State(ControllerStateError),
}

impl core::fmt::Display for NvmeObservationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::WrongBar(index) => write!(formatter, "NVMe registers came from BAR{index}"),
            Self::Access(reason) => write!(formatter, "NVMe register access: {reason}"),
            Self::State(reason) => write!(formatter, "{reason}"),
        }
    }
}

impl core::error::Error for NvmeObservationError {}

#[derive(Debug)]
pub struct ReleasableBar {
    pub(super) authority: BarAuthority,
    pub(super) region: ReleasableDevice,
}

impl BarMapping {
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
    pub fn into_releasable(self) -> ReleasableBar {
        ReleasableBar {
            authority: self.authority,
            region: self.region,
        }
    }
}

impl MappedBar {
    pub fn read_u32(&self, offset: usize) -> Result<u32, MmioError> {
        ReleasableDevice::read_u32(&self.region, offset)
    }

    pub fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), MmioError> {
        ReleasableDevice::write_u32(&mut self.region, offset, value)
    }

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

    pub fn into_releasable(self) -> ReleasableBar {
        ReleasableBar {
            authority: self.authority,
            region: self.region,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarBindError {
    Range,
    Authority(BarAuthorityError),
}

impl core::fmt::Display for BarBindError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Range => formatter.write_str("the admitted range differs from the BAR"),
            Self::Authority(reason) => write!(formatter, "BAR authority: {reason}"),
        }
    }
}

impl core::error::Error for BarBindError {}

#[derive(Debug)]
#[must_use = "a refused decode transition retains mapping authority"]
pub struct BarEnableFailure {
    reason: ConfigBarEnableError,
    bar: DisabledBar,
}

#[derive(Debug)]
#[must_use = "a refused decode transition retains mapping and restoration authority"]
pub enum BarEnableOutcome {
    Enabled(MappedBar),
    Refused(BarEnableFailure),
}

impl BarEnableFailure {
    pub const fn reason(&self) -> ConfigBarEnableError {
        self.reason
    }

    pub fn into_bar(self) -> DisabledBar {
        self.bar
    }
}

impl FunctionClaim {
    pub const fn address(&self) -> Address {
        self.authority.address()
    }

    pub const fn identity(&self) -> (u16, u16) {
        u16_halves(self.identity)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClaimError {
    Unavailable,
    Absent,
    Taken,
    NoRoom,
    Exhausted,
    Header(u8),
}

impl core::fmt::Display for ClaimError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("configuration space is unavailable"),
            Self::Absent => formatter.write_str("the function is absent"),
            Self::Taken => formatter.write_str("the function is already claimed"),
            Self::NoRoom => formatter.write_str("the function registry is full"),
            Self::Exhausted => formatter.write_str("function claim generations are exhausted"),
            Self::Header(layout) => write!(formatter, "header layout {layout} is unsupported"),
        }
    }
}

impl core::error::Error for ClaimError {}

pub fn claim_function(at: Address) -> Result<FunctionClaim, ClaimError> {
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration {
        ports, functions, ..
    } = &mut *configuration;
    let (address, data) = ports.as_mut().ok_or(ClaimError::Unavailable)?;
    let at = at.function_base();
    Port::write(address, at.bits());
    let identity = Port::read(data);
    if pci_config::is_absent(u16_halves(identity).0) {
        return Err(ClaimError::Absent);
    }
    let header_at = at.at(header::HEADER_TYPE).ok_or(ClaimError::Unavailable)?;
    Port::write(address, header_at.bits());
    let header = HeaderType::from_bits(Port::read(data).to_le_bytes()[2]);
    let authority = functions.claim(at, header).map_err(|reason| match reason {
        FunctionClaimError::Taken => ClaimError::Taken,
        FunctionClaimError::NoRoom => ClaimError::NoRoom,
        FunctionClaimError::Exhausted => ClaimError::Exhausted,
        FunctionClaimError::Header(layout) => ClaimError::Header(layout),
    })?;
    Ok(FunctionClaim {
        authority,
        identity,
    })
}

#[derive(Debug)]
#[must_use = "a refused function release returns its claim for retry"]
pub struct ReleaseFailure {
    reason: FunctionReleaseError,
    function: FunctionClaim,
}

impl ReleaseFailure {
    pub fn into_function(self) -> FunctionClaim {
        self.function
    }
}

impl core::fmt::Display for ReleaseFailure {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.reason {
            FunctionReleaseError::Active => {
                formatter.write_str("the function still has a configuration sublease")
            }
            FunctionReleaseError::Stale => {
                formatter.write_str("the function authority is foreign or stale")
            }
        }
    }
}

impl core::error::Error for ReleaseFailure {}

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

pub fn admit_bus_master(
    authority: NvmeBusMasterAuthority<'_>,
    domain: DmaDomain<'_>,
    interrupts: InterruptIsolation<'_>,
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

// A bridge with BME clear does not forward its secondary side's DMA upstream.
pub fn admit_bridge_bus_master(
    bridge: &FunctionClaim,
    domain: &DmaDomain<'_>,
    interrupts: &InterruptIsolation<'_>,
    state: &mut CommandState,
) -> Result<(), EnableError> {
    if interrupts.function() != domain.function() {
        return Err(EnableError::Stale);
    }
    let mut configuration = CONFIGURATION.lock_leaf();
    let Configuration { ports, functions } = &mut *configuration;
    pci_config::admit_bridge_bus_master(
        functions,
        &bridge.authority,
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

// The whole probe runs under one hold of the pair, so no one sees a probe value.
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

// Callers return a BAR authority only after its device pages are unmapped.
pub fn release_memory_bar(authority: BarAuthority) -> Result<(), BarAuthorityFailure> {
    pci_config::release_memory_bar(&mut CONFIGURATION.lock_leaf().functions, authority)
}

pub fn release_admitted_bar(admitted: AdmittedBar) -> Result<(), BarAuthorityFailure> {
    release_memory_bar(admitted.authority)
}

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
