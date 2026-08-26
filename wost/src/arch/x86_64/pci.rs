//! Configuration space, through the two ports that have been there since 1992.
//!
//! The instruction half of PCI. `libs/pci-config` owns every number — the
//! address encoding, the header offsets, the class codes, the BAR decoding —
//! and what is here is the pair of ports and the two accesses that drive them.
//!
//! **CF8/CFC rather than ECAM**, and not out of nostalgia. ECAM needs an MCFG
//! table; QEMU's i440fx publishes none and VMware emulates a 440BX, so this is
//! the only mechanism guaranteed present on **both** boot targets D-18 makes
//! this project answer to. It reaches every byte a read-only driver needs, and
//! an ECAM path later is purely additive (`012` §2).
//!
//! **The two ports are claimed together or not at all.** A configuration
//! access is a write to one and a read from the other, so a claim that took
//! the address port and lost the data port would have foreclosed the first for
//! the life of the boot and left the mechanism unbuildable — which is exactly
//! what D-138 made `claim_pair` for.
//!
//! **The pair is held under one lock for the whole access.** Address and data
//! are two instructions with a window between them, and a second caller that
//! wrote its own address into that window would be answered with somebody
//! else's register. On one CPU the guard's cleared interrupt flag is what
//! closes it; on two it will be the lock itself.

use pci_config::Address;

use crate::sync::SpinLock;

use super::port::{self, Port};

/// The address port: which register the next data access names.
const ADDRESS_PORT: u16 = 0x0CF8;

/// The data port: the register itself.
const DATA_PORT: u16 = 0x0CFC;

/// The claimed pair, once. `None` until [`init`], and for good if it declined.
static PORTS: SpinLock<Option<(Port<u32>, Port<u32>)>> = SpinLock::new(None);

/// Claim the two ports, or report that something else already has them.
///
/// Idempotent: a second caller finds the pair already claimed and returns
/// `true` having done nothing, because the question it is really asking is
/// "can configuration space be reached", and it can.
pub fn init() -> bool {
    if is_available() {
        return true;
    }
    // The claim happens **outside** this module's guard, and it has to: the
    // port registry has a leaf of its own, and taking it under this one is the
    // nesting `lock_leaf` forbids. The first version did exactly that and the
    // machine check caught it on its first boot (D-168).
    let Some(pair) = port::claim_pair::<u32, u32>(ADDRESS_PORT, DATA_PORT) else {
        // Either another caller of this function got there first — in which
        // case configuration space is reachable and the answer is yes — or
        // something that is not this module holds the ports, and it is no.
        return is_available();
    };

    let mut ports = PORTS.lock_leaf();
    if ports.is_none() {
        *ports = Some(pair);
    }
    true
}

/// Whether configuration space can be reached at all.
pub fn is_available() -> bool {
    PORTS.lock_leaf().is_some()
}

/// Read the dword at `at`.
///
/// `None` only when the ports were never claimed. **An absent function is not
/// an error**: the bus answers all ones for one that is not there, so
/// `0xFFFF_FFFF` is a value and `pci_config::is_absent` is what reads it.
pub fn read(at: Address) -> Option<u32> {
    let mut ports = PORTS.lock_leaf();
    let (address, data) = ports.as_mut()?;
    address.write(at.bits());
    Some(data.read())
}

/// Write `value` to the dword at `at`.
///
/// `false` only when the ports were never claimed.
///
/// Used for two things and no others: setting the command register's bus
/// master and memory-space bits, and the all-ones probe that asks a base
/// address register how large its window is. Both restore what they found.
pub fn write(at: Address, value: u32) -> bool {
    let mut ports = PORTS.lock_leaf();
    let Some((address, data)) = ports.as_mut() else {
        return false;
    };
    address.write(at.bits());
    data.write(value);
    true
}

/// Ask a base-address register how large its window is, and put it back.
///
/// The protocol from the specification: write all ones, read the mask back,
/// restore the original. It is here rather than in the caller because the
/// restore is the part that must not be forgotten — a register left holding
/// `0xFFFF_FFFF` is a device claiming a four-gigabyte window at the top of
/// memory, and the next thing to enumerate the bus believes it.
///
/// Returns the original value and the mask, or `None` if the ports were never
/// claimed. The whole sequence runs under **one** hold of the pair, so nothing
/// can read the register while it is holding the probe value.
pub fn probe_window(at: Address) -> Option<(u32, u32)> {
    let mut ports = PORTS.lock_leaf();
    let (address, data) = ports.as_mut()?;

    address.write(at.bits());
    let original = data.read();

    address.write(at.bits());
    data.write(u32::MAX);

    address.write(at.bits());
    let mask = data.read();

    address.write(at.bits());
    data.write(original);

    Some((original, mask))
}
