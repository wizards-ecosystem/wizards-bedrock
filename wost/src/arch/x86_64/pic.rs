//! The legacy 8259 interrupt controllers.
//!
//! There is nothing to drive here. Interrupts arrive through the local APIC,
//! and the only thing the cascaded pair of 8259s is ever asked to do is stop
//! delivering — which is a prerequisite for enabling interrupts at all, because
//! whatever the firmware left them programmed with is still wired to this CPU.
//!
//! They are remapped before they are masked. A controller left where the
//! firmware puts it delivers IRQ 0 as vector 8, which is the double fault: an
//! interrupt that leaks through would arrive as the one exception whose report
//! is guaranteed to be a lie.

use core::fmt;

use super::port::{self, PortBlock};
use crate::panic::PanicToken;
use crate::sync::SpinLock;

/// Command and data port of the controller wired directly to the CPU.
const PRIMARY: u16 = 0x20;

/// Command and data port of the one cascaded onto it.
const SECONDARY: u16 = 0xA0;

/// Ports each controller occupies: a command register and a data register.
const REGISTER_COUNT: usize = 2;

/// Register offsets from a controller's base port.
mod register {
    /// Takes the initialisation sequence's first word and the operation
    /// commands; reads back the request and in-service registers.
    pub(super) const COMMAND: usize = 0;
    /// Takes the rest of the initialisation sequence, and the interrupt mask
    /// once it is over.
    pub(super) const DATA: usize = 1;
}

/// The four-word initialisation sequence, Intel 8259A datasheet §"Programming
/// the 8259A". Writing the first commits the controller to receiving the rest,
/// in order, on the data port.
mod initialise {
    /// Begin initialisation, and promise a fourth word.
    pub(super) const BEGIN: u8 = 0x11;
    /// The primary's second-word input line the secondary cascades onto,
    /// as a bitmap: IRQ 2.
    pub(super) const CASCADE_LINE: u8 = 1 << 2;
    /// The secondary's second word says the same thing as a number.
    pub(super) const CASCADE_IDENTITY: u8 = 2;
    /// Fourth word: 8086 mode, which is what makes the vector offsets above
    /// mean anything.
    pub(super) const MODE_8086: u8 = 0x01;
}

/// Where the primary's eight lines are remapped to — immediately above the
/// architectural exceptions, which is where every x86 kernel puts them.
const PRIMARY_VECTORS: u8 = 0x20;

/// Where the secondary's eight follow.
const SECONDARY_VECTORS: u8 = PRIMARY_VECTORS + 8;

/// Every line masked.
const MASK_ALL: u8 = 0xFF;

/// The controllers, once claimed. `None` until [`silence`] has run, which makes
/// this the record of whether it has.
static CONTROLLERS: SpinLock<Option<Pair>> = SpinLock::new(None);

/// Why the legacy controllers could not be silenced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PicError {
    /// Something else already owns their ports.
    PortsUnavailable,
}

impl fmt::Display for PicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortsUnavailable => f.write_str("their ports are already claimed"),
        }
    }
}

impl core::error::Error for PicError {}

/// Remap both controllers clear of the exception vectors and mask every line.
///
/// Does nothing after the first successful call.
pub fn silence() -> Result<(), PicError> {
    let mut controllers = CONTROLLERS.lock();
    if controllers.is_some() {
        return Ok(());
    }

    let mut pair = Pair {
        primary: port::claim_block(PRIMARY).ok_or(PicError::PortsUnavailable)?,
        secondary: port::claim_block(SECONDARY).ok_or(PicError::PortsUnavailable)?,
    };
    pair.remap();
    pair.mask_all();
    *controllers = Some(pair);
    Ok(())
}

/// Whether [`silence`] has run to completion.
pub fn is_silenced() -> bool {
    CONTROLLERS.lock().is_some()
}

/// The interrupt mask each controller holds, read back from the hardware, or
/// `None` before [`silence`] has claimed them.
///
/// Read back rather than remembered: the point of asking is to find out whether
/// the write landed.
pub fn masks() -> Option<(u8, u8)> {
    let mut controllers = CONTROLLERS.lock();
    let pair = controllers.as_mut()?;
    Some((
        pair.primary.read(register::DATA),
        pair.secondary.read(register::DATA),
    ))
}

/// Reclaim this module's lock on the terminal panic path.
///
/// Called by [`panic::reclaim`](crate::panic::reclaim). A fault raised inside
/// [`silence`] would otherwise leave the lock held for ever, and the panic path
/// is entitled to look at anything.
pub(crate) fn force_unlock(panic: &PanicToken) {
    CONTROLLERS.force_unlock(panic);
}

/// Both controllers.
#[derive(Debug)]
struct Pair {
    primary: PortBlock<REGISTER_COUNT>,
    secondary: PortBlock<REGISTER_COUNT>,
}

impl Pair {
    /// Walk both controllers through the initialisation sequence, moving their
    /// vectors clear of the architectural exceptions.
    ///
    /// The first word of the sequence clears the interrupt mask, so for the
    /// length of this function every line the firmware left connected is
    /// unmasked. That is harmless only because it runs with the interrupt flag
    /// clear — this is called before anything sets it, and never again.
    ///
    /// The classic delay between accesses is deliberately absent. It covers the
    /// bus recovery time of a discrete 8259A; on every machine this kernel
    /// boots, the pair is emulated by the chipset or the hypervisor and each
    /// `out` is already a completed transaction.
    fn remap(&mut self) {
        for (controller, vectors, cascade) in [
            (&mut self.primary, PRIMARY_VECTORS, initialise::CASCADE_LINE),
            (
                &mut self.secondary,
                SECONDARY_VECTORS,
                initialise::CASCADE_IDENTITY,
            ),
        ] {
            controller.write(register::COMMAND, initialise::BEGIN);
            controller.write(register::DATA, vectors);
            controller.write(register::DATA, cascade);
            controller.write(register::DATA, initialise::MODE_8086);
        }
    }

    /// Disconnect every input line on both controllers.
    fn mask_all(&mut self) {
        self.primary.write(register::DATA, MASK_ALL);
        self.secondary.write(register::DATA, MASK_ALL);
    }
}
