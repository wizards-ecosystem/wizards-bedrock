use super::*;

/// The layout QEMU and VMware both report: 8-bit channels, R high.
fn rgb888() -> PixelFormat {
    PixelFormat::new(Channel::new(16, 8), Channel::new(8, 8), Channel::new(0, 8))
        .expect("valid RGB888")
}

/// The same depth with the channels the other way round.
fn bgr888() -> PixelFormat {
    PixelFormat::new(Channel::new(0, 8), Channel::new(8, 8), Channel::new(16, 8))
        .expect("valid BGR888")
}

fn rgb565() -> PixelFormat {
    PixelFormat::new(Channel::new(11, 5), Channel::new(5, 6), Channel::new(0, 5))
        .expect("valid RGB565")
}

fn layout() -> FramebufferLayout {
    FramebufferLayout::new(0x1000, 1280, 720, 5120, 4).expect("valid layout")
}

/// A padded framebuffer spans its **pitch**, not its visible width.
///
/// Every other test here uses geometry where `pitch == width *
/// bytes_per_pixel`, so the two are interchangeable and
/// `row_bytes.checked_mul(height)` survives. On any framebuffer whose
/// rows are padded — which is most real hardware, and what `pitch` exists
/// to express — that maps short by `(pitch - row_bytes) * height` bytes,
/// and the bottom of the screen is off the end of the mapping (D-155).
#[test]
fn a_padded_framebuffer_spans_its_pitch() {
    const WIDTH: usize = 1024;
    const HEIGHT: usize = 768;
    const BYTES_PER_PIXEL: usize = 4;
    /// Deliberately more than `WIDTH * BYTES_PER_PIXEL`, which is 4096.
    const PITCH: usize = 4352;

    let layout = FramebufferLayout::new(0x1000, WIDTH, HEIGHT, PITCH, BYTES_PER_PIXEL)
        .expect("padded geometry is valid");

    assert_eq!(layout.span(), PITCH * HEIGHT, "the span is the pitch's");
    assert_ne!(
        layout.span(),
        WIDTH * BYTES_PER_PIXEL * HEIGHT,
        "the two are distinguishable here, which is the point"
    );

    // And the last visible pixel is inside that span — the property the
    // span exists to guarantee.
    let last = layout
        .offset(WIDTH - 1, HEIGHT - 1)
        .expect("the bottom-right pixel is visible");
    assert!(last + BYTES_PER_PIXEL <= layout.span());
}

#[test]
fn row_copy_uses_padded_starts_without_publishing_padding() {
    let layout = FramebufferLayout::new(0x1000, 3, 4, 20, 4).expect("padded layout");
    let mut shadow: Vec<u32> = (0..20).collect();
    let original = shadow.clone();
    let mut offsets = Vec::new();

    for (row, span) in layout
        .copy_rows_up(1, 0, usize::MAX)
        .expect("upward copy")
        .enumerate()
    {
        offsets.push(span.destination_offset());
        let source = (row + 1) * 5;
        assert_eq!(
            span.source(&shadow).expect("complete shadow"),
            &original[source..source + 3],
            "published words differ from the source row"
        );
        span.copy_within(&mut shadow).expect("complete shadow");
    }

    assert_eq!(offsets, [0, 20, 40]);
    for row in 0..3 {
        let destination = row * 5;
        let source = (row + 1) * 5;
        assert_eq!(
            &shadow[destination..destination + 3],
            &original[source..source + 3]
        );
        assert_eq!(
            &shadow[destination + 3..destination + 5],
            &original[destination + 3..destination + 5],
            "row padding changed"
        );
    }
}

#[test]
fn every_small_row_copy_stays_within_its_padded_layout() {
    for width in 1..=4 {
        for padding in 0..=3 {
            let stride = width + padding;
            for height in 1..=5 {
                let layout =
                    FramebufferLayout::new(0, width, height, stride * 4, 4).expect("small layout");
                for from_y in 0..=height + 1 {
                    for to_y in 0..=height + 1 {
                        for rows in 0..=height + 1 {
                            let Some(plan) = layout.copy_rows_up(from_y, to_y, rows) else {
                                assert!(to_y > from_y);
                                continue;
                            };
                            let expected_rows = rows
                                .min(height.saturating_sub(from_y))
                                .min(height.saturating_sub(to_y));
                            assert_eq!(plan.len(), expected_rows);

                            let mut shadow: Vec<u32> =
                                (0..stride * height).map(|word| word as u32).collect();
                            let original = shadow.clone();
                            for (row, span) in plan.enumerate() {
                                let source = (from_y + row) * stride;
                                assert_eq!(
                                    span.source(&shadow).expect("complete shadow"),
                                    &original[source..source + width]
                                );
                                assert_eq!(span.destination_offset(), (to_y + row) * stride * 4);
                                span.copy_within(&mut shadow).expect("complete shadow");
                            }

                            for row in 0..height {
                                let padding = row * stride + width..(row + 1) * stride;
                                assert_eq!(&shadow[padding.clone()], &original[padding]);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn row_copy_refuses_downward_and_extreme_empty_ranges() {
    let layout = FramebufferLayout::new(0, 2, 4, 16, 4).expect("padded layout");
    assert!(layout.copy_rows_up(0, 1, 1).is_none());
    assert_eq!(
        layout
            .copy_rows_up(usize::MAX, 0, usize::MAX)
            .expect("out-of-range copy is empty")
            .len(),
        0
    );
}

#[test]
fn row_copy_refuses_a_short_shadow_without_side_effects() {
    let layout = FramebufferLayout::new(0, 2, 4, 16, 4).expect("padded layout");
    let mut plan = layout.copy_rows_up(3, 2, usize::MAX).expect("clipped copy");
    assert_eq!(plan.len(), 1);
    let span = core::iter::Iterator::next(&mut plan).expect("one row");
    let mut short = [0_u32; 13];
    let before = short;
    assert_eq!(span.source(&short), None);
    assert_eq!(span.copy_within(&mut short), None);
    assert_eq!(short, before, "refusal changed a partial shadow");
    assert_eq!(core::iter::Iterator::next(&mut plan), None);
}

#[test]
fn from_hex_splits_the_channels() {
    assert_eq!(
        Rgb::from_hex(0x0B_10_21),
        Rgb {
            red: 0x0B,
            green: 0x10,
            blue: 0x21
        }
    );
}

#[test]
fn packing_rgb888_is_the_identity_on_a_hex_literal() {
    // The property the console relies on: on the framebuffers we actually
    // boot against, a 0xRRGGBB theme colour survives packing untouched.
    for hex in [0x00_00_00, 0x0B_10_21, 0xE8_D5_A3, 0xFF_FF_FF] {
        assert_eq!(rgb888().pack(Rgb::from_hex(hex)), hex);
    }
}

#[test]
fn packing_honours_channel_order() {
    let colour = Rgb::from_hex(0xAA_BB_CC);
    assert_eq!(rgb888().pack(colour), 0xAA_BB_CC);
    assert_eq!(bgr888().pack(colour), 0xCC_BB_AA);
}

#[test]
fn diagnostic_messages_keep_their_channel_geometry_and_values() {
    for (channel, expected) in [
        (ColourChannel::Red, "red"),
        (ColourChannel::Green, "green"),
        (ColourChannel::Blue, "blue"),
    ] {
        assert_eq!(channel.to_string(), expected);
    }

    let format_errors = [
        (
            PixelFormatError::EmptyChannel(ColourChannel::Red),
            "red channel has no bits",
        ),
        (
            PixelFormatError::ChannelOutOfRange {
                channel: ColourChannel::Green,
                shift: 31,
                size: 2,
            },
            "green channel at bit 31 with width 2 leaves a 32-bit pixel",
        ),
        (
            PixelFormatError::OverlappingChannels {
                first: ColourChannel::Red,
                second: ColourChannel::Blue,
            },
            "red and blue channels overlap",
        ),
    ];
    for (error, expected) in format_errors {
        assert_eq!(error.to_string(), expected);
    }

    let layout_errors = [
        (
            FramebufferLayoutError::ZeroWidth,
            "framebuffer width is zero",
        ),
        (
            FramebufferLayoutError::ZeroHeight,
            "framebuffer height is zero",
        ),
        (
            FramebufferLayoutError::ZeroBytesPerPixel,
            "framebuffer bytes per pixel is zero",
        ),
        (
            FramebufferLayoutError::UnsupportedBytesPerPixel(8),
            "framebuffer uses 8 bytes per pixel, expected 4",
        ),
        (
            FramebufferLayoutError::RowBytesOverflow,
            "visible row byte count overflows",
        ),
        (
            FramebufferLayoutError::PitchTooSmall {
                row_bytes: 16,
                pitch: 12,
            },
            "row needs 16 bytes but pitch is 12",
        ),
        (
            FramebufferLayoutError::MisalignedBase(0x1002),
            "framebuffer base 0x1002 is not 4-byte aligned",
        ),
        (
            FramebufferLayoutError::MisalignedPitch(6),
            "framebuffer pitch 6 is not 4-byte aligned",
        ),
        (
            FramebufferLayoutError::SpanOverflow,
            "framebuffer span overflows",
        ),
        (
            FramebufferLayoutError::AddressRangeOverflow,
            "framebuffer address range overflows",
        ),
    ];
    for (error, expected) in layout_errors {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn narrow_channels_stay_within_their_bits() {
    let packed = rgb565().pack(Rgb::from_hex(0xFF_FF_FF));
    assert_eq!(packed, 0xFFFF);
    assert_eq!(rgb565().pack(Rgb::from_hex(0x00_00_00)), 0);
}

#[test]
fn a_zero_width_channel_contributes_nothing() {
    assert_eq!(Channel::new(0, 0).encode(u8::MAX), 0);
}

#[test]
fn every_pixel_format_channel_must_be_nonempty() {
    for (red, green, blue, empty) in [
        (
            Channel::new(16, 0),
            Channel::new(8, 8),
            Channel::new(0, 8),
            ColourChannel::Red,
        ),
        (
            Channel::new(16, 8),
            Channel::new(8, 0),
            Channel::new(0, 8),
            ColourChannel::Green,
        ),
        (
            Channel::new(16, 8),
            Channel::new(8, 8),
            Channel::new(0, 0),
            ColourChannel::Blue,
        ),
    ] {
        assert_eq!(
            PixelFormat::new(red, green, blue),
            Err(PixelFormatError::EmptyChannel(empty))
        );
    }
}

#[test]
fn every_pixel_format_channel_must_fit_the_word() {
    for (red, green, blue, channel) in [
        (
            Channel::new(31, 2),
            Channel::new(8, 8),
            Channel::new(0, 8),
            ColourChannel::Red,
        ),
        (
            Channel::new(16, 8),
            Channel::new(31, 2),
            Channel::new(0, 8),
            ColourChannel::Green,
        ),
        (
            Channel::new(16, 8),
            Channel::new(8, 8),
            Channel::new(31, 2),
            ColourChannel::Blue,
        ),
    ] {
        assert_eq!(
            PixelFormat::new(red, green, blue),
            Err(PixelFormatError::ChannelOutOfRange {
                channel,
                shift: 31,
                size: 2,
            })
        );
    }
}

#[test]
fn every_pixel_format_channel_pair_must_be_disjoint() {
    for (red, green, blue, first, second) in [
        (
            Channel::new(16, 8),
            Channel::new(16, 8),
            Channel::new(0, 8),
            ColourChannel::Red,
            ColourChannel::Green,
        ),
        (
            Channel::new(16, 8),
            Channel::new(8, 8),
            Channel::new(16, 8),
            ColourChannel::Red,
            ColourChannel::Blue,
        ),
        (
            Channel::new(16, 8),
            Channel::new(8, 8),
            Channel::new(8, 8),
            ColourChannel::Green,
            ColourChannel::Blue,
        ),
    ] {
        assert_eq!(
            PixelFormat::new(red, green, blue),
            Err(PixelFormatError::OverlappingChannels { first, second })
        );
    }
}

#[test]
fn a_one_bit_channel_overlap_is_still_an_overlap() {
    assert_eq!(
        PixelFormat::new(Channel::new(0, 8), Channel::new(7, 8), Channel::new(16, 8),),
        Err(PixelFormatError::OverlappingChannels {
            first: ColourChannel::Red,
            second: ColourChannel::Green,
        })
    );
}

/// The 30-bit-colour layout real displays report — ten bits a channel in a
/// 32-bit pixel. Full intensity must reach full scale: scaling by shifting
/// left instead would stop three quarters of a step short, and white would
/// come out very slightly grey on hardware nobody tests on.
#[test]
fn a_ten_bit_channel_reaches_full_scale() {
    let xrgb2101010 = PixelFormat::new(
        Channel::new(20, 10),
        Channel::new(10, 10),
        Channel::new(0, 10),
    )
    .expect("valid 30-bit format");

    assert_eq!(xrgb2101010.pack(Rgb::from_hex(0xFF_FF_FF)), 0x3FFF_FFFF);
    assert_eq!(xrgb2101010.pack(Rgb::from_hex(0x00_00_00)), 0);
    assert_eq!(
        xrgb2101010.pack(Rgb::from_hex(0xFF_00_00)),
        0x3FF << 20,
        "red alone fills its own ten bits and nothing else"
    );
}

/// Every width, from one bit to the whole word: nothing packs to more than
/// its own bits, black is zero and white is every bit of the channel.
#[test]
fn every_channel_width_spans_exactly_its_own_bits() {
    for size in 1..=32u8 {
        let channel = Channel::new(0, size);
        let mask = if size == 32 {
            u32::MAX
        } else {
            (1u32 << size) - 1
        };

        assert_eq!(channel.encode(0), 0, "size {size} black");
        assert_eq!(channel.encode(u8::MAX), mask, "size {size} white");
        for intensity in 0..=u8::MAX {
            assert_eq!(channel.encode(intensity) & !mask, 0, "size {size} spill");
        }
    }
}

/// A brighter input can never encode darker, at any width — the property a
/// scaling bug shows up as, and the one a shift-based version only has by
/// accident.
#[test]
fn encoding_is_monotonic_at_every_width() {
    for size in 1..=16u8 {
        let channel = Channel::new(0, size);
        let mut previous = 0;
        for intensity in 0..=u8::MAX {
            let encoded = channel.encode(intensity);
            assert!(encoded >= previous, "size {size} at {intensity}");
            previous = encoded;
        }
    }
}

/// A shifted channel is the unshifted one moved, with nothing lost off
/// either end — swept over every placement that fits a 32-bit pixel.
#[test]
fn shifting_a_channel_only_moves_it() {
    for size in 1..=8u8 {
        for shift in 0..=(32 - size) {
            let placed = Channel::new(shift, size).encode(0xA7);
            let unplaced = Channel::new(0, size).encode(0xA7);

            assert_eq!(placed, unplaced << shift, "size {size} shift {shift}");
        }
    }
}

/// Nothing a bootloader can report may shift past the end of a pixel: the
/// channel drops out instead, which loses a colour rather than panicking in
/// debug or wrapping in release.
#[test]
fn a_channel_that_cannot_fit_the_word_contributes_nothing() {
    for (shift, size) in [(32, 8), (25, 8), (0, 33), (255, 255), (31, 2)] {
        assert_eq!(
            Channel::new(shift, size).encode(u8::MAX),
            0,
            "shift {shift} size {size}"
        );
    }
}

/// The boundary the case above sits next to: a channel ending exactly at
/// bit 31 is legal and must still encode.
#[test]
fn a_channel_ending_at_the_top_of_the_word_still_encodes() {
    assert_eq!(Channel::new(24, 8).encode(u8::MAX), 0xFF00_0000);
    assert_eq!(Channel::new(31, 1).encode(u8::MAX), 0x8000_0000);
    assert_eq!(Channel::new(0, 32).encode(u8::MAX), u32::MAX);
}

#[test]
fn blending_lands_on_the_endpoints() {
    let background = Rgb::from_hex(0x0B_10_21);
    let foreground = Rgb::from_hex(0xE8_D5_A3);
    assert_eq!(background.blend(foreground, 0), background);
    assert_eq!(background.blend(foreground, 255), foreground);
}

#[test]
fn blending_moves_monotonically_towards_the_foreground() {
    let background = Rgb::from_hex(0x00_00_00);
    let foreground = Rgb::from_hex(0xFF_FF_FF);
    let mut previous = 0;
    for intensity in 0..=255u8 {
        let red = background.blend(foreground, intensity).red;
        assert!(red >= previous, "intensity {intensity} went backwards");
        previous = red;
    }
    assert_eq!(previous, 255);
}

#[test]
fn checked_layout_preserves_geometry_and_bounds_offsets() {
    let layout = layout();
    assert_eq!(layout.width, 1280);
    assert_eq!(layout.height, 720);
    assert_eq!(layout.pitch, 5120);
    assert_eq!(layout.bytes_per_pixel, 4);
    assert_eq!(layout.span(), 5120 * 720);
    assert_eq!(layout.offset(1279, 719), Some(719 * 5120 + 1279 * 4));
    assert_eq!(layout.offset(1280, 0), None);
    assert_eq!(layout.offset(0, 720), None);
}

#[test]
fn a_zero_width_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, 0, 1, 4, 4),
        Err(FramebufferLayoutError::ZeroWidth)
    );
}

#[test]
fn a_zero_height_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, 1, 0, 4, 4),
        Err(FramebufferLayoutError::ZeroHeight)
    );
}

#[test]
fn a_zero_pixel_size_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, 1, 1, 4, 0),
        Err(FramebufferLayoutError::ZeroBytesPerPixel)
    );
}

#[test]
fn a_non_u32_pixel_size_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, 1, 1, 8, 8),
        Err(FramebufferLayoutError::UnsupportedBytesPerPixel(8))
    );
}

#[test]
fn a_visible_row_that_overflows_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, usize::MAX, 1, usize::MAX, 4),
        Err(FramebufferLayoutError::RowBytesOverflow)
    );
}

#[test]
fn a_pitch_shorter_than_the_visible_row_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, 2, 1, 4, 4),
        Err(FramebufferLayoutError::PitchTooSmall {
            row_bytes: 8,
            pitch: 4
        })
    );
}

/// The modulus is four, not merely even.
///
/// Include an even value not divisible by four so a weaker modulus cannot
/// satisfy the check. Otherwise later rows are misaligned and `write_u32`
/// refuses the kernel's only output (D-105).
#[test]
fn a_misaligned_mapping_base_is_refused() {
    assert_eq!(
        FramebufferLayout::new(1, 1, 1, 4, 4),
        Err(FramebufferLayoutError::MisalignedBase(1))
    );
    // Even, and still not a multiple of four.
    assert_eq!(
        FramebufferLayout::new(2, 1, 1, 4, 4),
        Err(FramebufferLayoutError::MisalignedBase(2))
    );
    assert!(FramebufferLayout::new(4, 1, 1, 4, 4).is_ok());
}

#[test]
fn a_misaligned_pitch_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, 1, 1, 5, 4),
        Err(FramebufferLayoutError::MisalignedPitch(5))
    );
    // Even, and still not a multiple of four.
    assert_eq!(
        FramebufferLayout::new(0, 1, 1, 6, 4),
        Err(FramebufferLayoutError::MisalignedPitch(6))
    );
    assert!(FramebufferLayout::new(0, 1, 1, 8, 4).is_ok());
}

#[test]
fn a_framebuffer_span_that_overflows_is_refused() {
    assert_eq!(
        FramebufferLayout::new(0, 1, usize::MAX, 8, 4),
        Err(FramebufferLayoutError::SpanOverflow)
    );
}

#[test]
fn a_framebuffer_address_range_that_wraps_is_refused() {
    assert_eq!(
        FramebufferLayout::new(usize::MAX - 3, 1, 1, 4, 4),
        Err(FramebufferLayoutError::AddressRangeOverflow)
    );
}

#[test]
fn offsets_follow_the_pitch_not_the_width() {
    // 1280 visible pixels but a 5120-byte pitch: row 1 starts at 5120.
    assert_eq!(offset(0, 0, 5120, 4), Some(0));
    assert_eq!(offset(1, 0, 5120, 4), Some(4));
    assert_eq!(offset(0, 1, 5120, 4), Some(5120));
    assert_eq!(offset(2, 3, 5120, 4), Some(3 * 5120 + 8));
}

#[test]
fn offsets_respect_a_padded_pitch() {
    // A 100-pixel row padded to 512 bytes rather than 400.
    assert_eq!(offset(0, 1, 512, 4), Some(512));
    assert_ne!(offset(0, 1, 512, 4), offset(100, 0, 512, 4));
}

#[test]
fn an_offset_with_an_overflowing_column_is_refused() {
    assert_eq!(offset(usize::MAX, 0, 0, 2), None, "column product");
}

#[test]
fn an_offset_with_an_overflowing_row_is_refused() {
    assert_eq!(offset(0, usize::MAX, 2, 0), None, "row product");
}

#[test]
fn an_offset_with_an_overflowing_sum_is_refused() {
    assert_eq!(offset(1, 1, usize::MAX - 1, 2), None, "row plus column");
}

/// A channel that exactly fills the word is accepted, and one bit more is
/// refused — at *both* copies of the bound.
///
/// Exercise the boundary in both `channel_mask` and `encode` so the
/// duplicated checks cannot drift while refusing legal RGBx8888 (D-105).
#[test]
fn a_channel_that_exactly_fills_the_word_is_accepted_by_both_copies() {
    // 24 + 8 == 32: the top channel of a standard RGBx8888 report.
    let top = Channel::new(24, 8);
    assert_ne!(top.encode(0xFF), 0, "encode refused a channel that fits");

    let format = PixelFormat::new(Channel::new(0, 8), Channel::new(8, 8), top);
    assert!(format.is_ok(), "channel_mask refused a channel that fits");

    // And one bit past the end is refused by both.
    let over = Channel::new(25, 8);
    assert_eq!(
        over.encode(0xFF),
        0,
        "encode accepted a channel that does not fit"
    );
    assert!(
        PixelFormat::new(Channel::new(0, 8), Channel::new(8, 8), over).is_err(),
        "channel_mask accepted a channel that does not fit"
    );
}

/// Blending really reads both operands.
///
/// The endpoints `0x000000` and `0xFFFFFF` make `mix(0, 255, t) == t`, so
/// the monotonicity sweep asserted `t >= t - 1` and multiplied the `from`
/// operand by literal zero every iteration — it passed with `from` dropped
/// from `mix` entirely (D-105).
#[test]
fn blending_reads_the_background_as_well_as_the_foreground() {
    let from = Rgb::from_hex(0x4080C0);
    let to = Rgb::from_hex(0xC04000);

    assert_eq!(from.blend(to, 0), from, "t = 0 is the background");
    assert_eq!(from.blend(to, 255), to, "t = 255 is the foreground");

    // Halfway depends on both: a `blend` that ignored `from` would give
    // 96 / 32 / 0 instead of 128 / 95 / 95.
    let middle = from.blend(to, 128);
    assert_eq!(middle.red, mix(0x40, 0xC0, 128));
    assert_eq!(middle.red, 0x80);
    assert_eq!(middle.green, 95);
    assert_eq!(middle.blue, 95);
}
