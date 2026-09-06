//! Exhaustive bounded oracle for device-window gap selection.

use mmio_range::{AddressRange, first_fit_by};

fn oracle(
    window: AddressRange,
    len: u64,
    align: u64,
    occupied: &[Option<AddressRange>],
) -> Option<u64> {
    (window.base()..window.end()).find(|base| {
        let base = *base;
        base.is_multiple_of(align)
            && base.checked_add(len).is_some_and(|end| end <= window.end())
            && occupied.iter().flatten().all(|range| {
                let end = base + len;
                end <= range.base() || range.end() <= base
            })
    })
}

#[test]
fn every_small_first_fit_matches_a_bytewise_oracle() {
    const LIMIT: u64 = 8;
    let mut ranges = vec![None];
    for base in 0..LIMIT {
        for end in base + 1..=LIMIT {
            ranges.push(Some(AddressRange::new(base, end - base).expect("range")));
        }
    }

    let mut checked = 0usize;
    for window_base in 0..=3 {
        let window = AddressRange::new(window_base, LIMIT - window_base).expect("window");
        for len in 1..=LIMIT {
            for align in [1, 2, 4, 8] {
                for &left in &ranges {
                    for &right in &ranges {
                        let occupied = [left, right];
                        assert_eq!(
                            first_fit_by(window, len, align, &occupied, |range| *range),
                            oracle(window, len, align, &occupied),
                            "window={window:?}, len={len}, align={align}, occupied={occupied:?}"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }

    assert_eq!(checked, 175_232);
}

#[test]
fn a_zero_length_or_unaligned_request_is_refused_before_any_search() {
    let window = AddressRange::new(0, 64).expect("window");
    let occupied: [Option<AddressRange>; 0] = [];
    assert_eq!(first_fit_by(window, 0, 8, &occupied, |range| *range), None);
    assert_eq!(first_fit_by(window, 8, 0, &occupied, |range| *range), None);
    assert_eq!(first_fit_by(window, 8, 3, &occupied, |range| *range), None);
    assert_eq!(
        first_fit_by(window, 8, 8, &occupied, |range| *range),
        Some(0)
    );
}
