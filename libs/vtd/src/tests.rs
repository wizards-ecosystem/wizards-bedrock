use super::*;

#[test]
fn version_bounds_the_register_invalidation_interface() {
    assert_eq!(VERSION, 0);
    assert_eq!(Version::from_bits(0x51).bits(), 0x51);
    assert_eq!(Version::from_bits(0x51).major(), 5);
    assert_eq!(Version::from_bits(0x51).minor(), 1);
    for major in 1..=5 {
        assert!(Version::from_bits(major << 4).register_invalidation_supported());
    }
    assert!(!Version::from_bits(0).register_invalidation_supported());
    assert!(!Version::from_bits(6 << 4).register_invalidation_supported());
    assert!(!Version::from_bits(15 << 4).register_invalidation_supported());
}

#[test]
fn register_offsets_are_the_spec_byte_offsets() {
    assert_eq!(CAP, 0x08);
    assert_eq!(ECAP, 0x10);
    assert_eq!(GCMD, 0x18);
    assert_eq!(GSTS, 0x1C);
    assert_eq!(RTADDR, 0x20);
    assert_eq!(CCMD, 0x28);
    assert_eq!(FSTS, 0x34);
    assert_eq!(FECTL, 0x38);
}

#[test]
fn fault_event_control_masks_the_non_remapped_message_source() {
    assert_eq!(fault_event_control::PENDING, 1 << 30);
    assert_eq!(fault_event_control::MASK, 1 << 31);
    assert!(!fault_event_control::masked(0));
    assert!(!fault_event_control::masked(1 << 30));
    assert!(fault_event_control::masked(1 << 31));
    assert!(fault_event_control::masked((1 << 31) | (1 << 30)));
}

/// The address field is bits 63:12 (52 bits), independent of the constant.
#[test]
fn the_entry_address_field_is_bits_63_through_12() {
    assert_eq!(ADDRESS_MASK, !0xFFFu64);
    assert_eq!(ADDRESS_MASK.trailing_zeros(), 12);
    assert_eq!(ADDRESS_MASK.count_ones(), 52);
}

#[test]
fn the_root_table_address_drops_the_mode_and_reserved_bits() {
    assert_eq!(
        root_table_address(0x1234_5000 | 0b1100_0000_0000),
        0x1234_5000
    );
    assert_eq!(root_table_address(0x1234_5FFF), 0x1234_5000);
    assert_eq!(root_table_address(0), 0);
}

/// Capability field positions, restated from the spec rather than from the
/// constants above.
#[test]
fn capability_field_positions_match_the_spec() {
    assert_eq!(Capability::ND_MASK, 0x7, "ND bits 2:0");
    assert_eq!(Capability::RWBF, 1 << 4);
    assert_eq!(Capability::CM, 1 << 7);
    assert_eq!(Capability::SAGAW_SHIFT, 8);
    assert_eq!(Capability::SAGAW_MASK, 0x1F, "SAGAW bits 12:8");
    assert_eq!(Capability::MGAW_SHIFT, 16);
    assert_eq!(Capability::MGAW_MASK, 0x3F, "MGAW bits 21:16");
    assert_eq!(Capability::FRO_SHIFT, 24);
    assert_eq!(Capability::FRO_MASK, 0x3FF, "FRO bits 33:24");
    assert_eq!(Capability::NFR_SHIFT, 40);
    assert_eq!(Capability::NFR_MASK, 0xFF, "NFR bits 47:40");
    assert_eq!(Capability::DWD, 1 << 54);
    assert_eq!(Capability::DRD, 1 << 55);
}

#[test]
fn capability_decodes_each_field_from_its_own_bits() {
    // Pairwise-distinct field values placed with independent literal shifts.
    let cap = Capability::from_bits(
        5 | (1 << 4)
            | (1 << 7)
            | (4 << 8)
            | (47 << 16)
            | (0x123u64 << 24)
            | (0x12u64 << 40)
            | (1 << 54)
            | (1 << 55),
    );
    assert_eq!(cap.number_of_domains(), 1 << (4 + 2 * 5)); // ND = 5
    assert!(cap.requires_write_buffer_flush());
    assert!(cap.caching_mode());
    assert!(cap.sagaw_supports(4));
    assert!(!cap.sagaw_supports(3));
    assert!(!cap.sagaw_supports(5));
    assert!(!cap.sagaw_supports(2));
    assert_eq!(cap.max_guest_address_width(), 48); // MGAW field 47
    assert_eq!(cap.fault_recording_offset_bytes(), 0x123 * 16);
    assert_eq!(cap.number_of_fault_recording_registers(), 0x12 + 1);
    assert!(cap.write_draining());
    assert!(cap.read_draining());
    assert!(cap.dma_draining());
    assert_eq!(cap.bits() & 0x7, 5);
}

#[test]
fn number_of_domains_follows_the_power_of_two_formula() {
    assert_eq!(Capability::from_bits(0).number_of_domains(), 16); // 2^4
    assert_eq!(Capability::from_bits(1).number_of_domains(), 64); // 2^6
    assert_eq!(Capability::from_bits(3).number_of_domains(), 1 << 10);
    assert_eq!(Capability::from_bits(7).number_of_domains(), 1 << 18);
}

#[test]
fn write_buffer_flush_and_caching_mode_are_read_independently() {
    assert!(!Capability::from_bits(1 << 7).requires_write_buffer_flush());
    assert!(Capability::from_bits(1 << 7).caching_mode());
    assert!(Capability::from_bits(1 << 4).requires_write_buffer_flush());
    assert!(!Capability::from_bits(1 << 4).caching_mode());
}

#[test]
fn dma_draining_requires_both_independent_capability_bits() {
    assert!(!Capability::from_bits(0).dma_draining());
    assert!(!Capability::from_bits(1 << 54).dma_draining());
    assert!(!Capability::from_bits(1 << 55).dma_draining());
    assert!(Capability::from_bits((1 << 54) | (1 << 55)).dma_draining());
    assert!(!Capability::from_bits(1 << 61).enhanced_set_interrupt_table_pointer());
    assert!(Capability::from_bits(1 << 62).enhanced_set_interrupt_table_pointer());
}

#[test]
fn sagaw_maps_each_level_to_its_own_bit() {
    // Bit (levels - 2), placed with an independent literal.
    for levels in 2u8..=5 {
        let cap = Capability::from_bits(1 << (8 + (levels - 2) as u64));
        assert!(cap.sagaw_supports(levels), "level {levels}");
        for other in 2u8..=5 {
            if other != levels {
                assert!(!cap.sagaw_supports(other), "level {levels} leaked {other}");
            }
        }
    }
    assert!(!Capability::from_bits(0).sagaw_supports(4));
    assert!(!Capability::from_bits(u64::MAX).sagaw_supports(1));
    assert!(!Capability::from_bits(u64::MAX).sagaw_supports(7));
}

#[test]
fn extended_capability_field_positions_match_the_spec() {
    assert_eq!(ExtendedCapability::C, 1 << 0);
    assert_eq!(ExtendedCapability::QI, 1 << 1);
    assert_eq!(ExtendedCapability::IR, 1 << 3);
    assert_eq!(ExtendedCapability::EIM, 1 << 4);
    assert_eq!(ExtendedCapability::IRO_SHIFT, 8);
    assert_eq!(ExtendedCapability::IRO_MASK, 0x3FF, "IRO bits 17:8");
}

#[test]
fn extended_capability_decodes_each_field() {
    let ecap = ExtendedCapability::from_bits(1 | (1 << 1) | (1 << 3) | (1 << 4) | (0x2Au64 << 8));
    assert!(ecap.page_walk_coherent());
    assert!(ecap.queued_invalidation());
    assert!(ecap.interrupt_remapping());
    assert!(ecap.extended_interrupt_mode());
    assert_eq!(ecap.iotlb_register_offset_bytes(), 0x2A * 16);

    let only_ir = ExtendedCapability::from_bits(1 << 3);
    assert!(only_ir.interrupt_remapping());
    assert!(!only_ir.extended_interrupt_mode());
    let only_eim = ExtendedCapability::from_bits(1 << 4);
    assert!(!only_eim.interrupt_remapping());
    assert!(only_eim.extended_interrupt_mode());

    // Only IRO set: the three single-bit fields read false.
    let only_iro = ExtendedCapability::from_bits(0x2Au64 << 8);
    assert!(!only_iro.page_walk_coherent());
    assert!(!only_iro.queued_invalidation());
    assert!(!only_iro.interrupt_remapping());
    assert!(!only_iro.extended_interrupt_mode());
    assert_eq!(only_iro.iotlb_register_offset_bytes(), 0x2A * 16);
    assert_eq!(only_iro.bits(), 0x2Au64 << 8);
}

#[test]
fn global_command_and_status_bits_match_the_spec() {
    assert_eq!(gcmd::TE, 1 << 31);
    assert_eq!(gcmd::SRTP, 1 << 30);
    assert_eq!(gcmd::WBF, 1 << 27);
    assert_eq!(gcmd::QIE, 1 << 26);
    assert_eq!(gcmd::IRE, 1 << 25);
    assert_eq!(gcmd::SIRTP, 1 << 24);
    assert_eq!(gcmd::CFI, 1 << 23);
    assert_eq!(gsts::TES, 1 << 31);
    assert_eq!(gsts::RTPS, 1 << 30);
    assert_eq!(gsts::WBFS, 1 << 27);
    assert_eq!(gsts::QIES, 1 << 26);
    assert_eq!(gsts::IRES, 1 << 25);
    assert_eq!(gsts::IRTPS, 1 << 24);
    assert_eq!(gsts::CFIS, 1 << 23);
}

#[test]
fn command_helpers_or_one_bit_into_a_preserved_word() {
    let preserved = gcmd::IRE | gcmd::TE;
    assert_eq!(set_root_table_pointer(preserved), preserved | gcmd::SRTP);
    assert_eq!(enable_translation(0), gcmd::TE);
    assert_eq!(set_interrupt_table_pointer(0), 1 << 24);
    assert_eq!(enable_interrupt_remapping(gcmd::CFI), 1 << 25);
    // The preserved bits survive.
    assert_eq!(enable_translation(gcmd::IRE) & gcmd::IRE, gcmd::IRE);
}

#[test]
fn status_predicates_read_their_own_bit() {
    assert!(root_table_pointer_set(gsts::RTPS));
    assert!(!root_table_pointer_set(gsts::TES));
    assert!(translation_enabled(gsts::TES));
    assert!(!translation_enabled(gsts::RTPS));
    assert!(interrupt_table_pointer_set(gsts::IRTPS));
    assert!(!interrupt_table_pointer_set(gsts::IRES));
    assert!(interrupt_remapping_enabled(gsts::IRES));
    assert!(!interrupt_remapping_enabled(gsts::IRTPS));
    assert!(compatibility_interrupts_enabled(gsts::CFIS));
    assert!(!compatibility_interrupts_enabled(gsts::IRES));
}

#[test]
fn takeover_requires_every_firmware_facility_to_be_inactive() {
    assert!(takeover_ready(0));
    assert!(takeover_ready(gsts::RTPS | gsts::IRTPS));
    assert!(!takeover_ready(gsts::TES));
    assert!(!takeover_ready(gsts::QIES));
    assert!(!takeover_ready(gsts::IRES));
    assert!(!takeover_ready(gsts::CFIS));
    assert!(!takeover_ready(u32::MAX));
}

#[test]
fn interrupt_table_and_queue_addresses_are_exact_and_aligned() {
    assert_eq!(IRTA, 0xB8);
    assert_eq!(IQA, 0x90);
    assert_eq!(IQH, 0x80);
    assert_eq!(IQT, 0x88);
    assert_eq!(interrupt_table_address(0x1234_5000), Some(0x1234_5800));
    assert_eq!(interrupt_table_address(0x1234_5001), None);
    assert_eq!(interrupt_table_base(0x1234_580f), 0x1234_5000);
    assert!(extended_interrupt_mode_enabled(0x800));
    assert!(interrupt_table_has_two_entries(0x1234_5800));
    assert!(!interrupt_table_has_two_entries(0x1234_5801));
    assert_eq!(invalidation_queue_address(0x2234_5000), Some(0x2234_5000));
    assert_eq!(invalidation_queue_address(0x2234_5008), None);
}

#[test]
fn queued_interrupt_cache_invalidation_descriptors_match_the_spec() {
    assert_eq!(
        InvalidationDescriptor::global_interrupt_entry_cache(),
        InvalidationDescriptor { lo: 4, hi: 0 }
    );
    assert_ne!(
        InvalidationDescriptor::global_interrupt_entry_cache().lo,
        4 | (1 << 4),
        "IEC granularity bit 4 selects one index rather than the global cache"
    );
    assert_eq!(
        InvalidationDescriptor::wait_status(0x4000, 0xa5a5_5a5a),
        Some(InvalidationDescriptor {
            lo: 5 | (1 << 5) | (0xa5a5_5a5au64 << 32),
            hi: 0x4000,
        })
    );
    assert_eq!(InvalidationDescriptor::wait_status(0x4002, 1), None);
}

#[test]
fn address_width_knows_its_depth_and_encoding() {
    assert_eq!(AddressWidth::Level3.levels(), 3);
    assert_eq!(AddressWidth::Level4.levels(), 4);
    assert_eq!(AddressWidth::Level5.levels(), 5);
    assert_eq!(AddressWidth::Level3.address_bits(), 39);
    assert_eq!(AddressWidth::Level4.address_bits(), 48);
    assert_eq!(AddressWidth::Level5.address_bits(), 57);
    assert_eq!(AddressWidth::Level3.context_encoding(), 1);
    assert_eq!(AddressWidth::Level4.context_encoding(), 2);
    assert_eq!(AddressWidth::Level5.context_encoding(), 3);
}

/// Coverage ends exactly at each width's power of two, on both sides.
#[test]
fn a_width_covers_every_address_below_its_power_of_two_and_none_above() {
    for (width, bits) in [
        (AddressWidth::Level3, 39u32),
        (AddressWidth::Level4, 48),
        (AddressWidth::Level5, 57),
    ] {
        assert!(width.covers(0));
        assert!(width.covers((1u64 << bits) - 1), "{bits}-bit top page");
        assert!(!width.covers(1u64 << bits), "{bits}-bit overflow");
        assert!(!width.covers(u64::MAX));
    }
    // A 48-bit walk covers an address a 39-bit walk refuses.
    assert!(AddressWidth::Level4.covers(1 << 39));
    assert!(!AddressWidth::Level3.covers(1 << 39));
}

#[test]
fn choose_prefers_the_widest_supported_table() {
    // A large MGAW so only SAGAW decides.
    let big_mgaw = 62u64 << 16; // MGAW field 62 -> 63-bit
    let all = Capability::from_bits((1 << 9) | (1 << 10) | (1 << 11) | big_mgaw);
    assert_eq!(AddressWidth::choose(all), Some(AddressWidth::Level5));

    let three_four = Capability::from_bits((1 << 9) | (1 << 10) | big_mgaw);
    assert_eq!(AddressWidth::choose(three_four), Some(AddressWidth::Level4));

    let three = Capability::from_bits((1 << 9) | big_mgaw);
    assert_eq!(AddressWidth::choose(three), Some(AddressWidth::Level3));

    // The 2-level width is supported but never chosen.
    let two = Capability::from_bits((1 << 8) | big_mgaw);
    assert_eq!(AddressWidth::choose(two), None);
    assert_eq!(AddressWidth::choose(Capability::from_bits(big_mgaw)), None);
}

#[test]
fn supported_table_depth_is_independent_of_guest_address_limit() {
    for sagaw in 0u64..32 {
        let expected = [
            (3, AddressWidth::Level5),
            (2, AddressWidth::Level4),
            (1, AddressWidth::Level3),
        ]
        .into_iter()
        .find_map(|(bit, width)| (sagaw & (1 << bit) != 0).then_some(width));
        for mgaw_field in 0u64..64 {
            let cap = Capability::from_bits((sagaw << 8) | (mgaw_field << 16));
            assert_eq!(
                AddressWidth::choose(cap),
                expected,
                "{sagaw:#x}/{mgaw_field}"
            );
        }
    }
    let four_only = Capability::from_bits((1 << 10) | (38 << 16));
    assert_eq!(AddressWidth::choose(four_only), Some(AddressWidth::Level4));
}

#[test]
fn effective_coverage_obeys_both_table_and_guest_limits() {
    for (width, table_bits) in [
        (AddressWidth::Level3, 39u32),
        (AddressWidth::Level4, 48),
        (AddressWidth::Level5, 57),
    ] {
        for guest_bits in 1u32..=64 {
            let cap = Capability::from_bits(u64::from(guest_bits - 1) << 16);
            let boundary = 1u64 << table_bits.min(guest_bits);
            assert!(width.covers_on(cap, 0));
            assert!(width.covers_on(cap, boundary - 1));
            assert!(!width.covers_on(cap, boundary));
            assert!(!width.covers_on(cap, u64::MAX));
        }
    }
}

#[test]
fn index_splits_an_iova_nine_bits_at_a_time() {
    // Distinct multi-bit indices placed with independent literal shifts.
    let iova =
        0xABC | (0x1F << 12) | (0x2E << 21) | (0x3D << 30) | (0x4Cu64 << 39) | (0x5Bu64 << 48);
    let w = AddressWidth::Level5;
    assert_eq!(w.index(iova, 0), 0x1F);
    assert_eq!(w.index(iova, 1), 0x2E);
    assert_eq!(w.index(iova, 2), 0x3D);
    assert_eq!(w.index(iova, 3), 0x4C);
    assert_eq!(w.index(iova, 4), 0x5B);
    // Every index is a 9-bit value.
    assert_eq!(AddressWidth::Level4.index(u64::MAX, 0), 0x1FF);
}

#[test]
fn a_root_entry_carries_present_and_a_masked_pointer() {
    let entry = RootEntry::present(0x1234_5000);
    assert_eq!(entry.lo, 0x1234_5000 | 1);
    assert_eq!(entry.hi, 0);
    assert!(entry.is_present());
    assert_eq!(entry.context_table_address(), 0x1234_5000);

    // A misaligned pointer loses its low bits but sets only bit 0 below 12.
    let masked = RootEntry::present(0x1234_5FFF);
    assert_eq!(masked.context_table_address(), 0x1234_5000);
    assert_eq!(
        masked.lo & 0xFFE,
        0,
        "no reserved low bit but present is set"
    );
    assert!(masked.is_present());
    assert!(!RootEntry::default().is_present());
}

#[test]
fn root_entry_alignment_is_checked_only_by_try_present() {
    assert!(RootEntry::try_present(0x2000).is_some());
    assert_eq!(RootEntry::try_present(0x2001), None);
    assert_eq!(
        RootEntry::try_present(0x2000),
        Some(RootEntry::present(0x2000))
    );
}

#[test]
fn a_context_entry_encodes_pointer_width_and_domain() {
    let entry = ContextEntry::present(0x2000, AddressWidth::Level4, 0xABC);
    assert_eq!(entry.lo, 0x2000 | 1);
    assert_eq!(entry.hi, 2 | (0xABC << 8));
    assert!(entry.is_present());
    assert_eq!(entry.translation_type(), 0, "legacy second-level only");
    assert_eq!(entry.second_level_address(), 0x2000);
    assert_eq!(entry.address_width_encoding(), 2);
    assert_eq!(entry.domain_id(), 0xABC);
}

#[test]
fn context_entry_masks_its_pointer_and_covers_each_width() {
    let masked = ContextEntry::present(0x2FFF, AddressWidth::Level3, 5);
    assert_eq!(masked.second_level_address(), 0x2000);
    assert_eq!(masked.address_width_encoding(), 1);

    for (width, encoding) in [
        (AddressWidth::Level3, 1),
        (AddressWidth::Level4, 2),
        (AddressWidth::Level5, 3),
    ] {
        let entry = ContextEntry::present(0x3000, width, 0xFFFF);
        assert_eq!(entry.address_width_encoding(), encoding);
        assert_eq!(entry.domain_id(), 0xFFFF);
    }

    assert!(!ContextEntry::default().is_present());
    assert_eq!(
        ContextEntry::try_present(0x3001, AddressWidth::Level4, 0),
        None
    );
    assert_eq!(
        ContextEntry::try_present(0x3000, AddressWidth::Level4, 9),
        Some(ContextEntry::present(0x3000, AddressWidth::Level4, 9))
    );
}

#[test]
fn second_level_entries_encode_permission_and_address() {
    assert_eq!(second_level::pte(0x3000, true, true), 0x3000 | 0b11);
    assert_eq!(second_level::pte(0x3000, true, false), 0x3000 | 0b01);
    assert_eq!(second_level::pte(0x3000, false, true), 0x3000 | 0b10);
    assert_eq!(second_level::pte(0x3000, false, false), 0x3000);
    // Read is bit 0, write is bit 1.
    assert_eq!(second_level::pte(0, true, false), 1);
    assert_eq!(second_level::pte(0, false, true), 2);
    // The address is masked to 4 KiB.
    assert_eq!(second_level::pte(0x3FFF, true, true), 0x3000 | 0b11);
    // A table pointer is read | write | address.
    assert_eq!(second_level::table_pointer(0x4000), 0x4000 | 0b11);
    assert_eq!(second_level::table_pointer(0x4FFF), 0x4000 | 0b11);
    assert_eq!(second_level::NOT_PRESENT, 0);
}

/// Reading an entry back inverts the encoding: presence is either
/// permission bit, and the address is the page-aligned remainder.
#[test]
fn second_level_entries_read_back_their_presence_and_address() {
    assert!(!second_level::is_present(second_level::NOT_PRESENT));
    assert!(
        !second_level::is_present(0x3000),
        "an address with no permission is absent"
    );
    assert!(second_level::is_present(second_level::pte(
        0x3000, true, false
    )));
    assert!(second_level::is_present(second_level::pte(
        0x3000, false, true
    )));
    assert!(second_level::is_present(second_level::table_pointer(
        0x4000
    )));
    assert_eq!(
        second_level::address(second_level::pte(0x3000, true, true)),
        0x3000
    );
    assert_eq!(
        second_level::address(second_level::table_pointer(0x4000)),
        0x4000
    );
    assert_eq!(second_level::address(0xFFFF_FFFF_FFFF_FFFF), ADDRESS_MASK);
}

#[test]
fn context_command_words_and_readback() {
    assert_eq!(context_command::global(), (1 << 63) | (1 << 61));
    // ICC clear means complete; a fresh command has ICC set.
    assert!(!context_command::completed(context_command::global()));
    assert!(context_command::completed(0));
    assert!(
        context_command::completed(1 << 61),
        "a stale granularity is not a pending command"
    );
    assert!(!context_command::global_completed(0));
    assert!(context_command::global_completed(1 << 59));
    assert!(!context_command::global_completed(2 << 59));
    assert!(!context_command::global_completed((1 << 63) | (1 << 59)));
}

#[test]
fn iotlb_command_words_and_readback() {
    assert_eq!(iotlb_command::global(), (1 << 63) | (1 << 60));
    assert_eq!(
        iotlb_command::global_with_drain(),
        (1 << 63) | (1 << 60) | (1 << 49) | (1 << 48)
    );
    assert!(!iotlb_command::completed(iotlb_command::global()));
    assert!(!iotlb_command::completed(iotlb_command::global_with_drain()));
    assert!(iotlb_command::completed(0));
    assert!(
        iotlb_command::completed(1 << 60),
        "a stale granularity is not a pending command"
    );
    assert!(!iotlb_command::global_completed(0));
    assert!(iotlb_command::global_completed(1 << 57));
    assert!(!iotlb_command::global_completed(2 << 57));
    assert!(!iotlb_command::global_completed((1 << 63) | (1 << 57)));
    assert_eq!(iotlb_command::COMMAND_REGISTER, 0x08);
}

#[test]
fn fault_status_reads_each_field() {
    assert_eq!(FaultStatus::PFO, 1 << 0);
    assert_eq!(FaultStatus::PPF, 1 << 1);
    assert_eq!(FaultStatus::FRI_SHIFT, 8);
    assert_eq!(FaultStatus::FRI_MASK, 0xFF);

    let status = FaultStatus::from_bits(1 | (1 << 1) | (0x2A << 8));
    assert!(status.overflow());
    assert!(status.primary_pending());
    assert_eq!(status.fault_record_index(), 0x2A);

    // Overflow and pending are distinct bits.
    assert!(FaultStatus::from_bits(1).overflow());
    assert!(!FaultStatus::from_bits(1).primary_pending());
    assert!(!FaultStatus::from_bits(2).overflow());
    assert!(FaultStatus::from_bits(2).primary_pending());
    assert_eq!(FaultStatus::from_bits(0xFF << 8).fault_record_index(), 0xFF);
    // Acknowledging an overflow writes the one write-1-to-clear bit.
    assert_eq!(FaultStatus::acknowledge_overflow(), 1);
}

#[test]
fn fault_recording_positions_match_the_spec() {
    // Positions restated relative to the 128-bit record: F bit 127, T bit
    // 126, FR bits 103:96, SID bits 79:64, FI bits 63:12.
    assert_eq!(FaultRecording::FAULT, 1 << (127 - 64));
    assert_eq!(FaultRecording::TYPE, 1 << (126 - 64));
    assert_eq!(FaultRecording::REASON_MASK, 0xFF, "FR bits 103:96");
    assert_eq!(FaultRecording::SID_MASK, 0xFFFF, "SID bits 79:64");
}

#[test]
fn a_recorded_fault_decodes_its_source_reason_and_address() {
    let record = FaultRecording {
        lo: 0xDEAD_E000,
        hi: (1 << 63) | (1 << 62) | (0x0C << (96 - 64)) | (0xA5 << (104 - 64)) | 0xBEEF,
    };
    assert!(record.is_fault());
    assert_eq!(record.source_id(), 0xBEEF);
    assert_eq!(record.fault_reason(), 0x0C);
    assert_eq!(record.fault_address(), 0xDEAD_E000);
    assert!(record.is_read_request());

    // The address is masked to a page, and a clear T bit is a write.
    let unaligned = FaultRecording {
        lo: 0xDEAD_EFFF,
        hi: 1 << 63,
    };
    assert_eq!(unaligned.fault_address(), 0xDEAD_E000);
    assert!(!unaligned.is_read_request());
}

#[test]
fn a_register_without_the_fault_bit_records_no_fault() {
    let clean = FaultRecording {
        lo: 0xDEAD_E000,
        hi: (0x0C << (96 - 64)) | 0xBEEF, // reason and SID set, but F clear
    };
    assert!(!clean.is_fault());

    // The clear value writes 1 to F and nothing else.
    let clear = FaultRecording::clear();
    assert_eq!(clear.hi, 1 << 63);
    assert_eq!(clear.lo, 0);
}
