use super::*;
use crate::bar::io_window_size;

#[test]
fn a_base_address_register_decodes_to_its_window() {
    assert_eq!(Bar::decode(0, 0), Some(Bar::Absent));

    assert_eq!(Bar::decode(0xC001, 0), Some(Bar::Io { port: 0xC000 }));
    assert_eq!(Bar::decode(0xFFFF, 0), Some(Bar::Io { port: 0xFFFC }));
    assert_eq!(Bar::decode(0x1_0001, 0), None);
    assert_eq!(Bar::decode(0xFFFF_0001, 0), None);

    assert_eq!(
        Bar::decode(0xFEBF_0000, 0),
        Some(Bar::Memory {
            address: 0xFEBF_0000,
            wide: false,
            prefetchable: false,
        })
    );

    assert_eq!(
        Bar::decode(0xFEBF_000C, 0x0000_0001),
        Some(Bar::Memory {
            address: 0x1_FEBF_0000,
            wide: true,
            prefetchable: true,
        })
    );
    assert_eq!(
        Bar::decode(0xFEBF_0004, 0x0000_0001),
        Some(Bar::Memory {
            address: 0x1_FEBF_0000,
            wide: true,
            prefetchable: false,
        })
    );
}

#[test]
fn every_memory_bar_type_encoding_has_one_exact_outcome() {
    const BASE: u32 = 0x0008_0000;
    const HIGH: u32 = 0x5A;

    for memory_type in 0..=3 {
        let decoded = Bar::decode(BASE | memory_type << 1, HIGH);
        match memory_type {
            0 => assert_eq!(
                decoded,
                Some(Bar::Memory {
                    address: u64::from(BASE),
                    wide: false,
                    prefetchable: false,
                })
            ),
            1 | 3 => assert_eq!(decoded, None),
            2 => assert_eq!(
                decoded,
                Some(Bar::Memory {
                    address: u64::from(BASE) | (u64::from(HIGH) << 32),
                    wide: true,
                    prefetchable: false,
                })
            ),
            _ => unreachable!("the complete two-bit domain was exhausted"),
        }
    }
}

#[test]
fn a_memory_window_size_is_the_complement_of_the_probe() {
    for bits in 4..31u32 {
        let size = 1u32 << bits;
        let mask = !(size - 1);
        assert_eq!(
            memory_window_size(mask, 0, MemoryWidth::Bits32),
            Some(u64::from(size)),
            "a {size}-byte window"
        );
    }

    assert_eq!(
        memory_window_size(0, 0xFFFF_FFFF, MemoryWidth::Bits64),
        Some(0x1_0000_0000)
    );
    assert_eq!(
        memory_window_size(0xFFF0_0000, 0xFFFF_FFFF, MemoryWidth::Bits64),
        Some(0x10_0000)
    );
}

#[test]
fn a_narrow_window_is_not_complemented_in_a_wide_word() {
    // The same mask must be complemented in the selected register width.
    assert_eq!(
        memory_window_size(0xFFFF_C000, 0, MemoryWidth::Bits32),
        Some(0x4000)
    );
    assert!(memory_window_size(0xFFFF_C000, 0, MemoryWidth::Bits64).unwrap() > u64::from(u32::MAX));
}

#[test]
fn an_io_window_size_is_a_port_count() {
    for bits in 2..16u32 {
        let size = 1u32 << bits;
        let mask = !(size - 1);
        assert_eq!(
            io_window_size(mask),
            u16::try_from(size).ok(),
            "a {size}-port window"
        );
    }
    assert_eq!(io_window_size(0xFFFF_FFE1), Some(32));
    assert_eq!(io_window_size(0x0000_FFFC), Some(4));
    assert_eq!(io_window_size(0x0000_FFE0), Some(32));
}
