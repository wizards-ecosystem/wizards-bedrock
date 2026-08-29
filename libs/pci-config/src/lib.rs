//! Configuration-space addressing, BAR decoding and the class codes this
//! kernel looks for.
//!
//! The arithmetic of talking to PCI, where `cargo test` and a mutation can
//! reach it. `wost` keeps the two ports and the `in`/`out` instructions, and
//! nothing else.
//!
//! **CF8/CFC rather than ECAM**, and the reason is not preference. ECAM needs
//! an MCFG table, and QEMU's i440fx publishes none while VMware emulates a
//! 440BX — so the mechanism guaranteed present on *both* boot targets is the
//! one from 1992. It reaches everything a read-only driver needs, and an ECAM
//! path later is purely additive (`012` §2).
//!
//! Three things here are worth more than their size:
//!
//! - **The offset's low two bits must be zero.** Configuration space is read a
//!   dword at a time; a misaligned offset does not fault, it silently reads the
//!   dword containing it and the caller shifts the wrong bytes out.
//! - **A BAR's size is learned by writing all ones and reading back**, and the
//!   answer is the two's complement of the mask — *in the register's own
//!   width*. Getting it wrong maps too little, which fails at the far end of
//!   the register file long after the mapping, or too much, which overlaps the
//!   next device.
//! - **A 64-bit BAR occupies two slots**, so a walk that steps one at a time
//!   reads the high half as a second base address and finds a device at a
//!   nonsense address.
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
    /// advance, and *there is no error anywhere to read* (`012` §2). It is the
    /// single most expensive bit to forget in this whole subsystem.
    pub const BUS_MASTER: u16 = 1 << 2;
}

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

    /// Which offset within its header.
    pub const fn offset(self) -> u16 {
        (self.0 & 0xFF) as u16
    }
}

/// A bounded registry of claimed PCI functions.
///
/// The fallible preparation closure runs before the registry's commit point.
/// This makes an unchanged registry on every probe and port-claim error a
/// property of the operation rather than statement ordering in its caller.
#[derive(Debug, PartialEq, Eq)]
pub struct ClaimRegistry<const N: usize> {
    slots: [Option<Address>; N],
}

impl<const N: usize> ClaimRegistry<N> {
    /// Create an empty function registry.
    pub const fn new() -> Self {
        Self { slots: [None; N] }
    }

    /// Prepare and claim one function, or change nothing.
    ///
    /// The address is canonicalized before duplicate lookup. `prepare` may do
    /// all fallible configuration and resource work; the registry records the
    /// function only after that work returns a value.
    ///
    /// # Errors
    ///
    /// [`RegistryError::Taken`] if any offset of the function is held,
    /// [`RegistryError::NoRoom`] if the fixed registry is full, or
    /// [`RegistryError::Prepare`] with the closure's error. Every refusal
    /// leaves all slots unchanged.
    pub fn claim<T, E>(
        &mut self,
        at: Address,
        prepare: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, RegistryError<E>> {
        let at = at.function_base();
        if self.is_claimed(at) {
            return Err(RegistryError::Taken);
        }
        let room = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(RegistryError::NoRoom)?;

        let prepared = prepare().map_err(RegistryError::Prepare)?;
        self.slots[room] = Some(at);
        Ok(prepared)
    }

    /// Whether any register offset of `at`'s function is held.
    pub fn is_claimed(&self, at: Address) -> bool {
        let at = at.function_base();
        self.slots.iter().flatten().any(|held| *held == at)
    }

    /// Release one function after its transient lease has retired.
    ///
    /// The kernel wrapper exposes this only by consuming the unique device
    /// lease. Permanent driver claims never call it (D-199).
    #[must_use = "a refused release leaves the function claimed"]
    pub fn release(&mut self, at: Address) -> bool {
        let at = at.function_base();
        let Some(slot) = self.slots.iter_mut().find(|held| **held == Some(at)) else {
            return false;
        };
        *slot = None;
        true
    }

    /// Functions recorded so far.
    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// Whether no function has been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<const N: usize> Default for ClaimRegistry<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Why a function registry transaction refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError<E> {
    /// Another driver already holds this function.
    Taken,
    /// The registry's fixed lifetime bound is exhausted.
    NoRoom,
    /// Fallible device/resource preparation refused.
    Prepare(E),
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
}

/// A base address register, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bar {
    /// A memory window.
    Memory {
        /// Where it begins.
        address: u64,
        /// Whether it took two register slots.
        wide: bool,
        /// Whether the device tolerates speculative reads. **Not** whether the
        /// kernel should map it that way: a register file is mapped
        /// uncacheable whatever this says, and mapping it write-back is a
        /// silent, hard-to-debug bug (`012` §2).
        prefetchable: bool,
    },
    /// An I/O port window.
    Io {
        /// The first port.
        port: u16,
    },
    /// The slot is unused, or is the high half of the register before it.
    Absent,
}

impl Bar {
    /// Bit 0: zero for memory, one for I/O.
    const IS_IO: u32 = 1;
    /// Bits 2:1 == 0b10: the address is 64 bits and takes the next slot too.
    const WIDE: u32 = 0b100;
    /// Bit 3.
    const PREFETCHABLE: u32 = 0b1000;

    /// Decode one slot, given the slot after it in case this one is wide.
    ///
    /// `high` is read unconditionally by the caller because it must be read
    /// *before* this can say whether it was needed, and reading it costs one
    /// configuration cycle.
    pub const fn decode(low: u32, high: u32) -> Self {
        if low == 0 {
            return Self::Absent;
        }
        if low & Self::IS_IO != 0 {
            return Self::Io {
                port: (low & !0x3) as u16,
            };
        }
        let base = (low & !0xF) as u64;
        if low & 0b110 == Self::WIDE {
            Self::Memory {
                address: base | ((high as u64) << 32),
                wide: true,
                prefetchable: low & Self::PREFETCHABLE != 0,
            }
        } else {
            Self::Memory {
                address: base,
                wide: false,
                prefetchable: low & Self::PREFETCHABLE != 0,
            }
        }
    }

    /// How many register slots this occupies.
    ///
    /// One, except a 64-bit memory window, which occupies two. A walk that
    /// steps one at a time over a wide register reads its high half as a
    /// second base address and believes there is a device at a nonsense
    /// address.
    pub const fn slots(self) -> usize {
        match self {
            Self::Memory { wide: true, .. } => 2,
            _ => 1,
        }
    }
}

/// The size of a **memory** window, from the all-ones probe.
///
/// The protocol is: write `0xFFFF_FFFF` to the register, read it back, restore
/// what was there. What comes back has the address bits the device does not
/// decode set to zero, so the size is the two's complement of the mask.
///
/// `wide` says whether `high` is part of the same register or the next one.
/// It is a parameter rather than a convention because the first version of
/// this function took "pass all ones for a narrow register", and the I/O case
/// then complemented a thirty-two-bit mask in sixty-four bits and answered
/// eighteen quintillion (D-168). A width you must remember to pass correctly
/// is a width that will be passed wrongly.
///
/// # Errors
///
/// `None` when the mask decodes nothing — an absent register, or a device that
/// answered all zeros, both of which mean "there is no window here" rather
/// than "the window is four gigabytes".
///
/// # Examples
///
/// ```
/// use pci_config::memory_window_size;
///
/// // A 16 KiB window: the low fourteen address bits are not decoded.
/// assert_eq!(memory_window_size(0xFFFF_C000, 0, false), Some(0x4000));
///
/// // A 64-bit window of four gigabytes, where the high half carries it.
/// assert_eq!(memory_window_size(0, 0xFFFF_FFFF, true), Some(0x1_0000_0000));
/// ```
pub const fn memory_window_size(low: u32, high: u32, wide: bool) -> Option<u64> {
    if wide {
        let mask = ((low & !0xF) as u64) | ((high as u64) << 32);
        if mask == 0 {
            return None;
        }
        let size = (!mask).wrapping_add(1);
        if size == 0 { None } else { Some(size) }
    } else {
        let mask = low & !0xF;
        if mask == 0 {
            return None;
        }
        Some((!mask).wrapping_add(1) as u64)
    }
}

/// The size of an **I/O** window, from the same probe.
///
/// Thirty-two bits throughout, and separate from the memory form for exactly
/// that reason: an I/O window is at most sixty-five thousand ports, and
/// complementing its mask in sixty-four bits gives a number no machine has.
///
/// # Errors
///
/// `None` when the mask decodes nothing, or when the size will not fit the
/// port space it claims to describe.
///
/// # Examples
///
/// ```
/// use pci_config::io_window_size;
///
/// // Thirty-two ports. The low two bits are the register's own.
/// assert_eq!(io_window_size(0xFFFF_FFE1), Some(32));
/// ```
pub const fn io_window_size(mask: u32) -> Option<u16> {
    let mask = mask & !0x3;
    if mask == 0 {
        return None;
    }
    let size = (!mask).wrapping_add(1);
    // A run that does not fit the sixteen-bit port space is a device
    // misreporting, and claiming it would be claiming most of I/O space.
    if size == 0 || size > u16::MAX as u32 {
        return None;
    }
    Some(size as u16)
}

/// One command-register access in a verified enable transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAccess {
    /// Read the command register. Only its low sixteen bits are consumed.
    Read,
    /// Write only command bits, leaving the status half of the dword clear.
    Write(u16),
}

/// The exact command word a successful enablement replaced.
///
/// Kept opaque so a retiring device lease restores only state which the
/// verified enable transaction actually observed (D-199).
#[derive(Debug, PartialEq, Eq)]
pub struct CommandState(u16);

/// Why memory decode and bus mastering could not be verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnableError {
    /// A configuration access failed and any attempted change was restored.
    Access,
    /// The device read back without every required command bit.
    Refused(u16),
    /// The original command could not be read back exactly after failure.
    Restore,
}

impl core::fmt::Display for EnableError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Access => f.write_str("configuration access failed"),
            Self::Refused(observed) => {
                write!(f, "the command register read back as {observed:#06x}")
            }
            Self::Restore => f.write_str("the original command could not be verified"),
        }
    }
}

impl core::error::Error for EnableError {}

/// Enable memory decode and bus mastering as one verified transaction.
///
/// An already-enabled function needs only its first read. Once a write is
/// attempted, any access loss or refused readback restores the exact original
/// command and reads it again before returning the initiating error.
///
/// # Errors
///
/// [`EnableError::Access`] or [`EnableError::Refused`] only after exact
/// rollback, and [`EnableError::Restore`] when rollback cannot be verified.
pub fn enable_bus_master(
    mut access: impl FnMut(CommandAccess) -> Option<u32>,
) -> Result<CommandState, EnableError> {
    let original = access(CommandAccess::Read).ok_or(EnableError::Access)? as u16;
    let required = command::MEMORY_SPACE | command::BUS_MASTER;
    if original & required == required {
        return Ok(CommandState(original));
    }
    let wanted = original | required;

    let failure = match access(CommandAccess::Write(wanted)) {
        None => EnableError::Access,
        Some(_) => match access(CommandAccess::Read) {
            Some(bits) if bits as u16 & wanted == wanted => return Ok(CommandState(original)),
            Some(bits) => EnableError::Refused(bits as u16),
            None => EnableError::Access,
        },
    };

    let _ = access(CommandAccess::Write(original));
    match access(CommandAccess::Read) {
        Some(bits) if bits as u16 == original => Err(failure),
        Some(_) | None => Err(EnableError::Restore),
    }
}

/// Restore the exact command word captured by [`enable_bus_master`].
///
/// # Errors
///
/// [`EnableError::Restore`] unless the final readback equals the captured word.
pub fn restore_command(
    state: CommandState,
    mut access: impl FnMut(CommandAccess) -> Option<u32>,
) -> Result<(), EnableError> {
    let _ = access(CommandAccess::Write(state.0));
    match access(CommandAccess::Read) {
        Some(bits) if bits as u16 == state.0 => Ok(()),
        Some(_) | None => Err(EnableError::Restore),
    }
}

/// One configuration-space cycle in a BAR-probe transaction.
///
/// A platform adapter implements these against its configuration mechanism.
/// Keeping the sequence here makes its ordering and rollback host-testable;
/// the adapter only has to keep its hardware lock for the closure's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeAccess {
    /// Read the command register. Only its low sixteen bits are consumed.
    ReadCommand,
    /// Write only command bits. The status half of the dword must be zero.
    WriteCommand(u16),
    /// Read one BAR slot.
    ReadBar(usize),
    /// Write one BAR slot.
    WriteBar(usize, u32),
}

/// One decoded and sized BAR window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// What the register said.
    pub bar: Bar,
    /// How many bytes or ports it covers.
    pub size: u64,
}

/// Every window discovered by one restored probe transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbedBars {
    windows: [Option<Window>; BARS],
}

impl ProbedBars {
    /// The window in BAR slot `index`, if that slot decodes one.
    pub const fn window(&self, index: usize) -> Option<Window> {
        if index < BARS {
            self.windows[index]
        } else {
            None
        }
    }

    /// All slots, including the empty high halves of wide BARs.
    pub const fn into_windows(self) -> [Option<Window>; BARS] {
        self.windows
    }
}

/// Why a BAR-probe transaction could not produce windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError<E> {
    /// The configuration mechanism refused an access before rollback failed.
    Access(E),
    /// At least one original register could not be put back.
    Restore(E),
    /// The header layout or a BAR's slot use is impossible.
    Malformed,
    /// A device returned a mask or base that cannot describe its window.
    InvalidWindow,
}

/// Probe every BAR under one decode-off, fully restored transaction.
///
/// The closure is called for configuration cycles while the platform adapter
/// holds its configuration lock. The command register is restored last, so a
/// device cannot decode a half-restored window. A 64-bit BAR has both halves
/// filled with ones before either half is read.
///
/// # Errors
///
/// Refuses an impossible register count, a wide BAR in the final available
/// slot, an invalid size or alignment, or any configuration access. Once a
/// write may have happened, every modified BAR and then the command register
/// are restored even when the initiating access or an earlier restore failed.
pub fn probe_bars<E>(
    registers: usize,
    mut access: impl FnMut(ProbeAccess) -> Result<u32, E>,
) -> Result<ProbedBars, ProbeError<E>> {
    if registers > BARS {
        return Err(ProbeError::Malformed);
    }

    let command = access(ProbeAccess::ReadCommand).map_err(ProbeError::Access)? as u16;
    let mut original = [0u32; BARS];
    for (index, slot) in original.iter_mut().enumerate().take(registers) {
        *slot = access(ProbeAccess::ReadBar(index)).map_err(ProbeError::Access)?;
    }

    let mut index = 0;
    while index < registers {
        let wide = original[index] & 0b110 == 0b100;
        if wide && index + 1 == registers {
            return Err(ProbeError::Malformed);
        }
        index += if wide { 2 } else { 1 };
    }

    let mut modified = [false; BARS];
    let command_modified = true;
    let decode_off = command & !(command::IO_SPACE | command::MEMORY_SPACE);
    if let Err(error) = access(ProbeAccess::WriteCommand(decode_off)) {
        return after_access_error(
            error,
            &mut access,
            &original,
            &modified,
            command,
            command_modified,
        );
    }

    let mut masks = [0u32; BARS];
    index = 0;
    while index < registers {
        let wide = original[index] & 0b110 == 0b100;
        modified[index] = true;
        if let Err(error) = access(ProbeAccess::WriteBar(index, u32::MAX)) {
            return after_access_error(
                error,
                &mut access,
                &original,
                &modified,
                command,
                command_modified,
            );
        }
        if wide {
            modified[index + 1] = true;
            if let Err(error) = access(ProbeAccess::WriteBar(index + 1, u32::MAX)) {
                return after_access_error(
                    error,
                    &mut access,
                    &original,
                    &modified,
                    command,
                    command_modified,
                );
            }
        }

        masks[index] = match access(ProbeAccess::ReadBar(index)) {
            Ok(mask) => mask,
            Err(error) => {
                return after_access_error(
                    error,
                    &mut access,
                    &original,
                    &modified,
                    command,
                    command_modified,
                );
            }
        };
        if wide {
            masks[index + 1] = match access(ProbeAccess::ReadBar(index + 1)) {
                Ok(mask) => mask,
                Err(error) => {
                    return after_access_error(
                        error,
                        &mut access,
                        &original,
                        &modified,
                        command,
                        command_modified,
                    );
                }
            };
        }
        index += if wide { 2 } else { 1 };
    }

    if let Some(error) = restore(&mut access, &original, &modified, command, command_modified) {
        return Err(ProbeError::Restore(error));
    }
    decode_windows(registers, &original, &masks).map(|windows| ProbedBars { windows })
}

fn after_access_error<E>(
    error: E,
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    original: &[u32; BARS],
    modified: &[bool; BARS],
    command: u16,
    command_modified: bool,
) -> Result<ProbedBars, ProbeError<E>> {
    match restore(access, original, modified, command, command_modified) {
        Some(restore) => Err(ProbeError::Restore(restore)),
        None => Err(ProbeError::Access(error)),
    }
}

fn restore<E>(
    access: &mut impl FnMut(ProbeAccess) -> Result<u32, E>,
    original: &[u32; BARS],
    modified: &[bool; BARS],
    command: u16,
    command_modified: bool,
) -> Option<E> {
    let mut failed = None;
    for index in (0..BARS).rev() {
        if modified[index]
            && let Err(error) = access(ProbeAccess::WriteBar(index, original[index]))
            && failed.is_none()
        {
            failed = Some(error);
        }
    }
    if command_modified
        && let Err(error) = access(ProbeAccess::WriteCommand(command))
        && failed.is_none()
    {
        failed = Some(error);
    }
    failed
}

fn decode_windows<E>(
    registers: usize,
    original: &[u32; BARS],
    masks: &[u32; BARS],
) -> Result<[Option<Window>; BARS], ProbeError<E>> {
    let mut windows = [None; BARS];
    let mut index = 0;
    while index < registers {
        let kind = if original[index] == 0 {
            masks[index] & 0xF
        } else {
            original[index]
        };
        let wide = kind & 0b110 == 0b100;
        let (bar, size) = if kind & 1 != 0 {
            let Some(size) = io_window_size(masks[index]) else {
                index += 1;
                continue;
            };
            (
                Bar::Io {
                    port: (original[index] & !0x3) as u16,
                },
                u64::from(size),
            )
        } else {
            if wide && index + 1 == registers {
                return Err(ProbeError::Malformed);
            }
            let high = if wide { original[index + 1] } else { 0 };
            let mask_high = if wide { masks[index + 1] } else { 0 };
            let Some(size) = memory_window_size(masks[index], mask_high, wide) else {
                index += if wide { 2 } else { 1 };
                continue;
            };
            let address = u64::from(original[index] & !0xF) | (u64::from(high) << 32);
            (
                Bar::Memory {
                    address,
                    wide,
                    prefetchable: kind & 0b1000 != 0,
                },
                size,
            )
        };

        let base = match bar {
            Bar::Memory { address, .. } => address,
            Bar::Io { port } => u64::from(port),
            Bar::Absent => 0,
        };
        if !size.is_power_of_two()
            || !base.is_multiple_of(size)
            || matches!(bar, Bar::Io { .. }) && base + size > u64::from(u16::MAX) + 1
        {
            return Err(ProbeError::InvalidWindow);
        }
        windows[index] = Some(Window { bar, size });
        index += if wide { 2 } else { 1 };
    }
    Ok(windows)
}

/// Whether a vendor word means "nothing is here".
///
/// An absent function does not fail a configuration read; the bus returns all
/// ones, so `0xFFFF` is the answer and not an error. A driver that treats it
/// as a vendor id enumerates 256 buses of imaginary devices.
pub const fn is_absent(vendor: u16) -> bool {
    vendor == 0xFFFF
}

/// A bridge's secondary bus number, out of the word at [`header::BUS_NUMBERS`].
///
/// The bus behind it, which is the number a recursive enumeration descends to.
pub const fn secondary_bus(bits: u32) -> u8 {
    (bits >> 8) as u8
}

/// A bridge's subordinate bus number: the highest bus behind it.
pub const fn subordinate_bus(bits: u32) -> u8 {
    (bits >> 16) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACE_CAPACITY: usize = 40;
    const COMMAND_TRACE_CAPACITY: usize = 5;

    struct FakeCommand {
        command: u16,
        writable: u16,
        trace: [Option<CommandAccess>; COMMAND_TRACE_CAPACITY],
        calls: usize,
        fail_at: Option<usize>,
    }

    impl FakeCommand {
        fn new(command: u16, writable: u16) -> Self {
            Self {
                command,
                writable,
                trace: [None; COMMAND_TRACE_CAPACITY],
                calls: 0,
                fail_at: None,
            }
        }

        fn access(&mut self, event: CommandAccess) -> Option<u32> {
            let call = self.calls;
            self.trace[call] = Some(event);
            self.calls += 1;
            if self.fail_at == Some(call) {
                return None;
            }

            match event {
                CommandAccess::Read => Some(u32::from(self.command)),
                CommandAccess::Write(value) => {
                    self.command = (self.command & !self.writable) | (value & self.writable);
                    Some(0)
                }
            }
        }

        fn events(&self) -> &[Option<CommandAccess>] {
            &self.trace[..self.calls]
        }
    }

    #[derive(Clone)]
    struct FakeConfig {
        command: u16,
        status: u16,
        bars: [u32; BARS],
        masks: [u32; BARS],
        trace: [Option<ProbeAccess>; TRACE_CAPACITY],
        calls: usize,
        fail_at: Option<usize>,
    }

    impl FakeConfig {
        fn device() -> Self {
            Self {
                command: command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
                status: 0xA55A,
                bars: [0x8000_0004, 1, 0x0000_C001, 0, 0, 0],
                masks: [0xFFFF_C004, u32::MAX, 0xFFFF_FFE1, 0, 0, 0],
                trace: [None; TRACE_CAPACITY],
                calls: 0,
                fail_at: None,
            }
        }

        fn access(&mut self, event: ProbeAccess) -> Result<u32, usize> {
            let call = self.calls;
            self.trace[call] = Some(event);
            self.calls += 1;

            let answer = match event {
                ProbeAccess::ReadCommand => {
                    u32::from(self.command) | (u32::from(self.status) << 16)
                }
                ProbeAccess::WriteCommand(command) => {
                    self.command = command;
                    0
                }
                ProbeAccess::ReadBar(index) => {
                    if self.bars[index] == u32::MAX {
                        self.masks[index]
                    } else {
                        self.bars[index]
                    }
                }
                ProbeAccess::WriteBar(index, value) => {
                    self.bars[index] = value;
                    0
                }
            };
            if self.fail_at == Some(call) {
                Err(call)
            } else {
                Ok(answer)
            }
        }

        fn events(&self) -> &[Option<ProbeAccess>] {
            &self.trace[..self.calls]
        }
    }

    /// A successful enable is read back and preserves every unrelated bit.
    #[test]
    fn bus_master_enable_is_verified_and_preserves_the_command() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;
        let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
        let mut fake = FakeCommand::new(original, u16::MAX);

        let state = enable_bus_master(|event| fake.access(event)).expect("enabled");
        assert_eq!(fake.command, wanted);
        assert_eq!(
            fake.events(),
            &[
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(wanted)),
                Some(CommandAccess::Read),
            ]
        );

        let mut already = FakeCommand::new(wanted, u16::MAX);
        let already_state = enable_bus_master(|event| already.access(event)).expect("enabled");
        assert_eq!(already.events(), &[Some(CommandAccess::Read)]);

        restore_command(state, |event| fake.access(event)).expect("restored");
        assert_eq!(fake.command, original);
        restore_command(already_state, |event| already.access(event)).expect("restored");
        assert_eq!(already.command, wanted);
    }

    /// A write is not success until the device reports every required bit.
    #[test]
    fn a_refused_bus_master_enable_restores_the_exact_command() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;
        let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
        let observed = wanted & !command::BUS_MASTER;
        let mut fake = FakeCommand::new(original, !command::BUS_MASTER);

        assert_eq!(
            enable_bus_master(|event| fake.access(event)),
            Err(EnableError::Refused(observed))
        );
        assert_eq!(fake.command, original);
        assert_eq!(
            fake.events(),
            &[
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(wanted)),
                Some(CommandAccess::Read),
                Some(CommandAccess::Write(original)),
                Some(CommandAccess::Read),
            ]
        );
    }

    /// Every access loss after a possible write either restores or says it could not.
    #[test]
    fn every_failed_bus_master_access_restores_or_reports_uncertainty() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;

        for fail_at in 0..=2 {
            let mut fake = FakeCommand::new(original, u16::MAX);
            fake.fail_at = Some(fail_at);
            assert_eq!(
                enable_bus_master(|event| fake.access(event)),
                Err(EnableError::Access),
                "access {fail_at}"
            );
            assert_eq!(fake.command, original, "access {fail_at}");
        }

        let refuses_bus_master = !command::BUS_MASTER;
        let mut restore_write = FakeCommand::new(original, refuses_bus_master);
        restore_write.fail_at = Some(3);
        assert_eq!(
            enable_bus_master(|event| restore_write.access(event)),
            Err(EnableError::Restore)
        );
        assert_ne!(restore_write.command, original);

        let mut restore_read = FakeCommand::new(original, refuses_bus_master);
        restore_read.fail_at = Some(4);
        assert_eq!(
            enable_bus_master(|event| restore_read.access(event)),
            Err(EnableError::Restore)
        );
        assert_eq!(restore_read.command, original);
    }

    /// A successful lease retirement restores the captured word exactly.
    #[test]
    fn a_bus_master_lease_restores_the_exact_command_or_reports_uncertainty() {
        const OTHER: u16 = 1 << 9;
        let original = command::IO_SPACE | OTHER;
        let mut restored = FakeCommand::new(original, u16::MAX);
        let state = enable_bus_master(|event| restored.access(event)).expect("enabled");
        assert_eq!(
            restore_command(state, |event| restored.access(event)),
            Ok(())
        );
        assert_eq!(restored.command, original);

        let mut refused = FakeCommand::new(original, u16::MAX);
        let state = enable_bus_master(|event| refused.access(event)).expect("enabled");
        refused.writable = 0;
        assert_eq!(
            restore_command(state, |event| refused.access(event)),
            Err(EnableError::Restore)
        );
        assert_ne!(refused.command, original);
    }

    /// An address is the bus, device, function and offset the manual gives.
    ///
    /// Transcribed from the specification's field positions rather than
    /// derived, and the values are **pairwise distinct** so no two fields can
    /// be swapped with this still passing (D-136).
    #[test]
    fn an_address_puts_every_field_where_the_manual_says() {
        let at = Address::new(0x12, 0x1A, 0x05, 0x3C).expect("a valid address");

        assert_eq!(at.bits() >> 31, 1, "the enable bit");
        assert_eq!(at.bus(), 0x12);
        assert_eq!(at.device(), 0x1A);
        assert_eq!(at.function(), 0x05);
        assert_eq!(at.offset(), 0x3C);
        assert_eq!(at.bits(), 0x8012_D53C);
    }

    /// Every encodable address round-trips through its own fields.
    ///
    /// Exhaustive over devices, functions and offsets, and a spread of buses:
    /// a field that overlapped its neighbour would survive any single example
    /// and cannot survive all of them.
    #[test]
    fn every_address_round_trips_through_its_fields() {
        for bus in [0u8, 1, 0x7F, 0x80, 0xFE, 0xFF] {
            for device in 0..DEVICES {
                for function in 0..FUNCTIONS {
                    for offset in (0..HEADER_BYTES).step_by(4) {
                        let at =
                            Address::new(bus, device, function, offset).expect("all encodable");
                        assert_eq!(
                            (at.bus(), at.device(), at.function(), at.offset()),
                            (bus, device, function, offset)
                        );
                    }
                }
            }
        }
    }

    /// Register offsets are not device identities.
    #[test]
    fn every_offset_of_one_function_has_one_identity() {
        let identity = Address::new(0x12, 0x1A, 5, 0).expect("valid");
        for offset in (0..HEADER_BYTES).step_by(4) {
            let alias = identity.at(offset).expect("valid offset");
            assert_eq!(alias.function_base(), identity);
        }
        assert_ne!(identity.at(header::CLASS), identity.at(header::BAR0));
    }

    /// Every fallible preparation edge precedes the registry commit point.
    #[test]
    fn every_refused_claim_preparation_leaves_the_registry_unchanged() {
        let at = Address::new(3, 4, 5, header::CLASS).expect("valid");
        let mut registry = ClaimRegistry::<2>::new();

        for refused in 0..8 {
            let result = registry.claim(at, || {
                for edge in 0..8 {
                    if edge == refused {
                        return Err(edge);
                    }
                }
                Ok(())
            });
            assert_eq!(result, Err(RegistryError::Prepare(refused)));
            assert!(registry.is_empty(), "failure at edge {refused} committed");
            assert!(!registry.is_claimed(at));
        }

        assert_eq!(registry.claim(at, || Ok::<_, ()>(0xA5)), Ok(0xA5));
        assert_eq!(registry.len(), 1);
        assert!(registry.is_claimed(at.at(header::BAR0).expect("valid alias")));
        assert_eq!(
            registry.claim(at.at(header::VENDOR).expect("valid alias"), || Ok::<_, ()>(
                ()
            )),
            Err(RegistryError::Taken)
        );
        assert_eq!(registry.len(), 1, "duplicate refusal changed the holder");
    }

    /// A transient claim returns exactly its canonical function slot.
    #[test]
    fn a_claim_release_is_exact_and_makes_room_for_the_same_function() {
        let first = Address::new(3, 4, 5, header::CLASS).expect("valid");
        let second = Address::new(3, 4, 6, header::VENDOR).expect("valid");
        let absent = Address::new(3, 4, 7, header::VENDOR).expect("valid");
        let mut registry = ClaimRegistry::<2>::new();

        assert_eq!(registry.claim(first, || Ok::<_, ()>(())), Ok(()));
        assert_eq!(registry.claim(second, || Ok::<_, ()>(())), Ok(()));
        assert!(!registry.release(absent));
        assert_eq!(registry.len(), 2, "a refused release changed the registry");

        assert!(registry.release(first.at(header::BAR0).expect("valid alias")));
        assert!(!registry.is_claimed(first));
        assert!(registry.is_claimed(second));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.claim(first, || Ok::<_, ()>(0xA5)), Ok(0xA5));
        assert!(!registry.release(absent));
        assert_eq!(registry.len(), 2);
    }

    /// An address that cannot be encoded is refused, not truncated.
    ///
    /// The misaligned offset is the one that matters: it does not fault, it
    /// reads the dword containing it, and the caller shifts the wrong bytes
    /// out of a plausible-looking value.
    #[test]
    fn an_address_that_cannot_be_encoded_is_refused() {
        assert!(Address::new(0, DEVICES, 0, 0).is_none(), "device 32");
        assert!(Address::new(0, 0, FUNCTIONS, 0).is_none(), "function 8");
        assert!(
            Address::new(0, 0, 0, HEADER_BYTES).is_none(),
            "past the header"
        );
        for misaligned in [1u16, 2, 3, 0x0D, 0xFF] {
            assert!(
                Address::new(0, 0, 0, misaligned).is_none(),
                "offset {misaligned:#x} is not a dword boundary"
            );
        }

        // And `at` refuses the same things, because it is the same rule from
        // the other direction — a walk that re-offsets is where a misaligned
        // value would actually come from.
        let at = Address::new(0, 1, 0, 0).expect("valid");
        assert!(at.at(0x0D).is_none());
        assert!(at.at(HEADER_BYTES).is_none());
        assert_eq!(at.at(0x10).expect("valid").offset(), 0x10);
        assert_eq!(at.at(0x10).expect("valid").device(), 1, "the rest is kept");
    }

    /// The class word is read as four separate bytes.
    ///
    /// Pairwise distinct on purpose: `01:08:02:03` would let class and
    /// subclass be swapped, so every byte here differs from every other.
    #[test]
    fn the_class_word_is_read_as_four_separate_bytes() {
        let class = Class::from_bits(0x0C_03_30_A1);
        assert_eq!(class.class, 0x0C);
        assert_eq!(class.subclass, 0x03);
        assert_eq!(class.interface, 0x30);
        assert_eq!(class.revision, 0xA1);
    }

    /// NVM Express is all three bytes, and its neighbours are not it.
    ///
    /// `01:08:01` is NVMHCI — an older interface a driver written for `02`
    /// would misprogram — so a two-byte test would accept the wrong device
    /// and the failure would be a controller that never answers.
    #[test]
    fn nvme_is_all_three_bytes_and_its_neighbours_are_not() {
        assert!(Class::from_bits(0x01_08_02_00).is_nvme());

        for near in [
            0x01_08_01_00u32, // NVMHCI
            0x01_08_03_00,    // a prog-if this driver does not know
            0x01_06_02_00,    // SATA
            0x02_08_02_00,    // a network controller with the same tail
        ] {
            assert!(
                !Class::from_bits(near).is_nvme(),
                "{near:#010x} is not NVM Express"
            );
        }
    }

    /// A bridge is recognised, because enumeration has to descend through one.
    #[test]
    fn a_bridge_is_recognised_and_nothing_else_is() {
        assert!(Class::from_bits(0x06_04_00_00).is_bridge());
        assert!(
            !Class::from_bits(0x06_00_00_00).is_bridge(),
            "a host bridge"
        );
        assert!(
            !Class::from_bits(0x06_01_00_00).is_bridge(),
            "an ISA bridge"
        );
        assert!(!Class::from_bits(0x01_08_02_00).is_bridge());
    }

    /// The header-type byte's two halves are read separately.
    #[test]
    fn the_header_type_byte_is_a_layout_and_a_flag() {
        let plain = HeaderType::from_bits(0x00);
        assert!(!plain.is_multifunction());
        assert_eq!(plain.layout(), 0);
        assert!(!plain.is_bridge());

        let multi_bridge = HeaderType::from_bits(0x81);
        assert!(multi_bridge.is_multifunction());
        assert_eq!(multi_bridge.layout(), 1);
        assert!(multi_bridge.is_bridge());

        // The flag must not leak into the layout, which is what a missing mask
        // would do: `0x81 & 0x7F` is 1, `0x81` is 129.
        let multi_plain = HeaderType::from_bits(0x80);
        assert!(multi_plain.is_multifunction());
        assert_eq!(multi_plain.layout(), 0);
        assert!(!multi_plain.is_bridge());
    }

    /// A base address register decodes into what it actually describes.
    #[test]
    fn a_base_address_register_decodes_to_its_window() {
        assert_eq!(Bar::decode(0, 0), Bar::Absent);

        assert_eq!(Bar::decode(0xC001, 0), Bar::Io { port: 0xC000 });

        assert_eq!(
            Bar::decode(0xFEBF_0000, 0),
            Bar::Memory {
                address: 0xFEBF_0000,
                wide: false,
                prefetchable: false,
            }
        );

        assert_eq!(
            Bar::decode(0xFEBF_000C, 0x0000_0001),
            Bar::Memory {
                address: 0x1_FEBF_0000,
                wide: true,
                prefetchable: true,
            }
        );
    }

    /// A 64-bit register occupies two slots and a 32-bit one does not.
    ///
    /// The whole reason this function exists: a walk that steps one at a time
    /// over a wide register reads its high half as a second base address, and
    /// believes there is a window at whatever the top thirty-two bits were.
    #[test]
    fn a_wide_register_occupies_the_slot_after_it() {
        assert_eq!(Bar::decode(0xFEBF_000C, 0).slots(), 2);
        assert_eq!(Bar::decode(0xFEBF_0000, 0).slots(), 1);
        assert_eq!(Bar::decode(0xC001, 0).slots(), 1);
        assert_eq!(Bar::decode(0, 0).slots(), 1);
    }

    /// A memory window's size is the two's complement of what came back.
    ///
    /// Every power of two from a page to two gigabytes, because the failure is
    /// off by a factor of two and a single example cannot see it.
    #[test]
    fn a_memory_window_size_is_the_complement_of_the_probe() {
        for bits in 4..31u32 {
            let size = 1u32 << bits;
            let mask = !(size - 1);
            assert_eq!(
                memory_window_size(mask, 0, false),
                Some(u64::from(size)),
                "a {size}-byte window"
            );
        }

        // A 64-bit window past four gigabytes, where the high half carries it.
        assert_eq!(
            memory_window_size(0, 0xFFFF_FFFF, true),
            Some(0x1_0000_0000)
        );
        assert_eq!(
            memory_window_size(0xFFF0_0000, 0xFFFF_FFFF, true),
            Some(0x10_0000)
        );
    }

    /// A narrow register's mask is complemented in thirty-two bits.
    ///
    /// The bug this function was split to prevent: complementing a
    /// thirty-two-bit mask in sixty-four bits sets the whole high half and
    /// answers eighteen quintillion, which a caller then tries to map
    /// (D-168).
    #[test]
    fn a_narrow_window_is_not_complemented_in_a_wide_word() {
        // The same mask read both ways: as a narrow register it is 16 KiB, as
        // the low half of a wide one it is very much not.
        assert_eq!(memory_window_size(0xFFFF_C000, 0, false), Some(0x4000));
        assert!(memory_window_size(0xFFFF_C000, 0, true).unwrap() > u64::from(u32::MAX));
    }

    /// An I/O window's size is a port count, not an address range.
    #[test]
    fn an_io_window_size_is_a_port_count() {
        for bits in 2..16u32 {
            let size = 1u32 << bits;
            let mask = !(size - 1);
            assert_eq!(
                io_window_size(mask),
                u16::try_from(size).ok(),
                "a {size}-port window"
            );
        }
        // The register's own low bits are not address bits.
        assert_eq!(io_window_size(0xFFFF_FFE1), Some(32));
    }

    /// A probe that decoded nothing is refused rather than read as huge.
    ///
    /// All zeros means "there is no window here". Read as a mask it is a
    /// four-gigabyte window at address zero, which a kernel would then map.
    #[test]
    fn a_probe_that_decoded_nothing_is_refused() {
        assert_eq!(memory_window_size(0, 0, false), None);
        assert_eq!(memory_window_size(0, 0, true), None);
        assert_eq!(io_window_size(0), None);
        // Only the low bits set, which are the register's own and not address
        // bits at all.
        assert_eq!(memory_window_size(0x0000_000F, 0, false), None);
        assert_eq!(io_window_size(0x0000_0003), None);
        // And a window larger than the port space is a device misreporting.
        assert_eq!(io_window_size(0xFFFF_0000), None);
    }

    /// Decode is off throughout one wide-aware transaction and restored last.
    #[test]
    fn a_bar_probe_is_one_restored_decode_off_transaction() {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        let probed = probe_bars(BARS, |event| fake.access(event)).expect("valid device");

        assert_eq!(fake.command, original.command);
        assert_eq!(
            fake.status, original.status,
            "status is never echoed into RW1C bits"
        );
        assert_eq!(fake.bars, original.bars);
        assert_eq!(
            probed.window(0),
            Some(Window {
                bar: Bar::Memory {
                    address: 0x1_8000_0000,
                    wide: true,
                    prefetchable: false,
                },
                size: 0x4000,
            })
        );
        assert_eq!(
            probed.window(1),
            None,
            "the high half is not another window"
        );
        assert_eq!(
            probed.window(2),
            Some(Window {
                bar: Bar::Io { port: 0xC000 },
                size: 32,
            })
        );

        let expected = [
            ProbeAccess::ReadCommand,
            ProbeAccess::ReadBar(0),
            ProbeAccess::ReadBar(1),
            ProbeAccess::ReadBar(2),
            ProbeAccess::ReadBar(3),
            ProbeAccess::ReadBar(4),
            ProbeAccess::ReadBar(5),
            ProbeAccess::WriteCommand(command::BUS_MASTER),
            ProbeAccess::WriteBar(0, u32::MAX),
            ProbeAccess::WriteBar(1, u32::MAX),
            ProbeAccess::ReadBar(0),
            ProbeAccess::ReadBar(1),
            ProbeAccess::WriteBar(2, u32::MAX),
            ProbeAccess::ReadBar(2),
            ProbeAccess::WriteBar(3, u32::MAX),
            ProbeAccess::ReadBar(3),
            ProbeAccess::WriteBar(4, u32::MAX),
            ProbeAccess::ReadBar(4),
            ProbeAccess::WriteBar(5, u32::MAX),
            ProbeAccess::ReadBar(5),
            ProbeAccess::WriteBar(5, 0),
            ProbeAccess::WriteBar(4, 0),
            ProbeAccess::WriteBar(3, 0),
            ProbeAccess::WriteBar(2, 0x0000_C001),
            ProbeAccess::WriteBar(1, 1),
            ProbeAccess::WriteBar(0, 0x8000_0004),
            ProbeAccess::WriteCommand(
                command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER,
            ),
        ];
        assert_eq!(fake.events(), expected.map(Some));
    }

    /// Every access failure after the first write rolls the exact device back.
    #[test]
    fn every_probe_access_failure_restores_the_device() {
        // Calls 7 through 19 are the decode-off write and probe cycles. The
        // events after them are rollback itself and have a separate proof.
        for fail_at in 7..20 {
            let mut fake = FakeConfig::device();
            let original = fake.clone();
            fake.fail_at = Some(fail_at);

            assert_eq!(
                probe_bars(BARS, |event| fake.access(event)),
                Err(ProbeError::Access(fail_at)),
                "failure at cycle {fail_at}"
            );
            assert_eq!(fake.command, original.command, "command at cycle {fail_at}");
            assert_eq!(fake.status, original.status, "status at cycle {fail_at}");
            assert_eq!(fake.bars, original.bars, "BARs at cycle {fail_at}");
            assert_eq!(
                fake.events().last(),
                Some(&Some(ProbeAccess::WriteCommand(original.command))),
                "decode is restored last at cycle {fail_at}"
            );
        }
    }

    /// One refused restore does not prevent the remaining registers restoring.
    #[test]
    fn a_restore_failure_still_attempts_every_later_restore() {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.fail_at = Some(23);

        assert_eq!(
            probe_bars(BARS, |event| fake.access(event)),
            Err(ProbeError::Restore(23))
        );
        assert_eq!(
            fake.command, original.command,
            "command restoration was attempted"
        );
        assert_eq!(fake.status, original.status);
        assert_eq!(
            fake.events().last(),
            Some(&Some(ProbeAccess::WriteCommand(original.command)))
        );
    }

    /// A wide BAR cannot consume a slot belonging to another header field.
    #[test]
    fn a_wide_bar_in_the_final_slot_is_refused_before_mutation() {
        let mut fake = FakeConfig::device();
        fake.bars[0] = 0x0000_C001;
        fake.bars[1] = 0x0000_0004;
        let original = fake.clone();

        assert_eq!(
            probe_bars(BRIDGE_BARS, |event| fake.access(event)),
            Err(ProbeError::Malformed)
        );
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);
        assert!(fake.events().iter().flatten().all(|event| !matches!(
            event,
            ProbeAccess::WriteCommand(_) | ProbeAccess::WriteBar(_, _)
        )));
    }

    /// A nonsensical mask is refused only after the hardware is restored.
    #[test]
    fn an_invalid_bar_mask_is_refused_after_restoration() {
        let mut fake = FakeConfig::device();
        fake.masks[2] = 0xFFFF_FFF5;
        let original = fake.clone();

        assert_eq!(
            probe_bars(BARS, |event| fake.access(event)),
            Err(ProbeError::InvalidWindow)
        );
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.status, original.status);
        assert_eq!(fake.bars, original.bars);
    }

    /// An absent function is all ones, and that is not a vendor.
    #[test]
    fn an_absent_function_reads_as_all_ones() {
        assert!(is_absent(0xFFFF));
        assert!(!is_absent(0x8086), "Intel");
        assert!(!is_absent(0x1B36), "QEMU");
        assert!(!is_absent(0x15AD), "VMware");
        assert!(!is_absent(0x0000), "a vendor of zero is odd, not absent");
    }

    /// A bridge's bus numbers are three separate bytes.
    #[test]
    fn a_bridges_bus_numbers_are_read_separately() {
        // Primary 0x11, secondary 0x22, subordinate 0x33: pairwise distinct,
        // so a shift off by eight cannot pass.
        let bits = 0x0044_3322u32 << 8 | 0x11;
        assert_eq!(secondary_bus(bits), 0x22);
        assert_eq!(subordinate_bus(bits), 0x33);
    }
}
