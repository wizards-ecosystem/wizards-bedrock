use super::*;

fn vector(raw: u8) -> InterruptVector {
    InterruptVector::new(raw).expect("test vector is assignable")
}

#[test]
fn enabling_packs_the_vector_below_the_enable_bit() {
    let spurious = Spurious::enabled(vector(0xFF));

    assert_eq!(spurious.bits(), 0x1FF);
    assert!(spurious.is_enabled());
    assert_eq!(spurious.bits() & 0xFF, 0xFF);
}

#[test]
fn a_register_read_back_round_trips() {
    let written = Spurious::enabled(vector(0x3F));
    let read_back = Spurious::from_bits(written.bits());

    assert_eq!(read_back, written);
    assert_eq!(read_back.bits() & 0xFF, 0x3F);
    assert!(read_back.is_enabled());
}

/// What the register holds before anything writes it.
#[test]
fn a_reset_apic_is_not_software_enabled() {
    assert!(!Spurious::from_bits(0x0000_00FF).is_enabled());
}

#[test]
#[should_panic(expected = "low four bits")]
fn a_vector_the_hardware_would_alter_is_refused() {
    let _ = Spurious::enabled(vector(0x30));
}

/// A literal anchor, since vectors ending in 0xF or 0x0 cannot tell 0x0F from 0x07.
#[test]
fn the_hardwired_low_bits_are_the_whole_nibble() {
    assert_eq!(HARDWIRED_LOW_BITS, 0b1111);
}

#[test]
#[should_panic(expected = "spurious")]
fn a_vector_whose_low_nibble_is_seven_is_refused() {
    let _ = Spurious::enabled(vector(0x37));
}
