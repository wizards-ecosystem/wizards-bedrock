//! Publication states for local-APIC initialization.

use core::fmt;

/// The externally observable lifetime of one local controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupts {
    /// The interrupt flag is clear.
    Excluded,
    /// The interrupt flag is set.
    Enabled,
}

/// One ordered boundary in the hardware transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
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
mod tests {
    use super::*;

    #[test]
    fn every_stored_state_round_trips() {
        for state in [
            Lifecycle::Cold,
            Lifecycle::Initializing,
            Lifecycle::Ready,
            Lifecycle::Poisoned,
        ] {
            assert_eq!(Lifecycle::from_bits(state.bits()), Some(state));
        }
        assert_eq!(Lifecycle::from_bits(4), None);
        assert_eq!(Lifecycle::from_bits(u8::MAX), None);
    }

    #[test]
    fn only_cold_hardware_can_begin() {
        assert_eq!(Lifecycle::Cold.admit(), Ok(()));
        assert_eq!(Lifecycle::Initializing.admit(), Err(BeginError::InProgress));
        assert_eq!(Lifecycle::Ready.admit(), Err(BeginError::Ready));
        assert_eq!(Lifecycle::Poisoned.admit(), Err(BeginError::Poisoned));
    }

    #[test]
    fn every_late_refusal_is_unpublished_and_terminal() {
        for failed_stage in 0..=InitStage::ALL.len() {
            let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();

            for &stage in &InitStage::ALL[..failed_stage] {
                assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
                assert!(!transaction.lifecycle().is_ready());
            }

            assert_eq!(transaction.refuse(), Lifecycle::Poisoned);
            assert!(!transaction.lifecycle().is_ready());
            assert_eq!(
                InitTransaction::begin(transaction.lifecycle()),
                Err(BeginError::Poisoned),
                "failed stage {failed_stage}"
            );
        }
    }

    #[test]
    fn every_intermediate_observation_refuses_reentry_and_readiness() {
        let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();

        for &stage in &InitStage::ALL {
            assert!(!transaction.lifecycle().is_ready());
            assert_eq!(
                InitTransaction::begin(transaction.lifecycle()),
                Err(BeginError::InProgress)
            );
            assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
        }
        assert!(!transaction.lifecycle().is_ready());
        assert_eq!(
            InitTransaction::begin(transaction.lifecycle()),
            Err(BeginError::InProgress)
        );
        assert_eq!(
            transaction.complete(Interrupts::Excluded),
            Ok(Lifecycle::Ready)
        );
        assert!(transaction.lifecycle().is_ready());
    }

    #[test]
    fn every_hardware_boundary_and_publication_require_interrupt_exclusion() {
        for lost_at in 0..=InitStage::ALL.len() {
            let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();

            for &stage in &InitStage::ALL[..lost_at] {
                assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
            }

            let refusal = if let Some(&stage) = InitStage::ALL.get(lost_at) {
                transaction.enter(stage, Interrupts::Enabled)
            } else {
                transaction.complete(Interrupts::Enabled).map(|_| ())
            };
            assert_eq!(refusal, Err(ProgressError::InterruptsEnabled));
            assert_eq!(transaction.lifecycle(), Lifecycle::Poisoned);
        }
    }

    #[test]
    fn publication_requires_every_stage_in_exact_order() {
        for completed in 0..InitStage::ALL.len() {
            let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();
            for &stage in &InitStage::ALL[..completed] {
                assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
            }
            assert_eq!(
                transaction.complete(Interrupts::Excluded),
                Err(ProgressError::Incomplete),
                "completed stages {completed}"
            );
            assert_eq!(transaction.lifecycle(), Lifecycle::Poisoned);
        }

        for expected_at in 0..InitStage::ALL.len() {
            for wrong in InitStage::ALL
                .iter()
                .copied()
                .filter(|stage| Some(stage) != InitStage::ALL.get(expected_at))
            {
                let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();
                for &stage in &InitStage::ALL[..expected_at] {
                    assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
                }
                assert_eq!(
                    transaction.enter(wrong, Interrupts::Excluded),
                    Err(ProgressError::OutOfOrder),
                    "expected stage {expected_at}, got {wrong:?}"
                );
                assert_eq!(transaction.lifecycle(), Lifecycle::Poisoned);
            }
        }
    }
}
