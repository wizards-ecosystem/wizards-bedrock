//! The control word written to program or latch a counter.
//!
//! One byte selects a counter, says how its sixteen bits will be transferred
//! over an eight-bit port, and picks an operating mode. The layout is the
//! Intel 8254 datasheet's: counter in bits 7–6, access in bits 5–4, mode in
//! bits 3–1, and binary-coded decimal in bit 0, which stays zero because
//! nothing has counted in decimal since the chip was new. Counter select 11
//! is not a fourth counter but the read-back command, whose byte has its own
//! layout below.
//!
//! Each multi-value hardware field is transcribed whole — every mode, every
//! access width — because an encoding is one primitive that is incoherent in
//! halves, which is the exception the conventions carve out of the
//! no-speculative-API rule.

/// Which of the three counters the word addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// The read-back command names counters one-hot in bits 3–1 instead.
    const fn select_bit(self) -> u8 {
        match self {
            Self::Zero => 1 << 1,
            Self::One => 1 << 2,
            Self::Two => 1 << 3,
        }
    }
}

/// How the sixteen-bit count crosses the eight-bit data port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Low byte only.
    LowByte,
    /// High byte only.
    HighByte,
    /// Low byte, then high byte — the only way to move a full count.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingMode {
    /// Mode 0: count down once, raise the output at zero, keep counting.
    InterruptOnTerminalCount,
    /// Mode 1: one-shot retriggered by the gate.
    HardwareRetriggerableOneShot,
    /// Mode 2: reload on zero, one output pulse per period.
    RateGenerator,
    /// Mode 3: reload on zero, output high half the period — the speaker mode.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlWord(u8);

impl ControlWord {
    /// Program `channel` to count in binary, in `mode`, loaded via `access`.
    pub const fn binary(channel: Channel, access: Access, mode: OperatingMode) -> Self {
        Self(channel.bits() | access.bits() | mode.bits())
    }

    /// Latch `channel`'s count for a coherent two-byte read.
    ///
    /// Access bits of zero are what distinguish a latch command from a
    /// programming word; the counter keeps running while the latched value
    /// waits to be read.
    pub const fn latch(channel: Channel) -> Self {
        Self(channel.bits())
    }

    /// Latch `channel`'s status byte — the read-back command asking for
    /// status only.
    ///
    /// The read-back layout is its own: bits 7–6 both set, bit 5 clear would
    /// latch counts, bit 4 clear latches status, and counters are named
    /// one-hot in bits 3–1. The status byte read from the counter's port
    /// afterwards carries [`STATUS_NULL_COUNT`](crate::STATUS_NULL_COUNT),
    /// which is why anyone asks.
    pub const fn read_back_status(channel: Channel) -> Self {
        /// Bits 7–6 set: this is a read-back, not a programming word.
        const READ_BACK: u8 = 0b11 << 6;
        /// Bit 5 set: do not latch the count.
        const SKIP_COUNT: u8 = 1 << 5;

        Self(READ_BACK | SKIP_COUNT | channel.select_bit())
    }

    /// The byte to write.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests;
