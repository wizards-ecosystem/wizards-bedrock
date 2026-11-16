use core::fmt;

use super::port::{self, PortBlock};
use crate::panic::PanicToken;
use crate::sync::SpinLock;

const PRIMARY: u16 = 0x20;

const SECONDARY: u16 = 0xA0;

const REGISTER_COUNT: usize = 2;

mod register {
    pub(super) const COMMAND: usize = 0;
    pub(super) const DATA: usize = 1;
}

// The 8259A datasheet's initialisation sequence, written in order after `BEGIN`.
mod initialise {
    // ICW1: begin initialisation and promise a fourth word.
    pub(super) const BEGIN: u8 = 0x11;
    pub(super) const CASCADE_LINE: u8 = 1 << 2;
    pub(super) const CASCADE_IDENTITY: u8 = 2;
    pub(super) const MODE_8086: u8 = 0x01;
}

const PRIMARY_VECTORS: u8 = 0x20;

const SECONDARY_VECTORS: u8 = PRIMARY_VECTORS + 8;

const MASK_ALL: u8 = 0xFF;

static CONTROLLERS: SpinLock<Option<Pair>> = SpinLock::new(None);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PicError {
    PortsUnavailable,
}

impl fmt::Display for PicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PortsUnavailable => formatter.write_str("their ports are already claimed"),
        }
    }
}

impl core::error::Error for PicError {}

// Remap before masking: where firmware puts it, IRQ 0 arrives as vector 8, the double fault.
pub fn silence() -> Result<(), PicError> {
    if CONTROLLERS.lock_leaf().is_some() {
        return Ok(());
    }

    // Claim the ports before taking `CONTROLLERS`, so the two leaf locks never nest.
    let claims = port::claim_block_pair::<2, 2>(PRIMARY, SECONDARY);

    let mut controllers = CONTROLLERS.lock_leaf();
    if controllers.is_some() {
        return Ok(());
    }
    let (primary, secondary) = claims.ok_or(PicError::PortsUnavailable)?;

    let mut pair = Pair { primary, secondary };
    pair.remap();
    pair.mask_all();
    *controllers = Some(pair);
    Ok(())
}

pub(super) fn is_silenced() -> bool {
    CONTROLLERS.lock_leaf().is_some()
}

pub(crate) fn force_unlock(panic: &PanicToken) {
    CONTROLLERS.force_unlock(panic);
}

#[derive(Debug)]
struct Pair {
    primary: PortBlock<REGISTER_COUNT>,
    secondary: PortBlock<REGISTER_COUNT>,
}

impl Pair {
    // The first word clears the mask, so this runs once, with interrupts off, before `mask_all`.
    fn remap(&mut self) {
        // No delay between accesses: every machine this kernel boots emulates the pair.
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

    fn mask_all(&mut self) {
        self.primary.write(register::DATA, MASK_ALL);
        self.secondary.write(register::DATA, MASK_ALL);
    }
}
