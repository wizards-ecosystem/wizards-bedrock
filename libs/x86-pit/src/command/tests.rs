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

/// The datasheet's read-back figure: 11 in the top bits, count latching
/// suppressed, the counter one-hot — 0xE2 is the byte for counter 0.
#[test]
fn a_status_read_back_matches_the_datasheet() {
    assert_eq!(ControlWord::read_back_status(Channel::Zero).bits(), 0xE2);
    assert_eq!(ControlWord::read_back_status(Channel::One).bits(), 0xE4);
    assert_eq!(ControlWord::read_back_status(Channel::Two).bits(), 0xE8);
}

#[test]
fn each_access_width_lands_in_bits_four_to_five() {
    for (access, value) in [
        (Access::LowByte, 0b01),
        (Access::HighByte, 0b10),
        (Access::LowHigh, 0b11),
    ] {
        let word = ControlWord::binary(Channel::Zero, access, OperatingMode::RateGenerator);
        assert_eq!((word.bits() >> 4) & 0b11, value, "{access:?}");
    }
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
