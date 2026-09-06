//! Independent bounded exhaustion of repeated-access acceptance and errors.

use mmio_range::{AccessError, AccessRange};

fn oracle(
    offset: usize,
    count: usize,
    width: usize,
    region_len: usize,
) -> Result<Vec<usize>, AccessError> {
    if width == 0 || width & (width - 1) != 0 {
        return Err(AccessError::InvalidWidth(width));
    }
    if count > 0 && offset & (width - 1) != 0 {
        return Err(AccessError::Misaligned {
            offset,
            align: width,
        });
    }

    let len = count * width;
    if offset + len > region_len {
        return Err(AccessError::OutOfBounds {
            offset,
            len,
            region_len,
        });
    }
    Ok((0..count).map(|index| offset + index * width).collect())
}

#[test]
fn every_small_repeated_access_matches_an_independent_oracle() {
    const OFFSETS: usize = 32;
    const COUNTS: usize = 8;
    const WIDTHS: usize = 65;
    const REGIONS: usize = 96;

    let mut accepted = 0usize;
    let mut invalid_width = 0usize;
    let mut misaligned = 0usize;
    let mut out_of_bounds = 0usize;

    for offset in 0..OFFSETS {
        for count in 0..COUNTS {
            for width in 0..WIDTHS {
                for region_len in 0..REGIONS {
                    let expected = oracle(offset, count, width, region_len);
                    let actual = AccessRange::repeated(offset, count, width, region_len).map(
                        |range| {
                            let mut offsets = range.offsets();
                            assert_eq!(
                                core::iter::Iterator::size_hint(&offsets),
                                (count, Some(count)),
                                "initial iterator length: offset={offset}, count={count}, width={width}, region={region_len}"
                            );
                            assert_eq!(offsets.len(), count);

                            let values: Vec<_> =
                                offsets.by_ref().take(count.saturating_add(1)).collect();
                            assert_eq!(
                                core::iter::Iterator::size_hint(&offsets),
                                (0, Some(0)),
                                "exhausted iterator length: offset={offset}, count={count}, width={width}, region={region_len}"
                            );
                            assert_eq!(offsets.len(), 0);
                            assert_eq!(core::iter::Iterator::next(&mut offsets), None);
                            assert_eq!(core::iter::Iterator::next(&mut offsets), None);
                            values
                        },
                    );
                    assert_eq!(
                        actual, expected,
                        "offset={offset}, count={count}, width={width}, region={region_len}"
                    );

                    match expected {
                        Ok(_) => accepted += 1,
                        Err(AccessError::InvalidWidth(_)) => invalid_width += 1,
                        Err(AccessError::Misaligned { .. }) => misaligned += 1,
                        Err(AccessError::OutOfBounds { .. }) => out_of_bounds += 1,
                        Err(_) => unreachable!("the bounded grid cannot overflow"),
                    }
                }
            }
        }
    }

    assert_eq!(
        accepted + invalid_width + misaligned + out_of_bounds,
        OFFSETS * COUNTS * WIDTHS * REGIONS
    );
    assert!(accepted > 0);
    assert!(invalid_width > 0);
    assert!(misaligned > 0);
    assert!(out_of_bounds > 0);
}
