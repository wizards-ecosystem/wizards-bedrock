#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

mod bar;
mod claims;
mod command_state;
mod fence;
mod memory;
mod probe;
mod topology;

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use claims::FunctionLease;
use command_state::{CommandPhase, CommandProbe};

pub use bar::Bar;
use bar::{MemoryWidth, memory_window_size};
pub use claims::{FunctionAuthority, FunctionClaimError, FunctionClaims};
pub use command_state::{
    CommandAccess, CommandState, EnableError, EnableFailure, FunctionReleaseError,
    FunctionReleaseFailure, RestoreFailure, admit_bridge_bus_master, commit_command,
    disable_bus_master_for_retirement, fence_bus_master, release_function, restore_command,
    verify_bus_master_fence,
};
pub use fence::{
    FUNCTION_AUTHORITY_LIMIT, HandoffAccess, HandoffFenceError, HandoffFencePolicy,
    HandoffFenceReport, ISA_DMA_PENDING_REQUESTS, fence_dma_handoff,
};
pub use memory::{
    BarAuthority, BarAuthorityError, BarAuthorityFailure, BarEnableAccess, BarEnableError,
    BarIdentityError, admit_mapped_bus_master, bind_memory_bar, enable_memory_bar,
    release_memory_bar,
};
pub use probe::{
    ProbeAccess, ProbeError, ProbeFailure, ProbeRetry, ProbedBars, StagedProbe, Window, probe_bars,
    retry_probe,
};
pub use topology::{BridgeRange, PciTopology, TopologyError};

pub const DEVICES: u8 = 32;

pub const FUNCTIONS: u8 = 8;

pub const HEADER_BYTES: u16 = 256;

pub const BARS: usize = 6;

// A type 1 header has two BARs; its third slot holds bus numbers.
pub(crate) const BRIDGE_BARS: usize = 2;

pub mod header {
    pub const VENDOR: u16 = 0x00;
    pub const COMMAND: u16 = 0x04;
    pub const CLASS: u16 = 0x08;
    pub const HEADER_TYPE: u16 = 0x0C;
    pub const BAR0: u16 = 0x10;
    pub const BUS_NUMBERS: u16 = 0x18;
}

mod command {
    pub(crate) const IO_SPACE: u16 = 1;
    pub(crate) const MEMORY_SPACE: u16 = 1 << 1;
    pub(crate) const BUS_MASTER: u16 = 1 << 2;
}

const PROBE_DECODE: u16 = command::IO_SPACE | command::MEMORY_SPACE;
const PROBE_DISABLED: u16 = PROBE_DECODE | command::BUS_MASTER;

static NEXT_COMMAND_REGISTRY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Address(u32);

impl Address {
    const ENABLE: u32 = 1 << 31;

    // A misaligned offset would silently read its containing dword.
    pub const fn new(bus: u8, device: u8, function: u8, offset: u16) -> Option<Self> {
        if device >= DEVICES || function >= FUNCTIONS {
            return None;
        }
        if offset >= HEADER_BYTES || !offset.is_multiple_of(4) {
            return None;
        }
        Some(Self(
            Self::ENABLE
                | ((bus as u32) << 16)
                | ((device as u32) << 11)
                | ((function as u32) << 8)
                | offset as u32,
        ))
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn at(self, offset: u16) -> Option<Self> {
        if offset >= HEADER_BYTES || !offset.is_multiple_of(4) {
            return None;
        }
        Some(Self((self.0 & !0xFF) | offset as u32))
    }

    #[must_use]
    pub const fn function_base(self) -> Self {
        Self(self.0 & !0xFF)
    }

    pub const fn bus(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub const fn device(self) -> u8 {
        ((self.0 >> 11) & 0x1F) as u8
    }

    pub const fn function(self) -> u8 {
        ((self.0 >> 8) & 0x07) as u8
    }
}

impl fmt::Display for Address {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:02x}:{:02x}.{}",
            self.bus(),
            self.device(),
            self.function()
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Class {
    pub class: u8,
    pub subclass: u8,
    pub interface: u8,
}

impl Class {
    pub const fn from_bits(bits: u32) -> Self {
        Self {
            interface: (bits >> 8) as u8,
            subclass: (bits >> 16) as u8,
            class: (bits >> 24) as u8,
        }
    }

    // `01:08:01` is NVMHCI, which this driver would misprogram.
    pub const fn is_nvme(self) -> bool {
        self.class == 0x01 && self.subclass == 0x08 && self.interface == 0x02
    }

    pub const fn is_bridge(self) -> bool {
        self.class == 0x06 && self.subclass == 0x04
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HeaderType(u8);

impl HeaderType {
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn is_multifunction(self) -> bool {
        self.0 & 0x80 != 0
    }

    pub const fn layout(self) -> u8 {
        self.0 & 0x7F
    }

    pub const fn is_bridge(self) -> bool {
        self.layout() == 1
    }

    pub const fn bar_registers(self) -> Option<usize> {
        match self.layout() {
            0 => Some(BARS),
            1 => Some(BRIDGE_BARS),
            _ => None,
        }
    }
}

#[derive(Debug)]
#[must_use = "qualified non-requester evidence must constrain requester inventory"]
pub struct QualifiedNonRequester {
    at: Address,
    identity: u32,
    class: u32,
    command: u16,
}

#[derive(Debug)]
#[must_use = "quiesced requester evidence must reach the topology snapshot"]
pub struct QuiescedIsaRequester {
    at: Address,
    identity: u32,
    class: u32,
    command: u16,
}

impl QuiescedIsaRequester {
    pub const fn address(&self) -> Address {
        self.at
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct QuiescedRequester {
    at: Address,
    identity: u32,
    command: u16,
}

impl QuiescedRequester {
    pub const fn address(&self) -> Address {
        self.at
    }

    pub const fn identity(&self) -> u32 {
        self.identity
    }

    pub const fn command(&self) -> u16 {
        self.command
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FunctionObservation {
    pub at: Address,
    pub identity: u32,
    pub class_bits: u32,
    pub command: u16,
    pub class: Class,
    pub header: HeaderType,
    pub bus_numbers: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CommandLease {
    registry: u64,
    function: FunctionLease,
    generation: u64,
}

impl CommandLease {
    const fn command_address(self) -> Address {
        Address(self.function.at.0 | header::COMMAND as u32)
    }
}

#[derive(PartialEq, Eq, Debug)]
struct CommandLeases<const N: usize> {
    slots: [Option<CommandLease>; N],
    identity: u64,
    generation: u64,
}

impl<const N: usize> CommandLeases<N> {
    const fn new() -> Self {
        Self {
            slots: [None; N],
            identity: 0,
            generation: 0,
        }
    }

    fn reserve(&mut self, authority: &FunctionAuthority) -> Result<CommandLease, EnableError> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|lease| lease.function == authority.lease)
        {
            return Err(EnableError::Leased);
        }
        let Some(room) = self.slots.iter().position(Option::is_none) else {
            return Err(EnableError::NoRoom);
        };
        let registry = if self.identity == 0 {
            let identity = NEXT_COMMAND_REGISTRY
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    current.checked_add(1)
                })
                .map_err(|_| EnableError::Exhausted)?;
            self.identity = identity;
            identity
        } else {
            self.identity
        };
        let Some(generation) = self.generation.checked_add(1) else {
            return Err(EnableError::Exhausted);
        };
        self.generation = generation;
        let lease = CommandLease {
            registry,
            function: authority.lease,
            generation,
        };
        self.slots[room] = Some(lease);
        Ok(lease)
    }

    fn holds(&self, authority: &FunctionAuthority, lease: CommandLease) -> bool {
        self.holds_function(authority.lease, lease)
    }

    fn holds_function(&self, function: FunctionLease, lease: CommandLease) -> bool {
        lease.function == function && self.slots.iter().flatten().any(|held| *held == lease)
    }

    fn release(&mut self, lease: CommandLease) -> bool {
        let Some(slot) = self.slots.iter_mut().find(|held| **held == Some(lease)) else {
            return false;
        };
        *slot = None;
        true
    }

    fn begin_probe(&mut self, authority: &FunctionAuthority) -> Result<CommandProbe, EnableError> {
        self.reserve(authority).map(CommandProbe)
    }

    fn finish_probe(
        &mut self,
        authority: &FunctionAuthority,
        probe: CommandProbe,
    ) -> Result<(), CommandProbe> {
        if self.holds(authority, probe.0) && self.release(probe.0) {
            Ok(())
        } else {
            Err(probe)
        }
    }

    fn stage_probe(
        &self,
        authority: &FunctionAuthority,
        probe: CommandProbe,
        before: u16,
    ) -> Result<CommandState, CommandProbe> {
        if !self.holds(authority, probe.0) {
            return Err(probe);
        }
        Ok(CommandState {
            lease: probe.0,
            before,
            installed: before & !PROBE_DISABLED,
            phase: CommandPhase::Installed,
        })
    }

    fn has_function(&self, authority: &FunctionAuthority) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|lease| lease.function == authority.lease)
    }
}

impl<const N: usize> Default for CommandLeases<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub const fn is_absent(vendor: u16) -> bool {
    vendor == 0xFFFF
}

pub(crate) const fn primary_bus(bits: u32) -> u8 {
    bits as u8
}

pub const fn secondary_bus(bits: u32) -> u8 {
    (bits >> 8) as u8
}

pub(crate) const fn subordinate_bus(bits: u32) -> u8 {
    (bits >> 16) as u8
}

#[cfg(test)]
mod tests;
