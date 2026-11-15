use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Lifecycle {
    Cold,
    Initializing,
    Ready,
    Poisoned,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Interrupts {
    Excluded,
    Enabled,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InitStage {
    Mode,
    Version,
    MaskSources,
    ConfigureSources,
    Identity,
}

impl InitStage {
    pub const ALL: [Self; 5] = [
        Self::Mode,
        Self::Version,
        Self::MaskSources,
        Self::ConfigureSources,
        Self::Identity,
    ];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum BeginError {
    InProgress,
    Ready,
    Poisoned,
}

impl fmt::Display for BeginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InProgress => "local apic initialization is in progress",
            Self::Ready => "the local apic is already ready",
            Self::Poisoned => "local apic initialization is poisoned",
        })
    }
}

impl core::error::Error for BeginError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ProgressError {
    InterruptsEnabled,
    OutOfOrder,
    Incomplete,
    Settled,
}

impl fmt::Display for ProgressError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InterruptsEnabled => "local apic initialization lost interrupt exclusion",
            Self::OutOfOrder => "local apic initialization is out of order",
            Self::Incomplete => "local apic initialization is incomplete",
            Self::Settled => "local apic initialization already settled",
        })
    }
}

impl core::error::Error for ProgressError {}

#[derive(PartialEq, Eq, Debug)]
pub struct InitTransaction {
    lifecycle: Lifecycle,
    next_stage: usize,
}

impl Lifecycle {
    pub const fn from_bits(bits: u8) -> Option<Self> {
        match bits {
            0 => Some(Self::Cold),
            1 => Some(Self::Initializing),
            2 => Some(Self::Ready),
            3 => Some(Self::Poisoned),
            _ => None,
        }
    }

    pub const fn bits(self) -> u8 {
        self as u8
    }

    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

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
    pub const fn begin(lifecycle: Lifecycle) -> Result<Self, BeginError> {
        match lifecycle.admit() {
            Ok(()) => Ok(Self {
                lifecycle: Lifecycle::Initializing,
                next_stage: 0,
            }),
            Err(reason) => Err(reason),
        }
    }

    pub const fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

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

    pub fn refuse(&mut self) -> Lifecycle {
        if matches!(self.lifecycle, Lifecycle::Initializing) {
            self.lifecycle = Lifecycle::Poisoned;
        }
        self.lifecycle
    }

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
