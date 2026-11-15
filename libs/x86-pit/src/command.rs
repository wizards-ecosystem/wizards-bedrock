#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Zero,
}

impl Channel {
    const fn bits(self) -> u8 {
        (match self {
            Self::Zero => 0b00,
        }) << 6
    }

    // The read-back command names counters one-hot in bits 3:1.
    const fn select_bit(self) -> u8 {
        match self {
            Self::Zero => 1 << 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    LowHigh,
}

impl Access {
    const fn bits(self) -> u8 {
        (match self {
            Self::LowHigh => 0b11,
        }) << 4
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OperatingMode {
    InterruptOnTerminalCount,
}

impl OperatingMode {
    const fn bits(self) -> u8 {
        (match self {
            Self::InterruptOnTerminalCount => 0b000,
        }) << 1
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ControlWord(u8);

impl ControlWord {
    pub const fn binary(channel: Channel, access: Access, mode: OperatingMode) -> Self {
        Self(channel.bits() | access.bits() | mode.bits())
    }

    // Zero access bits make the word a latch command.
    pub const fn latch(channel: Channel) -> Self {
        Self(channel.bits())
    }

    pub const fn read_back_status(channel: Channel) -> Self {
        const READ_BACK: u8 = 0b11 << 6;
        const SKIP_COUNT: u8 = 1 << 5;

        Self(READ_BACK | SKIP_COUNT | channel.select_bit())
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}
