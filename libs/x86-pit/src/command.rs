//! The control word written to program or latch a counter.
//!
//! One byte selects a counter, says how its sixteen bits will be transferred
//! over an eight-bit port, and picks an operating mode. The layout is the
//! Intel 8254 datasheet's: counter in bits 7–6, access in bits 5–4, mode in
//! bits 3–1, and binary-coded decimal in bit 0, which stays zero because
//! nothing has counted in decimal since the chip was new.

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

    /// The byte to write.
    pub const fn bits(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The word the kernel calibrates with, checked as a whole byte against
    /// the datasheet's figure: counter 0, low-then-high, mode 0, binary.
    #[test]
    fn the_calibration_word_is_0x30() {
        let word = ControlWord::binary(
            Channel::Zero,
            Access::LowHigh,
            OperatingMode::InterruptOnTerminalCount,
        );

        assert_eq!(word.bits(), 0x30);
    }

    /// The byte every PC manual prints for the speaker — counter 2, square
    /// wave — which cross-checks all three fields at once against forty years
    /// of published listings.
    #[test]
    fn the_classic_speaker_word_is_0xb6() {
        let word = ControlWord::binary(Channel::Two, Access::LowHigh, OperatingMode::SquareWave);

        assert_eq!(word.bits(), 0xB6);
    }

    #[test]
    fn a_latch_command_names_only_its_channel() {
        assert_eq!(ControlWord::latch(Channel::Zero).bits(), 0x00);
        assert_eq!(ControlWord::latch(Channel::One).bits(), 0x40);
        assert_eq!(ControlWord::latch(Channel::Two).bits(), 0x80);
    }

    #[test]
    fn binary_counting_keeps_the_bcd_bit_clear() {
        for mode in [
            OperatingMode::InterruptOnTerminalCount,
            OperatingMode::HardwareRetriggerableOneShot,
            OperatingMode::RateGenerator,
            OperatingMode::SquareWave,
            OperatingMode::SoftwareTriggeredStrobe,
            OperatingMode::HardwareTriggeredStrobe,
        ] {
            let word = ControlWord::binary(Channel::Zero, Access::LowHigh, mode);
            assert_eq!(word.bits() & 1, 0, "{mode:?}");
        }
    }

    #[test]
    fn each_mode_lands_in_bits_one_to_three() {
        for (mode, value) in [
            (OperatingMode::InterruptOnTerminalCount, 0),
            (OperatingMode::HardwareRetriggerableOneShot, 1),
            (OperatingMode::RateGenerator, 2),
            (OperatingMode::SquareWave, 3),
            (OperatingMode::SoftwareTriggeredStrobe, 4),
            (OperatingMode::HardwareTriggeredStrobe, 5),
        ] {
            let word = ControlWord::binary(Channel::Zero, Access::LowByte, mode);
            assert_eq!((word.bits() >> 1) & 0b111, value, "{mode:?}");
        }
    }
}
