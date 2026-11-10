use super::*;
use crate::bar::io_window_size;
use crate::probe::finish_probe_restoration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TestAccessError;

impl core::fmt::Display for TestAccessError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Write::write_str(f, "test access failure")
    }
}

impl core::error::Error for TestAccessError {}

#[test]
fn every_probe_error_describes_and_exposes_its_cause() {
    fn assert_error<T: core::error::Error>() {}

    assert_error::<ProbeError<TestAccessError>>();
    for (error, expected) in [
        (
            ProbeError::Unavailable(EnableError::Stale),
            "bar probe authority is unavailable: the command lease is no longer current",
        ),
        (
            ProbeError::Access(TestAccessError),
            "configuration access failed: test access failure",
        ),
        (
            ProbeError::Refused(0x00a5),
            "command register refused decode-off value and read back 0x00a5",
        ),
        (
            ProbeError::Restore(TestAccessError),
            "configuration restoration failed: test access failure",
        ),
        (
            ProbeError::RestoreChanged,
            "configuration restoration changed the captured register state",
        ),
        (
            ProbeError::Malformed,
            "the function has an unsupported BAR layout",
        ),
        (
            ProbeError::InvalidWindow,
            "a BAR returned an invalid memory window",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }

    assert!(
        core::error::Error::source(&ProbeError::<TestAccessError>::Unavailable(
            EnableError::Stale
        ))
        .is_some()
    );
    assert!(core::error::Error::source(&ProbeError::Access(TestAccessError)).is_some());
    assert!(core::error::Error::source(&ProbeError::Restore(TestAccessError)).is_some());
    assert!(core::error::Error::source(&ProbeError::<TestAccessError>::Malformed).is_none());
}

#[test]
fn bar_probe_reservations_are_registry_bound() {
    let at = command_address(0);
    let authority = device_authority(at);
    let mut first = CommandLeases::<1>::new();
    let mut second = CommandLeases::<1>::new();
    let first_probe = first.begin_probe(&authority).expect("first reservation");
    let second_probe = second.begin_probe(&authority).expect("second reservation");

    let first_probe = second
        .finish_probe(&authority, first_probe)
        .expect_err("another registry cannot release the reservation");
    assert_eq!(second.begin_probe(&authority), Err(EnableError::Leased));
    first
        .finish_probe(&authority, first_probe)
        .expect("the issuing registry releases it");
    second
        .finish_probe(&authority, second_probe)
        .expect("the other registry retained its reservation");
    let first_probe = first
        .begin_probe(&authority)
        .expect("first registry made room");
    let second_probe = second
        .begin_probe(&authority)
        .expect("second registry made room");
    first
        .finish_probe(&authority, first_probe)
        .expect("first exact release");
    second
        .finish_probe(&authority, second_probe)
        .expect("second exact release");
}

#[test]
fn public_probe_retry_restores_hardware_and_releases_its_sublease() {
    let at = command_address(0);
    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(at, HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    fake.fail_at = Some(30);
    let failure = crate::probe_bars(&mut functions, &authority, |_, access| fake.access(access))
        .expect_err("one restoration read is unavailable");
    assert_eq!(failure.reason(), &ProbeError::Restore(30));
    let retry = failure
        .into_retry()
        .expect("uncertain restoration retains retry authority");

    fake.fail_at = None;
    crate::retry_probe(&mut functions, &authority, retry, |_, access| {
        fake.access(access)
    })
    .expect("the issuing function restores its exact captured state");
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.status, original.status);
    assert_eq!(fake.bars, original.bars);
    release_function(&mut functions, authority).expect("retry released the probe sublease");
}

#[test]
fn a_probe_that_decoded_nothing_is_refused() {
    assert_eq!(memory_window_size(0, 0, MemoryWidth::Bits32), None);
    assert_eq!(memory_window_size(0, 0, MemoryWidth::Bits64), None);
    assert_eq!(io_window_size(0), None);
    assert_eq!(
        memory_window_size(0x0000_000F, 0, MemoryWidth::Bits32),
        None
    );
    assert_eq!(io_window_size(0x0000_0003), None);
    assert_eq!(io_window_size(0xFFFF_0000), None);
}

#[test]
fn probed_bars_return_their_complete_slot_array() {
    let first = Window {
        bar: Bar::Memory {
            address: 0x4000,
            wide: false,
            prefetchable: false,
        },
        size: 0x1000,
    };
    let mut windows = [None; BARS];
    windows[0] = Some(first);
    let probed = ProbedBars {
        function: None,
        windows,
    };

    assert_eq!(probed.into_windows(), windows);
}

#[test]
fn a_bar_probe_refuses_only_register_counts_above_its_bound() {
    let mut accessed = false;
    assert_eq!(
        probe_bars(BARS + 1, |_| {
            accessed = true;
            Ok::<_, ()>(0)
        }),
        Err(ProbeError::Malformed)
    );
    assert!(!accessed);

    let mut fake = FakeConfig::device();
    fake.bars = [0; BARS];
    fake.masks = [0; BARS];
    assert!(probe_bars(1, |event| fake.access(event)).is_ok());
}

#[test]
fn an_address_zero_memory_bar_is_discovered_from_its_mask() {
    let mut fake = FakeConfig::device();
    fake.bars = [0; BARS];
    fake.masks = [0xFFFF_F000, 0, 0, 0, 0, 0];

    let probed = probe_bars(1, |event| fake.access(event)).expect("valid BAR");
    assert_eq!(
        probed.into_windows()[0],
        Some(Window {
            bar: Bar::Memory {
                address: 0,
                wide: false,
                prefetchable: false,
            },
            size: 0x1000,
        })
    );
}

#[test]
fn an_io_address_bit_is_not_a_wide_memory_type() {
    let mut fake = FakeConfig::device();
    fake.bars = [0x0000_C005, 0, 0, 0, 0, 0];
    fake.masks = [0xFFFF_FFFD, 0, 0, 0, 0, 0];

    let probed = probe_bars(1, |event| fake.access(event)).expect("valid BAR");
    assert_eq!(
        probed.into_windows()[0],
        Some(Window {
            bar: Bar::Io { port: 0xC004 },
            size: 4,
        })
    );
}

#[test]
fn unsupported_memory_bar_types_refuse_before_any_probe_write() {
    for memory_type in [1, 3] {
        let mut fake = FakeConfig::device();
        fake.bars = [0; BARS];
        fake.bars[0] = 0x0008_0000 | memory_type << 1;
        let original = fake.clone();

        assert_eq!(
            probe_bars(1, |event| fake.access(event)),
            Err(ProbeError::Malformed),
            "memory type {memory_type:02b}"
        );
        assert_eq!(
            fake.events(),
            [ProbeAccess::ReadCommand, ProbeAccess::ReadBar(0)].map(Some),
            "memory type {memory_type:02b}"
        );
        assert_eq!(fake.command, original.command);
        assert_eq!(fake.bars, original.bars);
    }
}

#[test]
fn the_claimed_header_alone_sets_the_public_bar_probe_extent() {
    for (function, header, registers) in [(0, 0u8, BARS), (1, 1u8, BRIDGE_BARS)] {
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(function), HeaderType::from_bits(header))
            .expect("supported header authority");
        let mut fake = FakeConfig::device();

        let staged = crate::probe_bars(&mut functions, &authority, |at, event| {
            assert_eq!(at, authority.address());
            fake.access(event)
        })
        .expect("header-sized probe");
        let (_, state) = staged.into_parts();

        assert!(fake.events().iter().any(|event| {
            matches!(event, Some(ProbeAccess::ReadBar(index)) if *index == registers - 1)
        }));
        assert!(!fake.events().iter().any(|event| {
            matches!(event, Some(ProbeAccess::ReadBar(index) | ProbeAccess::WriteBar(index, _)) if *index >= registers)
        }));
        crate::restore_command(&mut functions, &authority, state, |_, access| {
            fake_command_access(&mut fake, access)
        })
        .expect("the staged command restores");
        release_function(&mut functions, authority).expect("a retired probe releases its function");
    }
}

#[test]
fn inherited_bus_master_stays_off_until_the_staged_command_retires() {
    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeConfig::device();
    let original = fake.command;
    assert_ne!(original & command::BUS_MASTER, 0);

    let staged = crate::probe_bars(&mut functions, &authority, |_, event| fake.access(event))
        .expect("the BARs probe with inherited DMA disabled");
    let (_, state) = staged.into_parts();
    assert_eq!(fake.bar_writes_with_bus_master, 0);
    assert_eq!(fake.command, original & !PROBE_DISABLED);

    let failure = release_function(&mut functions, authority)
        .expect_err("the staged command keeps its function authority");
    assert_eq!(failure.reason(), FunctionReleaseError::Active);
    let authority = failure.into_authority();
    crate::restore_command(&mut functions, &authority, state, |_, access| {
        fake_command_access(&mut fake, access)
    })
    .expect("retirement restores the inherited command exactly");
    assert_eq!(fake.command, original);
    release_function(&mut functions, authority)
        .expect("the function releases only after command retirement");
}

#[test]
fn refused_bus_master_clear_releases_only_after_exact_rollback() {
    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeConfig::device();
    let original = fake.command;
    fake.ignore_write_at = Some(7);

    let failure = crate::probe_bars(&mut functions, &authority, |_, event| fake.access(event))
        .expect_err("the device ignored the DMA-disable write");
    assert_eq!(failure.reason(), &ProbeError::Refused(original));
    assert!(failure.into_retry().is_none());
    assert_eq!(fake.bar_writes_with_bus_master, 0);
    assert_eq!(fake.command, original);
    release_function(&mut functions, authority)
        .expect("verified rollback released the probe sublease");
}

#[test]
fn a_bar_probe_is_one_restored_decode_off_transaction() {
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    let probed = probe_bars(BARS, |event| fake.access(event)).expect("valid device");

    assert_eq!(fake.command, original.command);
    assert_eq!(
        fake.status, original.status,
        "status is never echoed into RW1C bits"
    );
    assert_eq!(fake.bars, original.bars);
    assert_eq!(
        probed.into_windows()[0],
        Some(Window {
            bar: Bar::Memory {
                address: 0x1_8000_0000,
                wide: true,
                prefetchable: false,
            },
            size: 0x4000,
        })
    );
    assert_eq!(
        probed.into_windows()[1],
        None,
        "the high half is not another window"
    );
    assert_eq!(
        probed.into_windows()[2],
        Some(Window {
            bar: Bar::Io { port: 0xC000 },
            size: 32,
        })
    );

    let expected = [
        ProbeAccess::ReadCommand,
        ProbeAccess::ReadBar(0),
        ProbeAccess::ReadBar(1),
        ProbeAccess::ReadBar(2),
        ProbeAccess::ReadBar(3),
        ProbeAccess::ReadBar(4),
        ProbeAccess::ReadBar(5),
        ProbeAccess::WriteCommand(0),
        ProbeAccess::ReadCommand,
        ProbeAccess::WriteBar(0, u32::MAX),
        ProbeAccess::WriteBar(1, u32::MAX),
        ProbeAccess::ReadBar(0),
        ProbeAccess::ReadBar(1),
        ProbeAccess::WriteBar(2, u32::MAX),
        ProbeAccess::ReadBar(2),
        ProbeAccess::WriteBar(3, u32::MAX),
        ProbeAccess::ReadBar(3),
        ProbeAccess::WriteBar(4, u32::MAX),
        ProbeAccess::ReadBar(4),
        ProbeAccess::WriteBar(5, u32::MAX),
        ProbeAccess::ReadBar(5),
        ProbeAccess::WriteCommand(0),
        ProbeAccess::ReadCommand,
        ProbeAccess::WriteBar(5, 0),
        ProbeAccess::WriteBar(4, 0),
        ProbeAccess::WriteBar(3, 0),
        ProbeAccess::WriteBar(2, 0x0000_C001),
        ProbeAccess::WriteBar(1, 1),
        ProbeAccess::WriteBar(0, 0x8000_0004),
        ProbeAccess::ReadBar(5),
        ProbeAccess::ReadBar(4),
        ProbeAccess::ReadBar(3),
        ProbeAccess::ReadBar(2),
        ProbeAccess::ReadBar(1),
        ProbeAccess::ReadBar(0),
        ProbeAccess::WriteCommand(command::IO_SPACE | command::MEMORY_SPACE | command::BUS_MASTER),
        ProbeAccess::ReadCommand,
    ];
    assert_eq!(fake.events(), expected.map(Some));
}

#[test]
fn every_probe_access_failure_restores_the_device() {
    // Calls 7 through 20 are the decode-off write and probe cycles. The
    // events after them are rollback itself and have a separate proof.
    for fail_at in 7..=20 {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.fail_at = Some(fail_at);

        assert_eq!(
            probe_bars(BARS, |event| fake.access(event)),
            Err(ProbeError::Access(fail_at)),
            "failure at cycle {fail_at}"
        );
        assert_eq!(fake.command, original.command, "command at cycle {fail_at}");
        assert_eq!(fake.status, original.status, "status at cycle {fail_at}");
        assert_eq!(fake.bars, original.bars, "BARs at cycle {fail_at}");
        assert_eq!(
            fake.events().last(),
            Some(&Some(ProbeAccess::ReadCommand)),
            "decode is restored last at cycle {fail_at}"
        );
    }
}

#[test]
fn a_restore_failure_still_attempts_every_later_restore() {
    for fail_at in 23..=28 {
        let mut fake = FakeConfig::device();
        let original = fake.clone();
        fake.fail_at = Some(fail_at);

        probe_bars(BARS, |event| fake.access(event))
            .expect("an applied restore write is verified by the later reads");
        assert_eq!(fake.command, original.command, "command at cycle {fail_at}");
        assert_eq!(fake.bars, original.bars, "BARs at cycle {fail_at}");
    }
}

#[test]
fn an_ignored_decode_off_write_is_refused_before_any_bar_access() {
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    fake.ignore_write_at = Some(7);

    assert_eq!(
        probe_bars(BARS, |event| fake.access(event)),
        Err(ProbeError::Refused(original.command))
    );
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.bars, original.bars);
    assert!(
        !fake.events()[7..]
            .iter()
            .any(|event| matches!(event, Some(ProbeAccess::WriteBar(_, _))))
    );
}

#[test]
fn a_restore_read_failure_keeps_decode_off() {
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    fake.fail_at = Some(30);

    assert_eq!(
        probe_bars(BARS, |event| fake.access(event)),
        Err(ProbeError::Restore(30))
    );
    assert_eq!(
        fake.command,
        original.command & !PROBE_DISABLED,
        "decode became live before every BAR was proven"
    );
    assert_eq!(fake.status, original.status);
    assert_eq!(fake.bars, original.bars);
    assert_eq!(fake.events().last(), Some(&Some(ProbeAccess::ReadBar(4))));
}

#[test]
fn an_uncertain_bar_restore_retains_exclusion_and_retries_exactly() {
    let supplied = command_address(0);
    let function = supplied.function_base();
    let authority = device_authority(supplied);
    let mut leases = CommandLeases::<1>::new();
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    fake.fail_at = Some(30);

    let failure = crate::probe::probe_bars_with_leases(&mut leases, &authority, |at, event| {
        assert_eq!(at, function);
        fake.access(event)
    })
    .expect_err("one restoration read was unavailable");
    assert_eq!(failure.reason(), &ProbeError::Restore(30));
    let retry = failure
        .into_retry()
        .expect("uncertain hardware retains its function reservation");
    assert_eq!(leases.begin_probe(&authority), Err(EnableError::Leased));
    assert_eq!(fake.command, original.command & !PROBE_DISABLED);

    fake.fail_at = None;
    crate::probe::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
        assert_eq!(at, function);
        fake.access(event)
    })
    .expect("the same owner restored every captured register");
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.status, original.status);
    assert_eq!(fake.bars, original.bars);

    let probe = leases
        .begin_probe(&authority)
        .expect("exact retry released the reservation");
    leases
        .finish_probe(&authority, probe)
        .expect("the new probe releases");
}

#[test]
fn a_retry_reproves_decode_is_off_before_restoring_bars() {
    let supplied = command_address(0);
    let function = supplied.function_base();
    let authority = device_authority(supplied);
    let mut leases = CommandLeases::<1>::new();
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    fake.fail_at = Some(30);

    let failure = crate::probe::probe_bars_with_leases(&mut leases, &authority, |at, event| {
        assert_eq!(at, function);
        fake.access(event)
    })
    .expect_err("one restoration read was unavailable");
    let retry = failure
        .into_retry()
        .expect("the uncertain restoration retains its owner");

    fake.fail_at = None;
    fake.command = original.command;
    let retry_start = fake.calls;
    fake.ignore_write_at = Some(retry_start);
    let failure =
        crate::probe::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
            assert_eq!(at, function);
            fake.access(event)
        })
        .expect_err("an ignored decode-off write cannot expose the BAR restore");
    assert_eq!(failure.reason(), &ProbeError::RestoreChanged);
    let retry = failure
        .into_retry()
        .expect("the failed recheck retains the same owner");
    assert_eq!(
        &fake.events()[retry_start..],
        &[
            Some(ProbeAccess::WriteCommand(
                original.command & !PROBE_DISABLED,
            )),
            Some(ProbeAccess::ReadCommand),
        ]
    );

    fake.ignore_write_at = None;
    crate::probe::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
        assert_eq!(at, function);
        fake.access(event)
    })
    .expect("an exact retry restores the captured device state");
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.bars, original.bars);
}

#[test]
fn ignored_bar_and_command_restore_writes_retain_retry_authority() {
    let supplied = command_address(0);
    let function = supplied.function_base();
    let authority = device_authority(supplied);
    let mut leases = CommandLeases::<1>::new();
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    fake.ignore_write_at = Some(26);

    let failure = crate::probe::probe_bars_with_leases(&mut leases, &authority, |at, event| {
        assert_eq!(at, function);
        fake.access(event)
    })
    .expect_err("an ignored BAR restoration write cannot be accepted");
    assert_eq!(failure.reason(), &ProbeError::RestoreChanged);
    let retry = failure
        .into_retry()
        .expect("the uncertain BAR restoration retains its owner");
    assert_eq!(leases.begin_probe(&authority), Err(EnableError::Leased));
    assert_eq!(fake.command, original.command & !PROBE_DISABLED);

    fake.ignore_write_at = None;
    crate::probe::retry_probe_with_leases(&mut leases, &authority, retry, |at, event| {
        assert_eq!(at, function);
        fake.access(event)
    })
    .expect("the same token retries the ignored BAR write");
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.bars, original.bars);

    let staged = crate::probe::probe_bars_with_leases(&mut leases, &authority, |at, event| {
        assert_eq!(at, function);
        fake.access(event)
    })
    .expect("a second probe stages its command");
    let (_, state) = staged.into_parts();
    fake.ignore_write_at = Some(fake.calls + 1);
    let failure = crate::command_state::restore_command_with_leases(
        &mut leases,
        &authority,
        state,
        |_, access| fake_command_access(&mut fake, access),
    )
    .expect_err("an ignored command restoration write cannot be accepted");
    assert_eq!(failure.reason(), EnableError::Refused(0));
    assert_eq!(leases.begin_probe(&authority), Err(EnableError::Leased));

    fake.ignore_write_at = None;
    crate::command_state::restore_command_with_leases(
        &mut leases,
        &authority,
        failure.into_state(),
        |_, access| fake_command_access(&mut fake, access),
    )
    .expect("the same command owner retries the ignored write");
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.bars, original.bars);
}

#[test]
fn an_ignored_final_probe_command_restore_retains_retry_authority() {
    let mut fake = FakeConfig::device();
    let original = fake.clone();
    let staged = probe_bars_unleased(BARS, |event| fake.access(event))
        .expect("a valid probe reaches its final command restore");
    fake.ignore_write_at = Some(fake.calls);

    let failure = restore_staged_probe(staged, &mut |event| fake.access(event))
        .expect_err("an ignored final command write cannot complete restoration");
    assert_eq!(failure.reason, ProbeError::RestoreChanged);
    let restore = failure
        .restore
        .expect("the failed readback retains restoration authority");
    assert_eq!(fake.command, original.command & !PROBE_DISABLED);

    fake.ignore_write_at = None;
    finish_probe_restoration(restore, &mut |event| fake.access(event))
        .expect("the retained authority retries the final command restore");
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.bars, original.bars);
}

#[test]
fn a_wide_bar_in_the_final_slot_is_refused_before_mutation() {
    let mut fake = FakeConfig::device();
    fake.bars[0] = 0x0000_C001;
    fake.bars[1] = 0x0000_0004;
    let original = fake.clone();

    assert_eq!(
        probe_bars(BRIDGE_BARS, |event| fake.access(event)),
        Err(ProbeError::Malformed)
    );
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.status, original.status);
    assert_eq!(fake.bars, original.bars);
    assert!(fake.events().iter().flatten().all(|event| !matches!(
        event,
        ProbeAccess::WriteCommand(_) | ProbeAccess::WriteBar(_, _)
    )));
}

#[test]
fn an_invalid_bar_mask_is_refused_after_restoration() {
    let mut fake = FakeConfig::device();
    fake.masks[2] = 0xFFFF_FFF5;
    let original = fake.clone();

    assert_eq!(
        probe_bars(BARS, |event| fake.access(event)),
        Err(ProbeError::InvalidWindow)
    );
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.status, original.status);
    assert_eq!(fake.bars, original.bars);
}

#[test]
fn a_misaligned_bar_base_is_refused_after_restoration() {
    let mut fake = FakeConfig::device();
    fake.bars = [0x0000_1800, 0, 0, 0, 0, 0];
    fake.masks = [0xFFFF_F000, 0, 0, 0, 0, 0];
    let original = fake.clone();

    assert_eq!(
        probe_bars(1, |event| fake.access(event)),
        Err(ProbeError::InvalidWindow)
    );
    assert_eq!(fake.command, original.command);
    assert_eq!(fake.status, original.status);
    assert_eq!(fake.bars, original.bars);
}
