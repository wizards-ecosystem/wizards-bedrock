//! The 8254 control word: counter in bits 7:6, access in 5:4, mode in 3:1, BCD in 0 left clear.
//!
//! Each multi-value field is transcribed whole, since an encoding is incoherent in halves.

/// Which of the three counters the word addresses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    /// Counter 0, historically the system timer interrupt.
    Zero,
    /// Counter 1, historically memory refresh.
    One,
    /// Counter 2, wired to the speaker through a gate.
    Two,
}

impl Channel {
    const fn bits(self) -> u8 {
        (match self {
            Self::Zero => 0b00,
            Self::One => 0b01,
            Self::Two => 0b10,
        }) << 6
    }

    /// The read-back command names counters one-hot in bits 3:1 instead.
    const fn select_bit(self) -> u8 {
        match self {
            Self::Zero => 1 << 1,
            Self::One => 1 << 2,
            Self::Two => 1 << 3,
        }
    }
}

/// How the sixteen-bit count crosses the eight-bit data port.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    LowByte,
    HighByte,
    /// Low byte, then high byte, the only way to move a full count.
    LowHigh,
}

impl Access {
    const fn bits(self) -> u8 {
        (match self {
            Self::LowByte => 0b01,
            Self::HighByte => 0b10,
            Self::LowHigh => 0b11,
        }) << 4
    }
}

/// What the counter does with its count.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OperatingMode {
    /// Mode 0: count down once, raise the output at zero, keep counting.
    InterruptOnTerminalCount,
    /// Mode 1: one-shot retriggered by the gate.
    HardwareRetriggerableOneShot,
    /// Mode 2: reload on zero, one output pulse per period.
    RateGenerator,
    /// Mode 3: reload on zero, output high half the period; the speaker mode.
    SquareWave,
    /// Mode 4: one output pulse, triggered by software writing a count.
    SoftwareTriggeredStrobe,
    /// Mode 5: one output pulse, triggered by the gate.
    HardwareTriggeredStrobe,
}

impl OperatingMode {
    const fn bits(self) -> u8 {
        (match self {
            Self::InterruptOnTerminalCount => 0b000,
            Self::HardwareRetriggerableOneShot => 0b001,
            Self::RateGenerator => 0b010,
            Self::SquareWave => 0b011,
            Self::SoftwareTriggeredStrobe => 0b100,
            Self::HardwareTriggeredStrobe => 0b101,
        }) << 1
    }
}

/// One control word, ready to write to the control port.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ControlWord(u8);

impl ControlWord {
    /// Program `channel` to count in binary, in `mode`, loaded via `access`.
    pub const fn binary(channel: Channel, access: Access, mode: OperatingMode) -> Self {
        Self(channel.bits() | access.bits() | mode.bits())
    }

    /// Latch `channel`'s count for a coherent two-byte read; zero access bits make it a latch.
    pub const fn latch(channel: Channel) -> Self {
        Self(channel.bits())
    }

    /// The read-back command latching `channel`'s status, with [`crate::STATUS_NULL_COUNT`].
    pub const fn read_back_status(channel: Channel) -> Self {
        /// Bits 7:6 set: a read-back command.
        const READ_BACK: u8 = 0b11 << 6;
        /// Bit 5 set: do not latch the count.
        const SKIP_COUNT: u8 = 1 << 5;

        Self(READ_BACK | SKIP_COUNT | channel.select_bit())
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests;
