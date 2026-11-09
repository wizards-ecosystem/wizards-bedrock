use super::*;
use crate::command_state::CommandPhase;

#[test]
fn public_memory_decode_accepts_only_its_function_authority() {
    let at = command_address(0);
    let mut issuer = FunctionClaims::<1>::new();
    let authority = issuer
        .claim(at, HeaderType::from_bits(0))
        .expect("issuing function authority");
    let mut foreign = FunctionClaims::<1>::new();
    let mut foreign_fake = FakeCommand::new(0, u16::MAX);

    let refusal = crate::enable_memory_space(&mut foreign, &authority, |_, access| {
        foreign_fake.access(access)
    })
    .expect_err("another registry cannot enable memory decode");
    assert_eq!(refusal.reason(), EnableError::Stale);
    assert_eq!(refusal.into_state(), None);
    assert_eq!(foreign_fake.calls, 0, "a foreign token reached the device");

    let mut fake = FakeCommand::new(0, u16::MAX);
    let state =
        crate::enable_memory_space(&mut issuer, &authority, |_, access| fake.access(access))
            .expect("the issuing registry enables memory decode");
    assert_eq!(fake.command, command::MEMORY_SPACE);
    crate::restore_command(&mut issuer, &authority, state, |_, access| {
        fake.access(access)
    })
    .expect("memory decode restores exactly");
    release_function(&mut issuer, authority).expect("settled function authority releases");
}

fn probe_claimed_bars<const N: usize>(
    functions: &mut FunctionClaims<N>,
    authority: &FunctionAuthority,
    fake: &mut FakeConfig,
) -> (ProbedBars, CommandState) {
    crate::probe_bars(functions, authority, |at, event| {
        assert_eq!(at, authority.address());
        fake.access(event)
    })
    .expect("the claimed function has valid BARs")
    .into_parts()
}

#[test]
fn every_error_text_names_its_condition() {
    for (text, expected) in [
        (FunctionClaimError::Taken.to_string(), "claimed"),
        (FunctionReleaseError::Active.to_string(), "sublease"),
        (BarAuthorityError::Busy.to_string(), "mapping authority"),
        (BarIdentityError::Changed.to_string(), "changed"),
        (
            BarEnableError::Identity(BarIdentityError::Access).to_string(),
            "unreadable",
        ),
    ] {
        assert!(text.contains(expected), "{text:?}");
    }
}

#[test]
fn a_bar_authority_names_its_index_and_refuses_foreign_or_forged_identities() {
    let mut functions = FunctionClaims::<2>::new();
    let first = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("first function");
    let second = functions
        .claim(command_address(1), HeaderType::from_bits(0))
        .expect("second function");
    let mut fake = FakeConfig::device();
    fake.bars = [0x8000_0000, 0x9000_0000, 0, 0, 0, 0];
    fake.masks = [0xFFFF_C000, 0xFFFF_F000, 0, 0, 0, 0];
    let (bars, mut state) = probe_claimed_bars(&mut functions, &first, &mut fake);
    let bar = bind_memory_bar(&mut functions, &first, &bars, 1).expect("BAR1 authority");
    assert_eq!(bar.index(), 1);

    let foreign = enable_memory_bar(&mut functions, &second, &bar, &mut state, |_, _| Some(0))
        .expect_err("a lease the authority was not bound to is refused on its own");
    assert_eq!(foreign, BarEnableError::Identity(BarIdentityError::Stale));

    let unmapped = BarAuthority {
        function: bar.function,
        index: 2,
        window: bar.window,
    };
    let stale = enable_memory_bar(&mut functions, &first, &unmapped, &mut state, |_, _| {
        Some(0)
    })
    .expect_err("an index the registry never mapped is stale");
    assert_eq!(stale, BarEnableError::Identity(BarIdentityError::Stale));
    let refused = release_memory_bar(&mut functions, unmapped)
        .expect_err("an index the registry never mapped cannot be released");
    assert_eq!(refused.to_string(), BarAuthorityError::Stale.to_string());

    release_memory_bar(&mut functions, bar).expect("the real authority returns");
    let held = release_function(&mut functions, first)
        .expect_err("a function with a live probe sublease stays claimed");
    assert!(held.to_string().contains("sublease"), "{held}");
    release_function(&mut functions, second).expect("an idle function releases");
}

#[test]
fn enabling_memory_decode_keeps_a_firmware_enabled_bit_and_refuses_a_retired_phase() {
    let mut functions = FunctionClaims::<1>::new();
    let function = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeConfig::device();
    let (bars, mut state) = probe_claimed_bars(&mut functions, &function, &mut fake);
    let bar = bind_memory_bar(&mut functions, &function, &bars, 0).expect("BAR0 authority");

    // The second admission finds decode already enabled and must keep it.
    for admission in 0..2 {
        let _ =
            enable_memory_bar(
                &mut functions,
                &function,
                &bar,
                &mut state,
                |_, event| match event {
                    BarEnableAccess::ReadBar(index) => Some(fake.bars[index]),
                    BarEnableAccess::Command(access) => fake_command_access(&mut fake, access),
                },
            );
        assert_ne!(
            fake.command & command::MEMORY_SPACE,
            0,
            "admission {admission} keeps decode"
        );
        assert_eq!(
            fake.command & command::BUS_MASTER,
            0,
            "admission {admission}"
        );
    }

    state.phase = CommandPhase::Restoring(fake.command);
    let stale =
        enable_memory_bar(
            &mut functions,
            &function,
            &bar,
            &mut state,
            |_, event| match event {
                BarEnableAccess::ReadBar(index) => Some(fake.bars[index]),
                BarEnableAccess::Command(access) => fake_command_access(&mut fake, access),
            },
        )
        .expect_err("a command transaction outside its installed phase is stale");
    assert_eq!(stale, BarEnableError::Command(EnableError::Stale));
}

#[test]
fn a_command_readback_that_differs_after_restoration_is_reported() {
    fn claimed() -> (FunctionClaims<1>, FunctionAuthority) {
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        (functions, authority)
    }
    fn position(fake: &FakeConfig, want: impl Fn(&ProbeAccess) -> bool, last: bool) -> usize {
        let events = fake.events();
        let matches = |event: &Option<ProbeAccess>| event.as_ref().is_some_and(&want);
        let found = if last {
            events.iter().rposition(matches)
        } else {
            events.iter().position(matches)
        };
        found.expect("the rehearsal performed the access")
    }

    let (mut rehearsal, rehearsed) = claimed();
    let mut clean = FakeConfig::device();
    let _ = probe_claimed_bars(&mut rehearsal, &rehearsed, &mut clean);
    let first_bar_write = position(
        &clean,
        |event| matches!(event, ProbeAccess::WriteBar(..)),
        false,
    );

    let (mut rehearsal, rehearsed) = claimed();
    let mut failing = FakeConfig::device();
    failing.fail_at = Some(first_bar_write);
    let _ = crate::probe_bars(&mut rehearsal, &rehearsed, |_, event| failing.access(event))
        .expect_err("the refused BAR write fails the probe");
    let restore_write = position(
        &failing,
        |event| matches!(event, ProbeAccess::WriteCommand(_)),
        true,
    );

    let (mut functions, authority) = claimed();
    let mut fake = FakeConfig::device();
    fake.fail_at = Some(first_bar_write);
    fake.ignore_write_at = Some(restore_write);
    let failure = crate::probe_bars(&mut functions, &authority, |_, event| fake.access(event))
        .expect_err("the device ignored the command restore after a failed probe");
    assert_eq!(failure.reason(), &ProbeError::RestoreChanged);
}

#[test]
fn a_bar_mapping_authority_is_linear_and_bound_to_its_probe_function() {
    let mut functions = FunctionClaims::<2>::new();
    let first = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("first function");
    let second = functions
        .claim(command_address(1), HeaderType::from_bits(0))
        .expect("second function");
    let mut fake = FakeConfig::device();
    let (bars, state) = probe_claimed_bars(&mut functions, &first, &mut fake);

    assert_eq!(
        bind_memory_bar(&mut functions, &second, &bars, 0),
        Err(BarAuthorityError::Stale)
    );
    assert_eq!(
        bind_memory_bar(&mut functions, &first, &bars, 2),
        Err(BarAuthorityError::NotMemory)
    );
    let bar = bind_memory_bar(&mut functions, &first, &bars, 0)
        .expect("the issuing function binds its memory BAR");
    assert_eq!(bar.index(), 0);
    assert_eq!(bar.window(), bars.window(0).expect("BAR0"));
    assert_eq!(
        bind_memory_bar(&mut functions, &first, &bars, 0),
        Err(BarAuthorityError::Busy)
    );

    let failure = release_function(&mut functions, first)
        .expect_err("a mapped BAR keeps its function claimed");
    assert_eq!(failure.reason(), FunctionReleaseError::Active);
    let first = failure.into_authority();
    release_memory_bar(&mut functions, bar).expect("exact BAR authority returns");
    crate::restore_command(&mut functions, &first, state, |_, access| {
        fake_command_access(&mut fake, access)
    })
    .expect("the staged probe restores");
    release_function(&mut functions, first).expect("the settled first function releases");
    release_function(&mut functions, second).expect("the untouched second function releases");
}

#[test]
fn changed_or_foreign_bar_identity_performs_zero_command_writes() {
    let mut functions = FunctionClaims::<2>::new();
    let first = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("first function");
    let second = functions
        .claim(command_address(1), HeaderType::from_bits(0))
        .expect("second function");
    let mut fake = FakeConfig::device();
    let (bars, mut state) = probe_claimed_bars(&mut functions, &first, &mut fake);
    let bar = bind_memory_bar(&mut functions, &first, &bars, 0).expect("BAR0 authority");

    let mut foreign_accesses = 0;
    let foreign = enable_memory_bar(&mut functions, &second, &bar, &mut state, |_, _| {
        foreign_accesses += 1;
        Some(0)
    })
    .expect_err("another function cannot spend the BAR authority");
    assert_eq!(foreign, BarEnableError::Identity(BarIdentityError::Stale));
    assert_eq!(foreign_accesses, 0);

    let mut command_writes = 0;
    let changed = enable_memory_bar(
        &mut functions,
        &first,
        &bar,
        &mut state,
        |_, event| match event {
            BarEnableAccess::ReadBar(0) => Some(fake.bars[0] + 0x4000),
            BarEnableAccess::ReadBar(1) => Some(fake.bars[1]),
            BarEnableAccess::ReadBar(_) => None,
            BarEnableAccess::Command(CommandAccess::Write(_)) => {
                command_writes += 1;
                Some(0)
            }
            BarEnableAccess::Command(CommandAccess::Read) => Some(u32::from(fake.command)),
        },
    )
    .expect_err("a changed BAR cannot enable decode");
    assert_eq!(changed, BarEnableError::Identity(BarIdentityError::Changed));
    assert_eq!(command_writes, 0);

    let inaccessible =
        enable_memory_bar(
            &mut functions,
            &first,
            &bar,
            &mut state,
            |_, event| match event {
                BarEnableAccess::ReadBar(0) => Some(fake.bars[0]),
                BarEnableAccess::ReadBar(1) => None,
                BarEnableAccess::ReadBar(_) => None,
                BarEnableAccess::Command(CommandAccess::Write(_)) => {
                    command_writes += 1;
                    Some(0)
                }
                BarEnableAccess::Command(CommandAccess::Read) => Some(u32::from(fake.command)),
            },
        )
        .expect_err("an incomplete wide identity cannot enable decode");
    assert_eq!(
        inaccessible,
        BarEnableError::Identity(BarIdentityError::Access)
    );
    assert_eq!(command_writes, 0);

    release_memory_bar(&mut functions, bar).expect("BAR authority returns");
    crate::restore_command(&mut functions, &first, state, |_, access| {
        fake_command_access(&mut fake, access)
    })
    .expect("the staged probe restores");
    release_function(&mut functions, first).expect("first function releases");
    release_function(&mut functions, second).expect("second function releases");
}

#[test]
fn every_type_only_bar_change_performs_zero_command_writes() {
    let mut functions = FunctionClaims::<1>::new();
    let function = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("function");
    let mut fake = FakeConfig::device();
    fake.bars = [0x8000_0000, 0, 0, 0, 0, 0];
    fake.masks = [0xFFFF_C000, 0, 0, 0, 0, 0];
    let (bars, mut state) = probe_claimed_bars(&mut functions, &function, &mut fake);
    let bar = bind_memory_bar(&mut functions, &function, &bars, 0).expect("BAR0 authority");

    for memory_type in 1..=3 {
        let mut command_writes = 0;
        let changed =
            enable_memory_bar(
                &mut functions,
                &function,
                &bar,
                &mut state,
                |_, event| match event {
                    BarEnableAccess::ReadBar(0) => Some(fake.bars[0] | memory_type << 1),
                    BarEnableAccess::ReadBar(_) => None,
                    BarEnableAccess::Command(CommandAccess::Write(_)) => {
                        command_writes += 1;
                        Some(0)
                    }
                    BarEnableAccess::Command(CommandAccess::Read) => Some(u32::from(fake.command)),
                },
            )
            .expect_err("a type-only BAR change cannot enable decode");
        assert_eq!(
            changed,
            BarEnableError::Identity(BarIdentityError::Changed),
            "memory type {memory_type:02b}"
        );
        assert_eq!(command_writes, 0, "memory type {memory_type:02b}");
    }

    release_memory_bar(&mut functions, bar).expect("BAR authority returns");
    crate::restore_command(&mut functions, &function, state, |_, access| {
        fake_command_access(&mut fake, access)
    })
    .expect("the staged probe restores");
    release_function(&mut functions, function).expect("function releases");
}

#[test]
fn exact_wide_bar_reads_precede_the_first_decode_write() {
    let mut functions = FunctionClaims::<1>::new();
    let function = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("function");
    let mut fake = FakeConfig::device();
    let (bars, mut state) = probe_claimed_bars(&mut functions, &function, &mut fake);
    let bar = bind_memory_bar(&mut functions, &function, &bars, 0).expect("BAR0 authority");
    let mut command = FakeCommand::new(0, u16::MAX);
    let mut trace = Vec::new();

    enable_memory_bar(&mut functions, &function, &bar, &mut state, |at, event| {
        trace.push(event);
        match event {
            BarEnableAccess::ReadBar(index) => {
                assert_eq!(
                    at,
                    function
                        .address()
                        .at(header::BAR0 + index as u16 * 4)
                        .expect("BAR address")
                );
                fake.bars.get(index).copied()
            }
            BarEnableAccess::Command(access) => {
                assert_eq!(
                    at,
                    function
                        .address()
                        .at(header::COMMAND)
                        .expect("command address")
                );
                command.access(access)
            }
        }
    })
    .expect("the unchanged mapped BAR enables memory decode");

    assert_eq!(
        trace,
        [
            BarEnableAccess::ReadBar(0),
            BarEnableAccess::ReadBar(1),
            BarEnableAccess::Command(CommandAccess::Read),
            BarEnableAccess::Command(CommandAccess::Write(command::MEMORY_SPACE)),
            BarEnableAccess::Command(CommandAccess::Read),
        ]
    );
    crate::restore_command(&mut functions, &function, state, |_, access| {
        command.access(access)
    })
    .expect("the command lease restores");
    release_memory_bar(&mut functions, bar).expect("BAR authority returns");
    release_function(&mut functions, function).expect("function releases");
}

#[test]
fn mapped_bus_master_admission_requires_its_bar_and_command_owner() {
    let mut functions = FunctionClaims::<2>::new();
    let first = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("first function");
    let second = functions
        .claim(command_address(1), HeaderType::from_bits(0))
        .expect("second function");
    let mut first_config = FakeConfig::device();
    let (first_bars, mut state) = probe_claimed_bars(&mut functions, &first, &mut first_config);
    let first_bar =
        bind_memory_bar(&mut functions, &first, &first_bars, 0).expect("first BAR authority");
    let mut second_config = FakeConfig::device();
    let (second_bars, second_state) =
        probe_claimed_bars(&mut functions, &second, &mut second_config);
    let second_bar =
        bind_memory_bar(&mut functions, &second, &second_bars, 0).expect("second BAR authority");
    let mut command = FakeCommand::new(0, u16::MAX);
    enable_memory_bar(
        &mut functions,
        &first,
        &first_bar,
        &mut state,
        |_, access| match access {
            BarEnableAccess::ReadBar(index) => first_config.bars.get(index).copied(),
            BarEnableAccess::Command(access) => command.access(access),
        },
    )
    .expect("first function memory decode");
    let calls = command.calls;

    assert_eq!(
        admit_mapped_bus_master(
            &mut functions,
            &second_bar,
            command_address(1),
            &mut state,
            |_, access| command.access(access)
        ),
        Err(EnableError::Stale)
    );
    assert_eq!(command.calls, calls, "a foreign BAR reached configuration");
    assert_eq!(
        admit_mapped_bus_master(
            &mut functions,
            &first_bar,
            command_address(1),
            &mut state,
            |_, access| command.access(access)
        ),
        Err(EnableError::ForeignDomain)
    );
    assert_eq!(
        command.calls, calls,
        "a domain for another function reached configuration"
    );

    admit_mapped_bus_master(
        &mut functions,
        &first_bar,
        command_address(0),
        &mut state,
        |_, access| command.access(access),
    )
    .expect("the mapped issuing BAR admits bus master behind its own domain");
    assert_eq!(command.command, command::MEMORY_SPACE | command::BUS_MASTER);
    crate::restore_command(&mut functions, &first, state, |_, access| {
        command.access(access)
    })
    .expect("the issuing function restores its command");
    crate::restore_command(&mut functions, &second, second_state, |_, access| {
        fake_command_access(&mut second_config, access)
    })
    .expect("the second staged probe restores");
    release_memory_bar(&mut functions, first_bar).expect("first BAR release");
    release_memory_bar(&mut functions, second_bar).expect("second BAR release");
    release_function(&mut functions, first).expect("first function release");
    release_function(&mut functions, second).expect("second function release");
}
