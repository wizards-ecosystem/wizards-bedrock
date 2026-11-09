//! PCI configuration-space addressing and BAR decoding.
//!
//! `wost` owns the CF8/CFC instructions; this crate owns their host-tested
//! arithmetic. Offsets are dword-aligned, BAR masks are complemented in their
//! register width, and 64-bit BARs consume two slots. CF8/CFC is the common
//! mechanism on the QEMU i440fx and VMware 440BX targets (`012` §2).
//!
//! # Examples
//!
//! ```
//! use pci_config::{Address, header};
//!
//! // Bus 0, device 2, function 0, the vendor and device word.
//! let at = Address::new(0, 2, 0, header::VENDOR).expect("a real address");
//! assert_eq!(at.bits(), 0x8000_1000);
//!
//! // The enable bit is what makes CF8 a configuration cycle rather than a
//! // write to whatever else lives at that port.
//! assert!(at.bits() & (1 << 31) != 0);
//! ```

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

use core::sync::atomic::{AtomicU64, Ordering};

use claims::FunctionLease;
use command_state::CommandPhase;

pub use bar::{Bar, MemoryWidth, memory_window_size};
pub use claims::{FunctionAuthority, FunctionClaimError, FunctionClaims};
pub use command_state::{
    CommandAccess, CommandProbe, CommandState, EnableError, EnableFailure, FunctionReleaseError,
    FunctionReleaseFailure, RestoreFailure, commit_command, disable_bus_master_for_retirement,
    enable_memory_space, fence_bus_master, release_function, restore_command,
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

/// Devices on one bus.
pub const DEVICES: u8 = 32;

/// Functions in one device.
pub const FUNCTIONS: u8 = 8;

/// Bytes of the configuration header this addressing reaches.
///
/// The original mechanism sees 256; the extended space ECAM adds is not
/// reachable through CF8/CFC at all, and a driver that needs it needs ECAM.
pub const HEADER_BYTES: u16 = 256;

/// Base address registers a type 0 header has.
pub const BARS: usize = 6;

/// Base address registers a type 1 header has.
///
/// The third slot in a bridge header is its bus-number register, not a BAR.
pub const BRIDGE_BARS: usize = 2;

/// Byte offsets in the configuration header.
///
/// Type 0 (a device) and type 1 (a bridge) share everything up to `0x0F`, and
/// the header type at `0x0E` is what says which the rest is.
pub mod header {
    /// Vendor id, and device id in the high half. `0xFFFF` means nothing is
    /// there — the bus returns all ones for an absent function.
    pub const VENDOR: u16 = 0x00;
    /// Command register. Bit 0 is I/O space, bit 1 is memory space, and bit 2
    /// is bus master.
    pub const COMMAND: u16 = 0x04;
    /// Revision in the low byte, then prog-if, subclass and class.
    pub const CLASS: u16 = 0x08;
    /// Cache line size, latency, header type, BIST.
    pub const HEADER_TYPE: u16 = 0x0C;
    /// The first base address register.
    pub const BAR0: u16 = 0x10;
    /// A bridge's primary, secondary and subordinate bus numbers.
    pub const BUS_NUMBERS: u16 = 0x18;
}

/// Command-register bits this kernel writes.
pub mod command {
    /// Respond to I/O-space accesses.
    pub const IO_SPACE: u16 = 1;
    /// Respond to memory-space accesses.
    pub const MEMORY_SPACE: u16 = 1 << 1;
    /// **Bus master.** Without it the controller cannot DMA, queues never
    /// advance, and there is no device error to read (`012` §2).
    pub const BUS_MASTER: u16 = 1 << 2;
}

const PROBE_DECODE: u16 = command::IO_SPACE | command::MEMORY_SPACE;
const PROBE_DISABLED: u16 = PROBE_DECODE | command::BUS_MASTER;

/// A configuration-space address, ready for the address port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address(u32);

impl Address {
    /// Bit 31: this is a configuration cycle and not something else.
    const ENABLE: u32 = 1 << 31;

    /// Build an address, or refuse one that cannot be encoded.
    ///
    /// # Errors
    ///
    /// `None` for a device above 31, a function above 7, an offset at or past
    /// [`HEADER_BYTES`], or an offset that is not a multiple of four — the
    /// last because configuration space is read a dword at a time and a
    /// misaligned offset does not fault, it quietly reads the dword containing
    /// it.
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

    /// The word to write to the address port.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The same function at a different offset.
    ///
    /// The common move — walk a header — expressed so the bus, device and
    /// function are not re-encoded and cannot be re-encoded differently.
    pub const fn at(self, offset: u16) -> Option<Self> {
        if offset >= HEADER_BYTES || !offset.is_multiple_of(4) {
            return None;
        }
        Some(Self((self.0 & !0xFF) | offset as u32))
    }

    /// The identity of this function, with no register offset attached.
    ///
    /// Device ownership is a bus/device/function claim. Treating two offsets
    /// in the same header as different identities would let two drivers claim
    /// the same function.
    #[must_use]
    pub const fn function_base(self) -> Self {
        Self(self.0 & !0xFF)
    }

    /// Which bus this names.
    pub const fn bus(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// Which device on that bus.
    pub const fn device(self) -> u8 {
        ((self.0 >> 11) & 0x1F) as u8
    }

    /// Which function of that device.
    pub const fn function(self) -> u8 {
        ((self.0 >> 8) & 0x07) as u8
    }
}

/// What a function says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Class {
    /// The broad kind: `0x01` is mass storage, `0x06` a bridge.
    pub class: u8,
    /// Within that kind: `0x08` under mass storage is a non-volatile memory
    /// controller.
    pub subclass: u8,
    /// The programming interface: `0x02` under `01:08` is NVM Express.
    pub interface: u8,
    /// Silicon revision, which decides nothing here and is read because it
    /// shares the word.
    pub revision: u8,
}

impl Class {
    /// Decode the class word at [`header::CLASS`].
    pub const fn from_bits(bits: u32) -> Self {
        Self {
            revision: bits as u8,
            interface: (bits >> 8) as u8,
            subclass: (bits >> 16) as u8,
            class: (bits >> 24) as u8,
        }
    }

    /// Whether this is an NVM Express controller.
    ///
    /// All three bytes, not two: `01:08` is "non-volatile memory controller"
    /// and `01:08:01` is NVMHCI, a different and older interface this driver
    /// would misprogram.
    pub const fn is_nvme(self) -> bool {
        self.class == 0x01 && self.subclass == 0x08 && self.interface == 0x02
    }

    /// Whether this is a PCI-to-PCI bridge, and so a bus to descend into.
    ///
    /// **Enumeration must recurse.** VMware places functions behind
    /// `pciBridge0`–`pciBridge7`, so a scan of bus zero alone may never find
    /// the controller (`012` §2).
    pub const fn is_bridge(self) -> bool {
        self.class == 0x06 && self.subclass == 0x04
    }
}

/// What the header-type byte says about a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderType(u8);

impl HeaderType {
    /// Decode the byte at [`header::HEADER_TYPE`] + 2.
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Whether the device has functions past zero.
    ///
    /// Bit 7. A walk that ignores it and probes all eight anyway is not wrong,
    /// only slower; a walk that *reads* it and gets the bit wrong misses every
    /// function of a multi-function device.
    pub const fn is_multifunction(self) -> bool {
        self.0 & 0x80 != 0
    }

    /// The layout of the rest of the header: 0 a device, 1 a bridge.
    pub const fn layout(self) -> u8 {
        self.0 & 0x7F
    }

    /// Whether the rest of the header is a bridge's.
    pub const fn is_bridge(self) -> bool {
        self.layout() == 1
    }

    /// BAR register slots defined by this header layout.
    ///
    /// `None` for layouts this kernel cannot safely size: their registers do
    /// not have either the type-0 device or type-1 bridge meaning.
    pub const fn bar_registers(self) -> Option<usize> {
        match self.layout() {
            0 => Some(BARS),
            1 => Some(BRIDGE_BARS),
            _ => None,
        }
    }
}

/// Exact platform evidence that one present function is not a DMA requester.
///
/// Construction is private to [`HandoffFencePolicy`], so topology code cannot
/// remove an arbitrary function from the requester inventory.
#[derive(Debug)]
#[must_use = "qualified non-requester evidence must constrain requester inventory"]
pub struct QualifiedNonRequester {
    at: Address,
    identity: u32,
    class: u32,
    command: u16,
}

/// Exact platform evidence that one requester's bus mastering is held off at
/// its DMA engines because its command bit 2 cannot be cleared.
///
/// Construction is private to the entry fence, after the exact VMware PIIX4
/// profile matched and both 8237 controllers were masked with no request
/// pending. The function remains a requester in every inventory.
#[derive(Debug)]
#[must_use = "quiesced requester evidence must reach the topology snapshot"]
pub struct QuiescedIsaRequester {
    at: Address,
    identity: u32,
    class: u32,
    command: u16,
}

impl QuiescedIsaRequester {
    /// Exact function whose entry-time quiesce minted this authority.
    pub const fn address(&self) -> Address {
        self.at
    }
}

/// The sealed snapshot's record of the one entry-quiesced ISA requester.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuiescedRequester {
    at: Address,
    identity: u32,
    command: u16,
}

impl QuiescedRequester {
    /// Exact function the entry fence quiesced.
    pub const fn address(&self) -> Address {
        self.at
    }

    /// Identity a later fence must still read.
    pub const fn identity(&self) -> u32 {
        self.identity
    }

    /// Command word a later fence must still read.
    pub const fn command(&self) -> u16 {
        self.command
    }
}

/// One untrusted later observation checked against non-requester authority.
///
/// Construction proves nothing by itself. [`PciTopology::observe_non_requester`]
/// consumes the earlier entry-fence authority and compares every security-
/// relevant field before excluding the function from requester inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionObservation {
    at: Address,
    identity: u32,
    class_bits: u32,
    command: u16,
    class: Class,
    header: HeaderType,
    bus_numbers: Option<u32>,
}

impl FunctionObservation {
    /// Gather the independently read fields of one present function.
    pub const fn new(
        at: Address,
        identity: u32,
        class_bits: u32,
        command: u16,
        class: Class,
        header: HeaderType,
        bus_numbers: Option<u32>,
    ) -> Self {
        Self {
            at,
            identity,
            class_bits,
            command,
            class,
            header,
            bus_numbers,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, PartialEq, Eq)]
struct CommandLeases<const N: usize> {
    slots: [Option<CommandLease>; N],
    identity: u64,
    generation: u64,
}

static NEXT_COMMAND_REGISTRY: AtomicU64 = AtomicU64::new(1);

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

    /// Finish a BAR probe whose command word was restored exactly.
    ///
    /// # Errors
    ///
    /// Returns the unchanged probe token when it did not come from this
    /// registry or no longer names its live generation.
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
        &mut self,
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

/// Whether a vendor word means "nothing is here".
///
/// An absent function does not fail a configuration read; the bus returns all
/// ones, so `0xFFFF` is the answer and not an error. A driver that treats it
/// as a vendor id enumerates 256 buses of imaginary devices.
pub const fn is_absent(vendor: u16) -> bool {
    vendor == 0xFFFF
}

/// A bridge's primary bus number, out of the word at [`header::BUS_NUMBERS`].
pub const fn primary_bus(bits: u32) -> u8 {
    bits as u8
}

/// A bridge's secondary bus number, out of the word at [`header::BUS_NUMBERS`].
///
/// The bus behind it, which is the number a recursive enumeration descends to.
pub const fn secondary_bus(bits: u32) -> u8 {
    (bits >> 8) as u8
}

/// A bridge's subordinate bus number, out of the word at [`header::BUS_NUMBERS`].
///
/// The highest bus behind it: the bridge forwards to every bus from its
/// secondary through this one.
pub const fn subordinate_bus(bits: u32) -> u8 {
    (bits >> 16) as u8
}

#[cfg(test)]
mod tests;
