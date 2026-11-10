//! The legacy 8259 pair, silenced before interrupts are enabled since firmware left it wired here.
//!
//! Remapped before masked: where firmware puts it, IRQ 0 arrives as vector 8, the double fault.

use core::fmt;

use super::port::{self, PortBlock};
use crate::panic::PanicToken;
use crate::sync::SpinLock;

/// The controller wired directly to the CPU.
const PRIMARY: u16 = 0x20;

/// The controller cascaded onto the primary.
const SECONDARY: u16 = 0xA0;

const REGISTER_COUNT: usize = 2;

mod register {
    /// The first initialisation word and operation commands; reads request and in-service state.
    pub(super) const COMMAND: usize = 0;
    /// The rest of the initialisation sequence, then the interrupt mask.
    pub(super) const DATA: usize = 1;
}

/// The 8259A datasheet's "Programming the 8259A" sequence, whose first word commits the controller
/// to receiving the rest, in order, on the data port.
mod initialise {
    /// Begin initialisation, and promise a fourth word.
    pub(super) const BEGIN: u8 = 0x11;
    /// The primary's input line the secondary cascades onto, as a bitmap: IRQ 2.
    pub(super) const CASCADE_LINE: u8 = 1 << 2;
    /// The same cascade line, as the secondary's number.
    pub(super) const CASCADE_IDENTITY: u8 = 2;
    /// 8086 mode, which is what makes the vector offsets mean anything.
    pub(super) const MODE_8086: u8 = 0x01;
}

/// Immediately above the architectural exceptions.
const PRIMARY_VECTORS: u8 = 0x20;

const SECONDARY_VECTORS: u8 = PRIMARY_VECTORS + 8;

const MASK_ALL: u8 = 0xFF;

/// `None` until [`silence`] has run, which makes this the record of whether it has.
static CONTROLLERS: SpinLock<Option<Pair>> = SpinLock::new(None);

/// Why the legacy controllers could not be silenced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PicError {
    /// Something else already owns their ports.
    PortsUnavailable,
}

impl fmt::Display for PicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortsUnavailable => {
                core::fmt::Write::write_str(f, "their ports are already claimed")
            }
        }
    }
}

impl core::error::Error for PicError {}

/// Remap both controllers clear of the exception vectors and mask every line.
///
/// Does nothing after the first successful call. Claims both port blocks before taking
/// `CONTROLLERS`, preserving leaf-lock order and all-or-none port ownership.
pub fn silence() -> Result<(), PicError> {
    if CONTROLLERS.lock_leaf().is_some() {
        return Ok(());
    }

    // The sole caller runs before scheduling; the second check still resolves a claim lost to a
    // publication just before it.
    let claims = port::claim_block_pair::<2, 2>(PRIMARY, SECONDARY);

    let mut controllers = CONTROLLERS.lock_leaf();
    if controllers.is_some() {
        // A successful pair cannot reach here: the published owner holds the same permanent claims.
        return Ok(());
    }
    let (primary, secondary) = claims.ok_or(PicError::PortsUnavailable)?;

    let mut pair = Pair { primary, secondary };
    pair.remap();
    pair.mask_all();
    *controllers = Some(pair);
    Ok(())
}

/// Whether [`silence`] has run to completion.
pub(super) fn is_silenced() -> bool {
    CONTROLLERS.lock_leaf().is_some()
}

/// Each controller's interrupt mask, or `None` before [`silence`] has claimed them.
///
/// Read back from the hardware, since the point of asking is whether the write landed.
#[cfg(feature = "selftest")]
pub fn masks() -> Option<(u8, u8)> {
    let mut controllers = CONTROLLERS.lock_leaf();
    let pair = controllers.as_mut()?;
    Some((
        PortBlock::read(&mut pair.primary, register::DATA),
        PortBlock::read(&mut pair.secondary, register::DATA),
    ))
}

/// Reclaim this module's lock for [`panic::reclaim`](crate::panic::reclaim), since a fault inside
/// [`silence`] would otherwise leave it held for ever.
pub(crate) fn force_unlock(panic: &PanicToken) {
    CONTROLLERS.force_unlock(panic);
}

#[derive(Debug)]
struct Pair {
    primary: PortBlock<REGISTER_COUNT>,
    secondary: PortBlock<REGISTER_COUNT>,
}

impl Pair {
    /// Walk both controllers through initialisation, moving their vectors clear of the exceptions.
    ///
    /// The first word clears the mask, leaving every firmware line unmasked until `mask_all`, so
    /// this must run with the interrupt flag clear, before anything sets it, and never again.
    /// No delay between accesses: the pair is emulated by the chipset or hypervisor, so each `out`
    /// is already a completed transaction.
    fn remap(&mut self) {
        for (controller, vectors, cascade) in [
            (&mut self.primary, PRIMARY_VECTORS, initialise::CASCADE_LINE),
            (
                &mut self.secondary,
                SECONDARY_VECTORS,
                initialise::CASCADE_IDENTITY,
            ),
        ] {
            PortBlock::write(controller, register::COMMAND, initialise::BEGIN);
            PortBlock::write(controller, register::DATA, vectors);
            PortBlock::write(controller, register::DATA, cascade);
            PortBlock::write(controller, register::DATA, initialise::MODE_8086);
        }
    }

    fn mask_all(&mut self) {
        PortBlock::write(&mut self.primary, register::DATA, MASK_ALL);
        PortBlock::write(&mut self.secondary, register::DATA, MASK_ALL);
    }
}
