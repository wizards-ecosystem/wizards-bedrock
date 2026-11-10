//! Publication states for local-APIC initialization.

use core::fmt;

/// The externally observable lifetime of one local controller.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Lifecycle {
    /// No hardware transition has begun.
    Cold,
    /// One caller owns an unpublished hardware transition.
    Initializing,
    /// Every source is configured and public register access is admitted.
    Ready,
    /// A hardware transition failed and cannot be safely retried.
    Poisoned,
}

/// Whether maskable interrupts are excluded at a hardware boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interrupts {
    /// The interrupt flag is clear.
    Excluded,
    /// The interrupt flag is set.
    Enabled,
}

/// One ordered boundary in the hardware transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InitStage {
    /// Enter and verify x2APIC mode.
    Mode,
    /// Validate the local controller's version register.
    Version,
    /// Mask every firmware-programmable local source.
    MaskSources,
    /// Configure every kernel-owned local source.
    ConfigureSources,
    /// Read the controller identity for the completed report.
    Identity,
}

impl InitStage {
    /// The complete hardware transaction in required order.
    pub const ALL: [Self; 5] = [
        Self::Mode,
        Self::Version,
        Self::MaskSources,
        Self::ConfigureSources,
        Self::Identity,
    ];
}

/// Why a caller cannot begin a hardware transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum BeginError {
    /// Another caller owns the unpublished transition.
    InProgress,
    /// Initialization already completed.
    Ready,
    /// A failed hardware transition made retry unsafe.
    Poisoned,
}

impl fmt::Display for BeginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InProgress => {
                core::fmt::Write::write_str(f, "local apic initialization is in progress")
            }
            Self::Ready => core::fmt::Write::write_str(f, "the local apic is already ready"),
            Self::Poisoned => {
                core::fmt::Write::write_str(f, "local apic initialization is poisoned")
            }
        }
    }
}

impl core::error::Error for BeginError {}

/// Why an initialization transaction cannot advance or publish.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ProgressError {
    /// Interrupt exclusion was lost before the next hardware boundary.
    InterruptsEnabled,
    /// A hardware boundary was entered out of order.
    OutOfOrder,
    /// Publication was attempted before every boundary completed.
    Incomplete,
    /// The transaction had already settled.
    Settled,
}

impl fmt::Display for ProgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InterruptsEnabled => {
                core::fmt::Write::write_str(f, "local apic initialization lost interrupt exclusion")
            }
            Self::OutOfOrder => {
                core::fmt::Write::write_str(f, "local apic initialization is out of order")
            }
            Self::Incomplete => {
                core::fmt::Write::write_str(f, "local apic initialization is incomplete")
            }
            Self::Settled => {
                core::fmt::Write::write_str(f, "local apic initialization already settled")
            }
        }
    }
}

impl core::error::Error for ProgressError {}

/// The hardware-free proof state used by the privileged APIC boundary.
#[derive(PartialEq, Eq, Debug)]
pub struct InitTransaction {
    lifecycle: Lifecycle,
    next_stage: usize,
}

impl Lifecycle {
    /// Recover a state stored in an atomic byte.
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Cold),
            1 => Some(Self::Initializing),
            2 => Some(Self::Ready),
            3 => Some(Self::Poisoned),
            _ => None,
        }
    }

    /// The byte stored by the hardware boundary.
    pub const fn bits(self) -> u8 {
        self as u8
    }

    /// Whether public register access may observe the controller.
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Admit the one transition that may touch cold hardware.
    pub const fn admit(self) -> Result<(), BeginError> {
        match self {
            Self::Cold => Ok(()),
            Self::Initializing => Err(BeginError::InProgress),
            Self::Ready => Err(BeginError::Ready),
            Self::Poisoned => Err(BeginError::Poisoned),
        }
    }
}

impl InitTransaction {
    /// Begin an unpublished transaction from one observed lifecycle state.
    pub const fn begin(lifecycle: Lifecycle) -> Result<Self, BeginError> {
        match lifecycle.admit() {
            Ok(()) => Ok(Self {
                lifecycle: Lifecycle::Initializing,
                next_stage: 0,
            }),
            Err(reason) => Err(reason),
        }
    }

    /// The lifecycle visible to a reentrant observer.
    pub const fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

    /// Enter the next hardware boundary while retaining interrupt exclusion.
    pub fn enter(&mut self, stage: InitStage, interrupts: Interrupts) -> Result<(), ProgressError> {
        if !matches!(self.lifecycle, Lifecycle::Initializing) {
            return Err(ProgressError::Settled);
        }
        if !progress_is_excluded(interrupts) {
            self.lifecycle = Lifecycle::Poisoned;
            return Err(ProgressError::InterruptsEnabled);
        }
        if InitStage::ALL.get(self.next_stage) != Some(&stage) {
            self.lifecycle = Lifecycle::Poisoned;
            return Err(ProgressError::OutOfOrder);
        }
        self.next_stage += 1;
        Ok(())
    }

    /// Finish a failed hardware transition without making it retryable.
    pub fn refuse(&mut self) -> Lifecycle {
        if matches!(self.lifecycle, Lifecycle::Initializing) {
            self.lifecycle = Lifecycle::Poisoned;
        }
        self.lifecycle
    }

    /// Publish readiness after the complete interrupt-excluded transaction.
    pub fn complete(&mut self, interrupts: Interrupts) -> Result<Lifecycle, ProgressError> {
        if !matches!(self.lifecycle, Lifecycle::Initializing) {
            return Err(ProgressError::Settled);
        }
        if !progress_is_excluded(interrupts) {
            self.lifecycle = Lifecycle::Poisoned;
            return Err(ProgressError::InterruptsEnabled);
        }
        if self.next_stage != InitStage::ALL.len() {
            self.lifecycle = Lifecycle::Poisoned;
            return Err(ProgressError::Incomplete);
        }
        self.lifecycle = Lifecycle::Ready;
        Ok(self.lifecycle)
    }
}

const fn progress_is_excluded(interrupts: Interrupts) -> bool {
    matches!(interrupts, Interrupts::Excluded)
}

#[cfg(test)]
mod tests;
