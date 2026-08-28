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
//! **Claims are permanent**, exactly as port claims are and for the same
//! reason (D-138). There is no release, so there is no window in which a device
//! is half-claimed, and a refusal strands nothing: the holder is unaffected and
//! the refused caller has taken nothing.
//!
//! **Enumeration recurses.** A scan of bus zero is not enough — VMware places
//! functions behind `pciBridge0`–`pciBridge7`, and QEMU's q35 puts a
//! DMI-to-PCI bridge at `00:1e.0` — so a controller can sit on a bus that
//! nothing on bus zero mentions except a bridge (`012` §2). The walk is
//! iterative over a worklist rather than recursive, because kernel stacks are
//! thirty-two kilobytes and a malicious or broken bridge topology is a cycle.

use pci_config::{
    Address, Bar, ClaimRegistry, Class, HeaderType, ProbeError, RegistryError, Window, command,
    header,
};
use wost::arch::current::pci;
use wost::sync::SpinLock;

use crate::thread::ThreadContext;

/// Base address registers a type 0 header has.
pub(crate) const BARS: usize = pci_config::BARS;

/// Functions that may be claimed at once.
///
/// A policy number, small because this kernel has one driver. Claims are
/// permanent, so this is a bound on devices *ever* claimed rather than on
/// devices claimed at one time — which is the same trade D-138 made for ports.
const MAX_CLAIMS: usize = 8;

/// Buses the walk will visit before giving up.
///
/// There are 256 and a correct topology visits each at most once. The bound is
/// here because a bridge that reports itself as its own secondary bus is a
/// cycle, and a kernel that trusts the machine's numbering hangs on it.
const MAX_BUSES: usize = 256;

/// One function found on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Function {
    /// Where it lives.
    pub(crate) at: Address,
    /// Who made it.
    pub(crate) vendor: u16,
    /// Which of theirs it is.
    pub(crate) device: u16,
    /// What it says it is.
    pub(crate) class: Class,
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
                    // Function zero absent means the whole device is: the
                    // specification allows a gap between functions, but never
                    // a device whose first function is missing.
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

                // **Both** tests, and they are different claims: the class
                // says what the function is for and the header type says what
                // the rest of its registers mean. A function that claimed one
                // and not the other would have its bus numbers read out of
                // whatever a type 0 header has there.
                if entry.class.is_bridge()
                    && entry.header.is_bridge()
                    && let Some(behind) = secondary_of(&entry)
                    && !seen[behind as usize]
                    && queued < MAX_BUSES
                {
                    pending[queued] = behind;
                    queued += 1;
                }

                // A single-function device answers every function with its
                // own header, so probing past zero would report it eight
                // times. Bit 7 of the header type is the machine's own answer
                // to how many there are.
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

    let class = Class::from_bits(pci::read(at.at(header::CLASS)?)?);
    let header = HeaderType::from_bits((pci::read(at.at(header::HEADER_TYPE)?)? >> 16) as u8);

    Some(Function {
        at,
        vendor,
        device: (identity >> 16) as u16,
        class,
        header,
    })
}

/// The bus behind a bridge.
fn secondary_of(entry: &Function) -> Option<u8> {
    let at = entry.at.at(header::BUS_NUMBERS)?;
    Some(pci_config::secondary_bus(pci::read(at)?))
}

/// Functions already claimed. Permanent, like the port registry (D-138).
static CLAIMED: SpinLock<ClaimRegistry<MAX_CLAIMS>> = SpinLock::new(ClaimRegistry::new());

/// Why a device could not be claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl core::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoBus => f.write_str("configuration space is not reachable"),
            Self::Absent => f.write_str("nothing is at that address"),
            Self::Taken => f.write_str("another driver holds it"),
            Self::NoRoom => f.write_str("no room in the device registry"),
            Self::WindowTaken => f.write_str("an i/o window it decodes is already claimed"),
            Self::InvalidWindow => f.write_str("a base address window is malformed"),
        }
    }
}

/// A claimed function, and the evidence for touching it.
///
/// Not `Clone` and not `Copy`: it *is* the claim, and a copy would be a second
/// driver for one device with nothing to say so. There is no release, so no
/// `Drop` either — a claim outlives the boot (D-138).
#[derive(Debug)]
pub(crate) struct Device {
    at: Address,
    vendor: u16,
    device: u16,
    windows: [Option<Window>; BARS],
    /// The I/O runs this device decodes, claimed with it.
    ///
    /// Held rather than merely recorded, and never read here: a `PortSpan` is
    /// the *access*, so this is the thing a driver reaches through to touch
    /// registers. Slice 5 is its first reader. It is kept in the device rather
    /// than handed back separately because the claim is one thing — a driver
    /// with the ports and no device would be the arrangement D-77 objected to.
    #[expect(dead_code)]
    ports: [Option<wost::arch::current::port::PortSpan>; BARS],
}

impl Device {
    /// Where it lives.
    #[cfg_attr(not(feature = "selftest"), expect(dead_code))]
    pub(crate) const fn at(&self) -> Address {
        self.at
    }

    /// Who made it, and which of theirs it is.
    pub(crate) const fn identity(&self) -> (u16, u16) {
        (self.vendor, self.device)
    }

    /// The window base address register `index` decodes, if it decodes one.
    pub(crate) fn window(&self, index: usize) -> Option<Window> {
        self.windows.get(index).copied().flatten()
    }

    /// Turn on memory space and **bus master**.
    ///
    /// Without bus master the controller cannot DMA: queues never advance, and
    /// **there is no error anywhere to read** (`012` §2). It is the single most
    /// expensive bit to forget in this subsystem, which is why it is set here,
    /// by the thing that holds the claim, rather than by a driver that might
    /// not.
    pub(crate) fn enable_bus_master(&mut self) -> bool {
        let Some(at) = self.at.at(header::COMMAND) else {
            return false;
        };
        let Some(current) = pci::read(at) else {
            return false;
        };
        let wanted = current as u16 | command::MEMORY_SPACE | command::BUS_MASTER;
        pci::write_command(at, wanted)
            && pci::read(at).is_some_and(|back| back as u16 & wanted == wanted)
    }
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
/// See [`ClaimError`]. Every one of them leaves the world as it was: a refused
/// claim records nothing, and the holder of a taken device is untouched.
pub(crate) fn claim(context: &mut ThreadContext, at: Address) -> Result<Device, ClaimError> {
    if !pci::is_available() {
        return Err(ClaimError::NoBus);
    }

    let at = at.function_base();
    let mut claimed = CLAIMED.lock(context);
    claimed
        .claim(at, || {
            let identity = pci::read(at).ok_or(ClaimError::NoBus)?;
            let vendor = identity as u16;
            if pci_config::is_absent(vendor) {
                return Err(ClaimError::Absent);
            }

            let layout = HeaderType::from_bits(
                (pci::read(at.at(header::HEADER_TYPE).ok_or(ClaimError::NoBus)?)
                    .ok_or(ClaimError::NoBus)?
                    >> 16) as u8,
            );
            // A bridge has two registers where a device has six; the rest of its
            // header is bus numbers, and reading those as base addresses is how a
            // walk comes to size and claim something that is not a window at all.
            let registers = if layout.is_bridge() {
                pci_config::BRIDGE_BARS
            } else {
                BARS
            };

            let windows = match pci::probe_bars(at, registers) {
                Ok(probed) => probed.into_windows(),
                Err(ProbeError::Access(()) | ProbeError::Restore(())) => {
                    return Err(ClaimError::NoBus);
                }
                Err(ProbeError::Malformed | ProbeError::InvalidWindow) => {
                    return Err(ClaimError::InvalidWindow);
                }
            };

            let mut requested = [(0u16, 0usize); BARS];
            let mut port_slots = [0usize; BARS];
            let mut port_count = 0;
            for (index, window) in windows.iter().enumerate() {
                if let Some(Window {
                    bar: Bar::Io { port },
                    size,
                }) = window
                {
                    let len = usize::try_from(*size).map_err(|_| ClaimError::InvalidWindow)?;
                    requested[port_count] = (*port, len);
                    port_slots[port_count] = index;
                    port_count += 1;
                }
            }

            let mut ports = [const { None }; BARS];
            if port_count > 0 {
                let owned =
                    wost::arch::current::port::claim_spans::<BARS>(&requested[..port_count])
                        .ok_or(ClaimError::WindowTaken)?;
                for (dense, span) in owned.into_iter().enumerate().take(port_count) {
                    ports[port_slots[dense]] = span;
                }
            }

            Ok(Device {
                at,
                vendor,
                device: (identity >> 16) as u16,
                windows,
                ports,
            })
        })
        .map_err(|error| match error {
            RegistryError::Taken => ClaimError::Taken,
            RegistryError::NoRoom => ClaimError::NoRoom,
            RegistryError::Prepare(error) => error,
        })
}

/// Whether the function at `at` is claimed.
#[cfg(feature = "selftest")]
pub(crate) fn is_claimed(at: Address) -> bool {
    CLAIMED.lock_leaf().is_claimed(at)
}

/// How many functions are claimed.
#[cfg(feature = "selftest")]
pub(crate) fn claims() -> usize {
    CLAIMED.lock_leaf().len()
}
