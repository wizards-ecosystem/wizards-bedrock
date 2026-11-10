use super::*;

#[test]
fn only_disabled_and_not_ready_registers_pass_dma_classification() {
    const OTHER_CC: u32 = 0xa5a5_5a00;
    const OTHER_CSTS: u32 = 0x5a5a_a5a0;

    for enabled in [false, true] {
        for ready in [false, true] {
            let cc = OTHER_CC | (u32::from(enabled) * configuration::ENABLE);
            let csts = OTHER_CSTS | (u32::from(ready) * status::READY);
            let classified = DisabledRegisters::from_bits(cc, csts);
            if enabled || ready {
                assert_eq!(classified, Err(ControllerStateError::Active { cc, csts }));
            } else {
                assert_eq!(classified.map(DisabledRegisters::bits), Ok((cc, csts)));
            }
        }
    }
    assert_eq!(
        ControllerStateError::Active { cc: 1, csts: 0 }.to_string(),
        "controller is active (cc=0x00000001, csts=0x00000000)"
    );
}

#[derive(PartialEq, Eq, Debug)]
struct PreparationOwner {
    identity: u64,
    accesses: usize,
}

fn refuse_unverified_access(
    identity: u64,
    refused: usize,
    accesses: usize,
) -> Result<
    ControllerPreparation<PreparationOwner>,
    PreparationFailure<ControllerPreparation<PreparationOwner>, usize>,
> {
    let mut preparation = ControllerPreparation::new(PreparationOwner {
        identity,
        accesses: 0,
    });
    for access in 0..accesses {
        let (returned, ()) = preparation.access(|owner| {
            owner.accesses += 1;
            if access == refused {
                Err(access)
            } else {
                Ok(())
            }
        })?;
        preparation = returned;
    }
    Ok(preparation)
}

#[test]
fn every_unverified_preparation_access_failure_preserves_its_quarantine_owner() {
    const ACCESSES: usize = 6;

    for refused in 0..ACCESSES {
        let identity = 0xcafe_0000 + refused as u64;
        let failure = refuse_unverified_access(identity, refused, ACCESSES)
            .expect_err("the selected preparation access must refuse");
        let (reason, preparation) = failure.into_parts();
        let (preparation, recovered) = preparation
            .access(|owner| Ok::<_, core::convert::Infallible>(owner.identity))
            .expect("the returned owner remains usable");
        let owner = preparation
            .prove_disabled(0, 0)
            .expect("the recovered owner accepts a valid disabled proof")
            .into_owner();

        assert_eq!(reason, refused);
        assert_eq!(recovered, identity);
        assert_eq!(owner.identity, identity);
        assert_eq!(owner.accesses, refused + 1);
    }

    let identity = 0xcafe_ffff;
    let failure = ControllerPreparation::new(PreparationOwner {
        identity,
        accesses: 0,
    })
    .prove_disabled(configuration::ENABLE, 0)
    .expect_err("an active controller cannot release its preparation owner");
    let (reason, preparation) = failure.into_parts();
    let (_, recovered) = preparation
        .access(|owner| Ok::<_, core::convert::Infallible>(owner.identity))
        .expect("a refused disabled proof returns the owner for quarantine");
    assert_eq!(
        reason,
        ControllerStateError::Active {
            cc: configuration::ENABLE,
            csts: 0,
        }
    );
    assert_eq!(recovered, identity);
}

#[test]
fn every_disabled_preparation_access_failure_returns_the_exact_retirement_owner() {
    const ACCESSES: usize = 5;

    for refused in 0..ACCESSES {
        let identity = 0xd15a_0000 + refused as u64;
        let mut preparation = ControllerPreparation::new(PreparationOwner {
            identity,
            accesses: 0,
        })
        .prove_disabled(0, 0)
        .expect("zeroed controller state is disabled");
        let mut failure = None;
        for access in 0..ACCESSES {
            match preparation.access(|owner| {
                owner.accesses += 1;
                if access == refused {
                    Err(access)
                } else {
                    Ok(())
                }
            }) {
                Ok((returned, ())) => preparation = returned,
                Err(refused) => {
                    failure = Some(refused);
                    break;
                }
            }
        }
        let failure = failure.expect("the selected disabled access must refuse");
        let (reason, preparation) = failure.into_parts();
        let owner = preparation.into_owner();

        assert_eq!(reason, refused);
        assert_eq!(owner.identity, identity);
        assert_eq!(owner.accesses, refused + 1);
    }
}

fn identify(blocks: u64) -> [u8; NAMESPACE_IDENTIFY_BYTES] {
    let mut bytes = [0u8; NAMESPACE_IDENTIFY_BYTES];
    bytes[..8].copy_from_slice(&blocks.to_le_bytes());
    bytes[FORMAT_COUNT_OFFSET] = MAX_LBA_FORMATS - 1;
    for format in 0..MAX_LBA_FORMATS {
        let offset = LBA_FORMATS_OFFSET + usize::from(format) * LBA_FORMAT_BYTES;
        bytes[offset + 2] = SUPPORTED_BLOCK_SHIFT;
    }
    bytes
}

#[test]
fn fixed_wire_sizes_and_bit_positions_match_the_nvme_format() {
    assert_eq!(SUBMISSION_ENTRY_BYTES, 64);
    assert_eq!(COMPLETION_ENTRY_BYTES, 16);
    assert_eq!(IOSQES, 6);
    assert_eq!(IOCQES, 4);
    assert_eq!(MAX_TRANSFER_BLOCKS, 65_536);
    assert_eq!(NAMESPACE_IDENTIFY_BYTES, 192);
    assert_eq!(status::READY, 1);
    assert_eq!(status::FATAL, 2);
}

#[test]
fn every_dma_state_reports_one_consistent_ownership_answer() {
    let cases = [
        (DmaOwnership::Idle, false, false),
        (DmaOwnership::Reserved, false, false),
        (DmaOwnership::InFlight, false, true),
        (DmaOwnership::Closed, true, false),
    ];

    for (state, closed, in_flight) in cases {
        let state = DmaState { state };
        assert_eq!(matches!(state.state, DmaOwnership::Closed), closed);
        assert_eq!(state.is_in_flight(), in_flight);
    }
}

#[test]
fn publication_closes_the_unpublished_cancellation_door() {
    let mut state = DmaState::idle();
    assert!(!state.cancel_unpublished());
    assert_eq!(state.publish_with(|_| ()), None);
    assert!(state.reserve());
    assert!(state.cancel_unpublished());
    assert!(state.reserve(), "cancellation restored the idle state");
    assert!(!state.complete());
    assert_eq!(
        state.publish_with(|publication| publication._state.is_in_flight()),
        Some(true)
    );
    assert!(!state.cancel_unpublished());
    assert_eq!(state.publish_with(|_| ()), None);
    assert!(state.complete());
    assert!(state.reserve(), "completion restored the idle state");
}

#[test]
fn closed_dma_state_cannot_publish_or_complete() {
    for mut state in [
        DmaState::idle(),
        DmaState {
            state: DmaOwnership::Reserved,
        },
        DmaState {
            state: DmaOwnership::InFlight,
        },
    ] {
        state.close();
        assert!(matches!(state.state, DmaOwnership::Closed));
        assert!(!state.reserve());
        assert!(!state.cancel_unpublished());
        assert_eq!(state.publish_with(|_| ()), None);
        assert!(!state.complete());
    }
}

/// A queue holds one fewer command than it has entries.
///
/// `012` §3 singles this out: a full queue and an empty one have the same
/// head and tail, so one slot is always unused — and **the symptom of
/// getting it wrong is a hang**, not an error. There is no controller
/// response and nothing to log.
#[test]
fn a_queue_holds_one_fewer_command_than_it_has_entries() {
    for entries in [2u32, 3, 8, 64, 1024, 65536] {
        let capacity = Capacity::new(entries).expect("a real queue size");
        assert_eq!(capacity.entries(), entries);
        assert_eq!(capacity.as_zero_based(), entries - 1);
    }
}

/// The last slot is never taken, whatever the head is.
///
/// Exhaustive over every head and tail of a small queue, because the rule
/// is about one specific pair out of every pair and a sample misses it.
#[test]
fn a_queue_refuses_the_entry_that_would_make_full_look_empty() {
    let capacity = Capacity::new(4).expect("four entries");

    for head in 0..4 {
        for tail in 0..4 {
            let room = capacity.has_room(head, tail).expect("valid indices");
            let would_wrap_onto_the_head = Capacity::next(capacity, tail) == Some(head);
            assert_eq!(room, !would_wrap_onto_the_head, "head {head}, tail {tail}");
        }
    }

    // And the count that follows from it: three, never four.
    let mut tail = 0;
    let mut admitted = 0;
    for _ in 0..capacity.entries() {
        if capacity.has_room(0, tail) != Some(true) {
            break;
        }
        tail = Capacity::next(capacity, tail).expect("valid tail");
        admitted += 1;
    }
    assert_eq!(admitted, capacity.entries() - 1);
    assert_eq!(admitted, 3);
}

#[test]
fn queue_positions_refuse_every_value_outside_the_capacity() {
    let capacity = Capacity::new(4).expect("four entries");
    for index in 0..4 {
        assert_eq!(Capacity::next(capacity, index), Some((index + 1) % 4));
        assert!(capacity.has_room(index, index).is_some());
    }
    for invalid in [4, 5, u32::MAX] {
        assert_eq!(Capacity::next(capacity, invalid), None);
        assert_eq!(capacity.has_room(invalid, 0), None);
        assert_eq!(capacity.has_room(0, invalid), None);
    }
}

#[test]
fn queue_indices_and_storage_bytes_are_exact() {
    let capacity = Capacity::new(4).expect("four entries");
    assert_eq!(capacity.submission_bytes(), 256);
    assert_eq!(capacity.completion_bytes(), 64);
    assert_eq!(
        (0..4)
            .map(|index| Capacity::next(capacity, index))
            .collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3), Some(0)]
    );
}

/// Every capability field is read from its own bits.
///
/// Pairwise distinct values so no two fields can be swapped with this
/// still passing (D-136), and every one of them changes what a correct
/// driver does.
#[test]
fn every_capability_field_is_read_from_its_own_bits() {
    let bits =
        0x0000_0000_0000_00FFu64 | (0x0A << 24) | (0x3 << 32) | (1 << 37) | (1 << 48) | (5 << 52);
    let cap = Capabilities::from_bits(bits);

    assert_eq!(cap.max_queue_entries(), 256);
    assert_eq!(cap.timeout_ms(), 5_000);
    assert_eq!(cap.doorbell_stride(), 3);
    assert_eq!(cap.doorbell_bytes(), 32);
    assert!(cap.supports_nvm_command_set());
    assert_eq!(cap.min_page_shift(), 13);
    assert_eq!(cap.max_page_shift(), 17);

    // Refused outside the reported range, or `CC.MPS` would hold a value
    // the controller rejects and the ready bit would never come.
    assert!(!cap.supports_page_size(4096), "below MPSMIN");
    assert!(cap.supports_page_size(8192));
    assert!(cap.supports_page_size(1 << 17));
    assert!(!cap.supports_page_size(1 << 18), "above MPSMAX");
    assert!(!cap.supports_page_size(6144), "not a power of two");
}

/// A controller that says nothing supports nothing.
///
/// The zeroed register, which is what a driver reads from an address it
/// mapped wrongly — and it must not look like a working controller.
#[test]
fn a_silent_controller_supports_nothing() {
    let cap = Capabilities::from_bits(0);
    assert!(!cap.supports_nvm_command_set());
    assert_eq!(cap.timeout_ms(), 0);
    assert_eq!(cap.max_queue_entries(), 1);
    assert!(Capacity::new(cap.max_queue_entries()).is_none());
}

/// Doorbells alternate, and the stride is the controller's.
///
/// A driver that assumes four bytes works on every controller reporting a
/// stride of zero and silently writes a *different queue's* doorbell on
/// any that does not.
#[test]
fn doorbells_alternate_and_use_the_controllers_stride() {
    let narrow = Capabilities::from_bits(0);
    assert_eq!(doorbell(narrow, 0, Doorbell::Submission), Some(0x1000));
    assert_eq!(doorbell(narrow, 0, Doorbell::Completion), Some(0x1004));
    assert_eq!(doorbell(narrow, 1, Doorbell::Submission), Some(0x1008));
    assert_eq!(doorbell(narrow, 1, Doorbell::Completion), Some(0x100C));

    // Stride 2: sixteen bytes apart, so the I/O submission doorbell is at
    // 0x1020 and a driver assuming four would write the admin pair.
    let wide = Capabilities::from_bits(2 << 32);
    assert_eq!(wide.doorbell_bytes(), 16);
    assert_eq!(doorbell(wide, 0, Doorbell::Submission), Some(0x1000));
    assert_eq!(doorbell(wide, 1, Doorbell::Submission), Some(0x1020));
    assert_eq!(doorbell(wide, 1, Doorbell::Completion), Some(0x1030));
}

/// A command puts every field where the specification says.
#[test]
fn a_command_puts_every_field_where_the_specification_says() {
    const LBA: u64 = 0x8877_6655_0000_0064;
    let transfer = ReadTransfer::new(LBA, 8, LBA + 8, 512, 4096).expect("valid transfer");
    let command =
        Command::read(0xABCD, 1, 0x1122_3344_5566_7000, 4096, transfer).expect("valid read");

    assert_eq!(command.id(), 0xABCD);

    let bytes = command.to_bytes();
    assert_eq!(bytes.len(), 64);
    // CDW0: opcode in the low byte, identifier in the high half.
    assert_eq!(&bytes[0..4], &[0x02, 0x00, 0xCD, 0xAB]);
    // NSID.
    assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
    // PRP1 at 24, PRP2 at 32 — the two most consequential offsets here.
    assert_eq!(&bytes[24..32], &0x1122_3344_5566_7000u64.to_le_bytes());
    assert_eq!(&bytes[32..40], &0u64.to_le_bytes());
    assert_eq!(&bytes[40..48], &LBA.to_le_bytes());
    assert_eq!(&bytes[48..52], &7u32.to_le_bytes());
}

#[test]
fn queue_negotiation_requests_and_proves_both_queue_domains() {
    let command = Command::number_of_queues(0x1234, 1, 1).expect("one queue pair");
    let bytes = command.to_bytes();
    assert_eq!(&bytes[0..4], &[admin::SET_FEATURES, 0, 0x34, 0x12]);
    assert_eq!(&bytes[40..44], &feature::NUMBER_OF_QUEUES.to_le_bytes());
    assert_eq!(&bytes[44..48], &0u32.to_le_bytes());
    assert!(Command::number_of_queues(1, 0, 1).is_none());
    assert!(Command::number_of_queues(1, 1, 0).is_none());
}

/// A completion is decoded field by field, and the phase tag is one bit.
///
/// The phase is how a new entry is recognised at all — there is no tail to
/// read — so a decoder that took it from the wrong bit would see either
/// every entry or none.
#[test]
fn a_completion_is_decoded_field_by_field() {
    let mut bytes = [0u8; COMPLETION_ENTRY_BYTES];
    bytes[0..4].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    // DW2: submission head 0x0011, submission queue 0x0022.
    bytes[8..12].copy_from_slice(&0x0022_0011u32.to_le_bytes());
    // DW3: command id 0x3344, phase set, status 0x0005.
    bytes[12..16].copy_from_slice(&((0x0005u32 << 17) | (1 << 16) | 0x3344).to_le_bytes());

    let completion = Completion::from_bytes(bytes);
    assert_eq!(completion.result, 0x1234_5678);
    assert_eq!(completion.submission_head, 0x0011);
    assert_eq!(completion.submission_queue, 0x0022);
    assert_eq!(completion.id, 0x3344);
    assert!(completion.phase);
    assert_eq!(completion.status, 5);
    assert!(!completion.is_success());

    // The phase bit must not leak into the status, which is the mistake
    // that makes every completion look like a failure.
    bytes[12..16].copy_from_slice(&(1u32 << 16).to_le_bytes());
    let clean = Completion::from_bytes(bytes);
    assert!(clean.phase);
    assert_eq!(clean.status, 0);
    assert!(clean.is_success());

    let empty = Completion::from_bytes([0; COMPLETION_ENTRY_BYTES]);
    assert!(!empty.phase);
    assert_eq!(empty.status, 0);
}

#[test]
fn completion_provenance_binds_queue_command_and_submission_head() {
    let capacity = Capacity::new(8).expect("queue capacity");
    let completion = Completion {
        result: 0,
        submission_head: 3,
        submission_queue: 1,
        id: 0x1234,
        phase: true,
        status: 0,
    };

    assert!(completion.matches_submission(1, 0x1234, 3, capacity));
    assert!(!completion.matches_submission(0, 0x1234, 3, capacity));
    assert!(!completion.matches_submission(1, 0x1235, 3, capacity));
    assert!(!completion.matches_submission(1, 0x1234, 2, capacity));

    let outside = Completion {
        submission_head: 8,
        ..completion
    };
    assert!(!outside.matches_submission(1, 0x1234, 8, capacity));
}

#[test]
fn namespace_errors_name_the_refused_geometry() {
    let cases = [
        (NamespaceError::Empty, "the namespace has no logical blocks"),
        (
            NamespaceError::TooManyFormats(17),
            "the namespace reports 17 lba formats",
        ),
        (
            NamespaceError::FormatOutsideList {
                selected: 3,
                formats: 2,
            },
            "lba format 3 is outside the 2 reported formats",
        ),
        (
            NamespaceError::ReservedFormatBits(0x80),
            "formatted lba size has reserved bits 0x80",
        ),
        (
            NamespaceError::ReservedLbaFormatBits(0xfc),
            "the selected lba format has reserved bits 0xfc",
        ),
        (
            NamespaceError::Metadata(8),
            "the selected lba format has 8 metadata bytes",
        ),
        (
            NamespaceError::Protection(3),
            "the namespace enables protection settings 0x03",
        ),
        (
            NamespaceError::BlockSize(12),
            "the selected lba data size exponent is 12",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn transfer_errors_name_the_refused_extent() {
    let cases = [
        (
            TransferError::Empty,
            "the transfer contains no logical blocks",
        ),
        (
            TransferError::TooManyBlocks,
            "the transfer does not fit the nlb field",
        ),
        (TransferError::BlockSize, "the logical-block size is zero"),
        (
            TransferError::OutsideNamespace,
            "the transfer crosses the namespace boundary",
        ),
        (
            TransferError::LengthOverflow,
            "the transfer byte length overflows",
        ),
        (
            TransferError::BufferTooSmall,
            "the dma buffer is shorter than the transfer",
        ),
        (
            TransferError::ControllerLimit,
            "the transfer exceeds the proved controller limit",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

/// A zero-sized namespace has no valid LBA, while both extent ends survive.
#[test]
fn a_namespace_has_at_least_one_logical_block() {
    assert_eq!(Namespace::parse(&identify(0)), Err(NamespaceError::Empty));

    let one = Namespace::parse(&identify(1)).expect("one addressable block");
    assert_eq!(one.blocks(), 1);
    assert_eq!(one.block_bytes(), SUPPORTED_BLOCK_BYTES);

    let largest = Namespace::parse(&identify(u64::MAX)).expect("the largest lba extent");
    assert_eq!(largest.blocks(), u64::MAX);
}

/// Exhaust the reported count and every byte value of the format selector.
#[test]
fn every_format_selector_stays_inside_the_negotiated_table() {
    for last in 0..=u8::MAX {
        let mut bytes = identify(1);
        bytes[FORMAT_COUNT_OFFSET] = last;
        let parsed = Namespace::parse(&bytes);
        if last < MAX_LBA_FORMATS {
            assert!(parsed.is_ok(), "zero-based format count {last}");
        } else {
            assert_eq!(
                parsed,
                Err(NamespaceError::TooManyFormats(u16::from(last) + 1)),
                "zero-based format count {last}"
            );
        }
    }

    for last in 0..MAX_LBA_FORMATS {
        for formatted in 0..=u8::MAX {
            let mut bytes = identify(1);
            bytes[FORMAT_COUNT_OFFSET] = last;
            bytes[FORMATTED_OFFSET] = formatted;
            let selected = formatted & 0x0f;
            let parsed = Namespace::parse(&bytes);
            if formatted & 0x80 != 0 {
                assert_eq!(
                    parsed,
                    Err(NamespaceError::ReservedFormatBits(formatted)),
                    "last {last}, flbas {formatted:#04x}"
                );
            } else if selected > last {
                assert_eq!(
                    parsed,
                    Err(NamespaceError::FormatOutsideList {
                        selected,
                        formats: last + 1,
                    }),
                    "last {last}, flbas {formatted:#04x}"
                );
            } else {
                assert!(
                    parsed.is_ok(),
                    "the high index and metadata-placement bits are inapplicable: last {last}, flbas {formatted:#04x}"
                );
            }
        }
    }
}

/// Every possible exponent has one policy, and none is ever shifted first.
#[test]
fn every_lbads_value_has_one_explicit_policy() {
    for block_shift in 0..=u8::MAX {
        let mut bytes = identify(1);
        bytes[LBA_FORMATS_OFFSET + 2] = block_shift;
        let parsed = Namespace::parse(&bytes);
        if block_shift == SUPPORTED_BLOCK_SHIFT {
            assert_eq!(
                parsed.expect("the one supported exponent").block_bytes(),
                512
            );
        } else {
            assert_eq!(parsed, Err(NamespaceError::BlockSize(block_shift)));
        }
    }
}

/// Metadata capabilities are harmless; selected metadata and protection are not.
#[test]
fn every_metadata_and_protection_setting_is_explicit() {
    for metadata in 0..=u16::MAX {
        let mut bytes = identify(1);
        bytes[LBA_FORMATS_OFFSET..LBA_FORMATS_OFFSET + 2].copy_from_slice(&metadata.to_le_bytes());
        let parsed = Namespace::parse(&bytes);
        if metadata == 0 {
            assert!(parsed.is_ok());
        } else {
            assert_eq!(parsed, Err(NamespaceError::Metadata(metadata)));
        }
    }

    for protection in 0..=u8::MAX {
        let mut bytes = identify(1);
        bytes[PROTECTION_SETTINGS_OFFSET] = protection;
        let parsed = Namespace::parse(&bytes);
        if protection == 0 {
            assert!(parsed.is_ok());
        } else {
            assert_eq!(parsed, Err(NamespaceError::Protection(protection)));
        }
    }

    for capabilities in 0..=u8::MAX {
        let mut bytes = identify(1);
        bytes[METADATA_CAPABILITIES_OFFSET] = capabilities;
        bytes[PROTECTION_CAPABILITIES_OFFSET] = capabilities;
        assert!(
            Namespace::parse(&bytes).is_ok(),
            "capabilities {capabilities:#04x} do not enable a transfer setting"
        );
    }
}

/// Relative performance is descriptive; every higher LBAF bit is reserved.
#[test]
fn every_lba_format_control_value_has_one_explicit_policy() {
    for control in 0..=u8::MAX {
        let mut bytes = identify(1);
        bytes[LBA_FORMATS_OFFSET + 3] = control;
        let parsed = Namespace::parse(&bytes);
        if control <= 3 {
            assert!(parsed.is_ok(), "relative performance {control}");
        } else {
            assert_eq!(parsed, Err(NamespaceError::ReservedLbaFormatBits(control)));
        }
    }
}

/// Zero has no NLB encoding and must never reach `blocks - 1`.
#[test]
fn an_empty_transfer_has_no_nlb_encoding() {
    assert_eq!(
        ReadTransfer::new(0, 0, 1, 512, 512),
        Err(TransferError::Empty)
    );
}

/// Exhaust every count around the sixteen-bit, zero-based NLB field.
#[test]
fn every_nonempty_nlb_count_has_one_exact_boundary() {
    let namespace = u64::from(MAX_TRANSFER_BLOCKS) + 1;
    for blocks in 1..=MAX_TRANSFER_BLOCKS + 1 {
        let transfer = ReadTransfer::new(0, blocks, namespace, 1, namespace);
        if blocks <= MAX_TRANSFER_BLOCKS {
            let transfer = transfer.expect("the NLB field can encode this count");
            assert_eq!(u32::from(transfer.zero_based_blocks()), blocks - 1);
            assert_eq!(transfer.bytes(), u64::from(blocks));
        } else {
            assert_eq!(transfer, Err(TransferError::TooManyBlocks));
        }
    }
}

/// The namespace end is exclusive and its addition never wraps.
#[test]
fn a_transfer_never_crosses_or_wraps_its_namespace() {
    let last = ReadTransfer::new(9, 1, 10, 512, 512).expect("the final block");
    assert_eq!(last.lba(), 9);
    assert_eq!(last.bytes(), 512);
    assert_eq!(
        ReadTransfer::new(9, 2, 10, 512, 1024),
        Err(TransferError::OutsideNamespace)
    );
    assert_eq!(
        ReadTransfer::new(u64::MAX, 1, u64::MAX, 1, 1),
        Err(TransferError::OutsideNamespace)
    );
    assert_eq!(
        ReadTransfer::new(0, 1, 0, 1, 1),
        Err(TransferError::OutsideNamespace)
    );
}

/// The byte extent must exist, multiply exactly and fit the DMA owner.
#[test]
fn transfer_bytes_must_fit_the_dma_buffer() {
    assert_eq!(
        ReadTransfer::new(0, 1, 1, 0, 0),
        Err(TransferError::BlockSize)
    );
    assert_eq!(
        ReadTransfer::new(0, 2, 2, u64::MAX, u64::MAX),
        Err(TransferError::LengthOverflow)
    );
    assert_eq!(
        ReadTransfer::new(0, 2, 2, 512, 1023),
        Err(TransferError::BufferTooSmall)
    );
    let exact = ReadTransfer::new(0, 2, 2, 512, 1024).expect("an exact buffer");
    assert_eq!(exact.bytes(), 1024);
}

#[test]
fn an_unidentified_controller_is_bounded_to_one_page() {
    let exact = ReadTransfer::new(0, 8, 32, 512, 4096).expect("one page");
    assert_eq!(exact.within_limit(4096), Ok(exact));

    let over = ReadTransfer::new(0, 9, 32, 512, 4608).expect("encodable transfer");
    assert_eq!(over.within_limit(4096), Err(TransferError::ControllerLimit));
}

#[test]
fn identify_binds_one_complete_aligned_page() {
    const PAGE: u64 = 4096;
    const LAST_VALID_PAGE: u64 = u64::MAX - (2 * PAGE - 1);
    const WRAPPING_PAGE: u64 = u64::MAX - (PAGE - 1);

    let command = Command::identify_namespace(7, 1, LAST_VALID_PAGE, PAGE)
        .expect("the last non-wrapping page");
    let bytes = command.to_bytes();
    assert_eq!(&bytes[24..32], &LAST_VALID_PAGE.to_le_bytes());
    assert_eq!(&bytes[32..40], &0u64.to_le_bytes());

    assert!(Command::identify_namespace(7, 0, 0x1000, PAGE).is_none());
    assert!(Command::identify_namespace(7, 1, 0x1001, PAGE).is_none());
    assert!(Command::identify_namespace(7, 1, 0x1000, 2048).is_none());
    assert!(Command::identify_namespace(7, 1, 0x2000, 5000).is_none());
    assert!(Command::identify_namespace(7, 1, WRAPPING_PAGE, PAGE).is_none());
}

#[test]
fn read_binds_its_validated_extent_to_one_page() {
    const PAGE: u64 = 4096;
    const LAST_VALID_PAGE: u64 = u64::MAX - (2 * PAGE - 1);
    const WRAPPING_PAGE: u64 = u64::MAX - (PAGE - 1);
    let exact = ReadTransfer::new(0, 8, 32, 512, PAGE).expect("one-page transfer");
    let too_large = ReadTransfer::new(0, 9, 32, 512, PAGE + 512).expect("encodable transfer");

    assert!(Command::read(9, 1, LAST_VALID_PAGE, PAGE, exact).is_some());
    assert!(Command::read(9, 0, 0x1000, PAGE, exact).is_none());
    assert!(Command::read(9, 1, 0x1001, PAGE, exact).is_none());
    assert!(Command::read(9, 1, 0x1000, 2048, exact).is_none());
    assert!(Command::read(9, 1, 0x2000, 5000, exact).is_none());
    assert!(Command::read(9, 1, WRAPPING_PAGE, PAGE, exact).is_none());
    assert!(Command::read(9, 1, 0x1000, PAGE, too_large).is_none());
}

#[test]
fn queue_commands_encode_every_dword_field_where_the_specification_puts_it() {
    const PAGE: u64 = 4096;
    const HIGH: u64 = 0x1_2345_6000;
    let capacity = Capacity::new(256).expect("a queue capacity");

    let completion = Command::create_completion_queue(11, 3, capacity, HIGH, PAGE)
        .expect("a completion queue above 4 GiB");
    let bytes = completion.to_bytes();
    assert_eq!(
        &bytes[24..32],
        &HIGH.to_le_bytes(),
        "PRP1 carries both halves"
    );
    assert_eq!(
        &bytes[40..44],
        &((255u32 << 16) | 3).to_le_bytes(),
        "QSIZE | QID"
    );
    assert_eq!(&bytes[44..48], &1u32.to_le_bytes(), "physically contiguous");

    let submission = Command::create_submission_queue(12, 5, 3, capacity, HIGH, PAGE)
        .expect("a submission queue bound to its completion queue");
    let bytes = submission.to_bytes();
    assert_eq!(&bytes[24..32], &HIGH.to_le_bytes());
    assert_eq!(
        &bytes[40..44],
        &((255u32 << 16) | 5).to_le_bytes(),
        "QSIZE | QID"
    );
    assert_eq!(
        &bytes[44..48],
        &((3u32 << 16) | 1).to_le_bytes(),
        "CQID | PC"
    );

    let queues = Command::number_of_queues(0x1234, 3, 5).expect("three and five queues");
    assert_eq!(
        &queues.to_bytes()[44..48],
        &((4u32 << 16) | 2).to_le_bytes(),
        "NCQR | NSQR, zero based"
    );
}

#[test]
fn queue_commands_bind_the_complete_contiguous_extent() {
    const PAGE: u64 = 4096;
    const WRAPPING_PAGE: u64 = u64::MAX - (PAGE - 1);
    let capacity = Capacity::new(256).expect("a queue capacity");

    let completion = Command::create_completion_queue(11, 1, capacity, 0x1000, PAGE)
        .expect("a complete completion queue");
    let submission = Command::create_submission_queue(12, 1, 1, capacity, 0x2000, PAGE)
        .expect("a complete submission queue");
    assert_eq!(&completion.to_bytes()[24..32], &0x1000u64.to_le_bytes());
    assert_eq!(&submission.to_bytes()[24..32], &0x2000u64.to_le_bytes());

    assert!(Command::create_completion_queue(11, 0, capacity, 0x1000, PAGE).is_none());
    assert!(Command::create_completion_queue(11, 1, capacity, 0x1001, PAGE).is_none());
    assert!(Command::create_completion_queue(11, 1, capacity, WRAPPING_PAGE, PAGE).is_none());
    assert!(Command::create_submission_queue(12, 0, 1, capacity, 0x2000, PAGE).is_none());
    assert!(Command::create_submission_queue(12, 1, 0, capacity, 0x2000, PAGE).is_none());
    assert!(Command::create_submission_queue(12, 1, 1, capacity, 0x2001, PAGE).is_none());
    assert!(Command::create_submission_queue(12, 1, 1, capacity, WRAPPING_PAGE, PAGE).is_none());
}
