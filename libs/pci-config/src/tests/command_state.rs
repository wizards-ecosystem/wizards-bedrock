use super::*;
use crate::command_state::CommandPhase;

#[test]
fn command_state_is_bound_to_its_registry_function_and_command_offset() {
    let supplied = command_address(0);
    let expected = supplied
        .function_base()
        .at(header::COMMAND)
        .expect("the command register is aligned");
    let other = command_address(1)
        .function_base()
        .at(header::COMMAND)
        .expect("the command register is aligned");
    let mut first_fake = FakeCommand::new(0, u16::MAX);
    let mut second_fake = FakeCommand::new(0, u16::MAX);
    let mut first = CommandLeases::<1>::new();
    let mut second = CommandLeases::<1>::new();

    let first_state = enable_bus_master(&mut first, supplied, |at, event| {
        assert_eq!(at, expected);
        first_fake.access(event)
    })
    .expect("the first registry enabled its function");
    let second_state = enable_bus_master(&mut second, supplied, |at, event| {
        assert_eq!(at, expected);
        second_fake.access(event)
    })
    .expect("the second registry enabled its function");
    assert_eq!(first_state.lease.generation, second_state.lease.generation);
    assert_ne!(first_state.lease.registry, second_state.lease.registry);

    let mut wrong_function = duplicate_state(&first_state);
    wrong_function.lease.function.at = other.function_base();
    let mut accesses = 0;
    let wrong_function = commit_command(&mut first, wrong_function, |_, _| {
        accesses += 1;
        None
    })
    .expect_err("the token cannot be aimed at another function");
    assert_eq!(wrong_function.reason(), EnableError::Stale);
    assert_eq!(accesses, 0);

    let mut accesses = 0;
    let cross_registry = commit_command(&mut second, first_state, |_, _| {
        accesses += 1;
        None
    })
    .expect_err("another registry cannot consume the token");
    assert_eq!(cross_registry.reason(), EnableError::Stale);
    assert_eq!(accesses, 0);

    commit_command(&mut first, cross_registry.into_state(), |at, event| {
        assert_eq!(at, expected);
        first_fake.access(event)
    })
    .expect("the issuing registry retained its token");
    commit_command(&mut second, second_state, |at, event| {
        assert_eq!(at, expected);
        second_fake.access(event)
    })
    .expect("the other registry retained its own token");
}

#[test]
fn command_leases_refuse_nesting_and_preserve_uncertain_retry_authority() {
    let at = command_address(0);
    let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
    fake.fail_at = Some(4);
    let mut leases = CommandLeases::<2>::new();

    let failure = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect_err("the final rollback read was unavailable");
    assert_eq!(failure.reason(), EnableError::Restore);
    let state = failure
        .into_state()
        .expect("an uncertain write retains cleanup authority");
    let calls = fake.calls;
    let nested = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect_err("a live command transition is exclusive");
    assert_eq!(nested.reason(), EnableError::Leased);
    assert_eq!(nested.into_state(), None);
    assert_eq!(fake.calls, calls, "nesting performed an access");
    assert_eq!(
        leases.begin_probe(&device_authority(at)),
        Err(EnableError::Leased)
    );

    fake.fail_at = None;
    restore_command(&mut leases, state, |_, event| fake.access(event))
        .expect("the same uncertain token retries exactly");
    fake.writable = u16::MAX;
    let replacement = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect("exact restoration made room again");
    commit_command(&mut leases, replacement, |_, event| fake.access(event))
        .expect("the replacement remained current");
}

#[test]
fn command_consumption_requires_the_installed_word_and_live_generation() {
    let at = command_address(0);
    let installed = command::MEMORY_SPACE | command::BUS_MASTER;
    let mut fake = FakeCommand::new(0, u16::MAX);
    let mut leases = CommandLeases::<1>::new();

    let first = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect("the first generation was installed");
    let stale = duplicate_state(&first);
    commit_command(&mut leases, first, |_, event| fake.access(event))
        .expect("the first generation was committed");
    let current = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect("the next generation was reserved");

    let calls = fake.calls;
    let stale = commit_command(&mut leases, stale, |_, event| fake.access(event))
        .expect_err("a released generation cannot consume its replacement");
    assert_eq!(stale.reason(), EnableError::Stale);
    assert_eq!(fake.calls, calls, "a stale token reached the device");

    fake.command = installed | command::IO_SPACE;
    let calls = fake.calls;
    let changed = commit_command(&mut leases, current, |_, event| fake.access(event))
        .expect_err("a changed command word cannot be committed");
    assert_eq!(changed.reason(), EnableError::Changed(fake.command));
    assert_eq!(fake.calls, calls + 1);
    assert_eq!(fake.events()[calls], Some(CommandAccess::Read));

    fake.command = installed;
    commit_command(&mut leases, changed.into_state(), |_, event| {
        fake.access(event)
    })
    .expect("the unchanged token retries after the exact word returns");

    fake.command = 0;
    let restorable = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect("a restorable generation was installed");
    fake.command = installed | command::IO_SPACE;
    let calls = fake.calls;
    let changed = restore_command(&mut leases, restorable, |_, event| fake.access(event))
        .expect_err("restoration cannot overwrite an independently changed word");
    assert_eq!(changed.reason(), EnableError::Changed(fake.command));
    assert_eq!(fake.calls, calls + 1);
    assert_eq!(fake.events()[calls], Some(CommandAccess::Read));

    fake.command = installed;
    restore_command(&mut leases, changed.into_state(), |_, event| {
        fake.access(event)
    })
    .expect("restoration retries with the same token");
    assert_eq!(fake.command, 0);
}

#[test]
fn command_generation_exhaustion_refuses_before_configuration_access() {
    let mut leases = CommandLeases::<1>::new();
    leases.generation = u64::MAX;
    let mut accessed = false;

    let failure = enable_bus_master(&mut leases, command_address(0), |_, _| {
        accessed = true;
        Some(0)
    })
    .expect_err("a command generation cannot wrap");
    assert_eq!(failure.reason(), EnableError::Exhausted);
    assert_eq!(failure.into_state(), None);
    assert!(!accessed);
    assert!(leases.slots.iter().all(Option::is_none));
}

#[test]
fn a_command_upgrade_cannot_cross_command_registries() {
    let at = command_address(0);
    let mut fake = FakeCommand::new(0, u16::MAX);
    let mut issuing = CommandLeases::<1>::new();
    let mut other = CommandLeases::<1>::new();
    let mut state = enable_memory_space(&mut issuing, at, |_, event| fake.access(event))
        .expect("the issuing registry installed memory decode");
    let calls = fake.calls;

    assert_eq!(
        admit_bus_master(&mut other, &mut state, |_, event| fake.access(event)),
        Err(EnableError::Stale)
    );
    assert_eq!(fake.calls, calls, "a foreign registry reached the device");

    admit_bus_master(&mut issuing, &mut state, |_, event| fake.access(event))
        .expect("the issuing registry retained its upgrade authority");
    restore_command(&mut issuing, state, |_, event| fake.access(event))
        .expect("the issuing registry retained its restoration authority");
}

#[test]
fn a_clean_upgrade_refusal_can_retry_the_same_state() {
    let at = command_address(0);
    let mut fake = FakeCommand::new(0, u16::MAX);
    let mut leases = CommandLeases::<1>::new();
    let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
        .expect("memory decode was installed");
    fake.fail_at = Some(fake.calls + 1);

    assert_eq!(
        admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event)),
        Err(EnableError::Access)
    );
    assert_eq!(state.phase, CommandPhase::Installed);
    assert_eq!(fake.command, command::MEMORY_SPACE);

    fake.fail_at = None;
    admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
        .expect("exact rollback left the same state retryable");
    restore_command(&mut leases, state, |_, event| fake.access(event))
        .expect("the retried upgrade retained its original word");
    assert_eq!(fake.command, 0);
}

#[test]
fn every_bus_master_upgrade_access_refusal_preserves_its_owner() {
    let at = command_address(0);
    let memory_only = command::MEMORY_SPACE;

    for fail_at in 0..=2 {
        let mut fake = FakeCommand::new(0, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
            .expect("memory decode stage");
        fake.calls = 0;
        fake.trace = [None; COMMAND_TRACE_CAPACITY];
        fake.fail_at = Some(fail_at);

        assert_eq!(
            admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event)),
            Err(EnableError::Access),
            "access {fail_at}"
        );
        assert_eq!(fake.command, memory_only, "access {fail_at}");
        assert_eq!(state.phase, CommandPhase::Installed, "access {fail_at}");
        assert_eq!(
            leases.begin_probe(&device_authority(at)),
            Err(EnableError::Leased),
            "access {fail_at} lost the command owner"
        );

        fake.injected_read = None;
        fake.fail_at = None;
        admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
            .expect("the unchanged owner retries the upgrade");
        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("the retried owner restores the original command");
        assert_eq!(fake.command, 0, "access {fail_at}");
    }
}

#[test]
fn failed_bus_master_upgrade_rollback_never_loses_cleanup_authority() {
    let at = command_address(0);

    for fail_at in [3, 4] {
        let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
        let mut leases = CommandLeases::<1>::new();
        let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
            .expect("memory decode stage");
        fake.calls = 0;
        fake.trace = [None; COMMAND_TRACE_CAPACITY];
        fake.fail_at = Some(fail_at);

        let failure = admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
            .expect_err("the device refuses bus master");
        assert_eq!(fake.command, command::MEMORY_SPACE, "access {fail_at}");
        assert_eq!(
            leases.begin_probe(&device_authority(at)),
            Err(EnableError::Leased),
            "access {fail_at} lost the command owner"
        );

        if fail_at == 3 {
            assert_eq!(failure, EnableError::Refused(command::MEMORY_SPACE));
            assert_eq!(state.phase, CommandPhase::Installed);
        } else {
            assert_eq!(failure, EnableError::Restore);
            assert_eq!(
                state.phase,
                CommandPhase::Restoring(command::MEMORY_SPACE | command::BUS_MASTER)
            );
        }

        fake.fail_at = None;
        fake.writable = u16::MAX;
        restore_command(&mut leases, state, |_, event| fake.access(event))
            .expect("the retained owner restores the original command");
        assert_eq!(fake.command, 0, "access {fail_at}");
    }
}

#[test]
fn bus_master_upgrade_rejects_another_function_before_access() {
    let mut functions = FunctionClaims::<2>::new();
    let first = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("first function authority");
    let second = functions
        .claim(command_address(1), HeaderType::from_bits(0))
        .expect("second function authority");
    let mut fake = FakeCommand::new(0, u16::MAX);
    let mut state =
        crate::enable_memory_space(&mut functions, &first, |_, event| fake.access(event))
            .expect("first function memory decode");
    let calls = fake.calls;

    assert_eq!(
        crate::command_state::admit_bus_master(&mut functions, &second, &mut state, |_, event| {
            fake.access(event)
        }),
        Err(EnableError::Stale)
    );
    assert_eq!(fake.calls, calls, "a foreign function reached the device");

    crate::command_state::admit_bus_master(&mut functions, &first, &mut state, |_, event| {
        fake.access(event)
    })
    .expect("the issuing function retained the upgrade owner");
    crate::restore_command(&mut functions, &first, state, |_, event| fake.access(event))
        .expect("the issuing function restores its command");
    release_function(&mut functions, first).expect("first release");
    release_function(&mut functions, second).expect("second release");
}

#[test]
fn an_uncertain_command_state_cannot_be_committed() {
    let at = command_address(0);
    let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
    fake.fail_at = Some(4);
    let mut leases = CommandLeases::<1>::new();
    let failure = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect_err("the rollback read was unavailable");
    assert_eq!(failure.reason(), EnableError::Restore);
    let state = failure
        .into_state()
        .expect("uncertainty retains the command owner");
    let calls = fake.calls;

    let failure = commit_command(&mut leases, state, |_, event| fake.access(event))
        .expect_err("uncertain hardware cannot be committed");
    assert_eq!(failure.reason(), EnableError::Stale);
    assert_eq!(fake.calls, calls, "commit inspected uncertain hardware");

    fake.fail_at = None;
    fake.writable = u16::MAX;
    restore_command(&mut leases, failure.into_state(), |_, event| {
        fake.access(event)
    })
    .expect("the refused commit returned exact cleanup authority");
}

#[test]
fn restoration_retains_the_prewrite_word_across_access_loss() {
    let at = command_address(0);
    let mut fake = FakeCommand::new(0, u16::MAX);
    let mut leases = CommandLeases::<1>::new();
    let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
        .expect("memory decode was installed");
    let wanted = command::MEMORY_SPACE | command::BUS_MASTER;
    fake.injected_read = Some((fake.calls + 2, command::IO_SPACE));
    fake.fail_at = Some(fake.calls + 3);

    assert_eq!(
        admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event)),
        Err(EnableError::Restore)
    );
    assert_eq!(fake.command, wanted);
    assert_eq!(state.phase, CommandPhase::Restoring(wanted));

    fake.injected_read = None;
    fake.fail_at = Some(fake.calls + 1);
    let failure = restore_command(&mut leases, state, |_, event| fake.access(event))
        .expect_err("the first restoration write was unavailable");
    assert_eq!(failure.reason(), EnableError::Refused(wanted));
    assert_eq!(fake.command, wanted);

    fake.fail_at = None;
    restore_command(&mut leases, failure.into_state(), |_, event| {
        fake.access(event)
    })
    .expect("the same token retained both possible hardware words");
    assert_eq!(fake.command, 0);
}

#[test]
fn an_already_original_word_releases_after_its_readback() {
    let at = command_address(0);
    let required = command::MEMORY_SPACE | command::BUS_MASTER;
    let mut fake = FakeCommand::new(required, u16::MAX);
    let mut leases = CommandLeases::<1>::new();
    let state = enable_bus_master(&mut leases, at, |_, event| fake.access(event))
        .expect("the already enabled command was leased");

    restore_command(&mut leases, state, |_, event| fake.access(event))
        .expect("the captured word already read back exactly");
    assert_eq!(
        fake.events(),
        &[Some(CommandAccess::Read), Some(CommandAccess::Read)]
    );
}

#[test]
fn an_interrupted_upgrade_records_retry_authority_before_the_write() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let at = command_address(0);
    let mut fake = FakeCommand::new(0, u16::MAX);
    let mut leases = CommandLeases::<1>::new();
    let mut state = enable_memory_space(&mut leases, at, |_, event| fake.access(event))
        .expect("memory decode was installed first");
    let installed = state.installed;
    let wanted = installed | command::BUS_MASTER;

    let interrupted = catch_unwind(AssertUnwindSafe(|| {
        let _ = admit_bus_master(&mut leases, &mut state, |_, event| match event {
            CommandAccess::Read => fake.access(event),
            CommandAccess::Write(_) => panic!("injected interruption before the write"),
        });
    }));
    assert!(interrupted.is_err());
    assert_eq!(state.phase, CommandPhase::Restoring(wanted));
    assert_eq!(fake.command, installed);
    assert_eq!(
        leases.begin_probe(&device_authority(at)),
        Err(EnableError::Leased)
    );

    restore_command(&mut leases, state, |_, event| fake.access(event))
        .expect("the pre-recorded state retained exact restoration authority");
    assert_eq!(fake.command, 0);
}

#[test]
fn one_registry_identity_binds_every_simultaneous_lease() {
    let mut leases = CommandLeases::<2>::new();
    let first_authority = device_authority(command_address(0));
    let second_authority = device_authority(command_address(1));
    let first = leases
        .begin_probe(&first_authority)
        .expect("the first function was reserved");
    let second = leases
        .begin_probe(&second_authority)
        .expect("the second function was reserved");

    assert_ne!(first.0.registry, 0);
    assert_eq!(second.0.registry, first.0.registry);

    leases
        .finish_probe(&first_authority, first)
        .expect("first release");
    leases
        .finish_probe(&second_authority, second)
        .expect("second release");
}

#[test]
fn bus_master_enable_is_verified_and_preserves_the_command() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | OTHER;
    let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
    let mut fake = FakeCommand::new(original, u16::MAX);
    let mut leases = CommandLeases::<1>::new();

    let state = enable_bus_master(&mut leases, command_address(0), |_, event| {
        fake.access(event)
    })
    .expect("enabled");
    assert_eq!(fake.command, wanted);
    assert_eq!(
        fake.events(),
        &[
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(wanted)),
            Some(CommandAccess::Read),
        ]
    );

    let mut already = FakeCommand::new(wanted, u16::MAX);
    let mut already_leases = CommandLeases::<1>::new();
    let already_state = enable_bus_master(&mut already_leases, command_address(0), |_, event| {
        already.access(event)
    })
    .expect("enabled");
    assert_eq!(already.events(), &[Some(CommandAccess::Read)]);

    restore_command(&mut leases, state, |_, event| fake.access(event)).expect("restored");
    assert_eq!(fake.command, original);
    restore_command(&mut already_leases, already_state, |_, event| {
        already.access(event)
    })
    .expect("restored");
    assert_eq!(already.command, wanted);
}

#[test]
fn bus_master_enable_preserves_either_already_required_bit() {
    const OTHER: u16 = 1 << 9;
    const REQUIRED: u16 = command::MEMORY_SPACE | command::BUS_MASTER;

    for original in [OTHER | command::MEMORY_SPACE, OTHER | command::BUS_MASTER] {
        let mut fake = FakeCommand::new(original, u16::MAX);
        let mut leases = CommandLeases::<1>::new();
        let state = enable_bus_master(&mut leases, command_address(0), |_, event| {
            fake.access(event)
        })
        .expect("enabled");

        assert_eq!(fake.command, original | REQUIRED);
        restore_command(&mut leases, state, |_, event| fake.access(event)).expect("restored");
        assert_eq!(fake.command, original);
    }
}

#[test]
fn memory_decode_and_bus_master_are_two_exact_restorable_stages() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
    let mut fake = FakeCommand::new(original, u16::MAX);
    let mut leases = CommandLeases::<1>::new();

    let mut original_state = enable_memory_space(&mut leases, command_address(0), |_, event| {
        fake.access(event)
    })
    .expect("memory decode enabled");
    let memory_only = (original | command::MEMORY_SPACE) & !command::BUS_MASTER;
    assert_eq!(fake.command, memory_only);
    assert_eq!(fake.command & command::BUS_MASTER, 0);

    admit_bus_master(&mut leases, &mut original_state, |_, event| {
        fake.access(event)
    })
    .expect("bus master enabled later");
    assert_eq!(
        fake.command,
        original | command::MEMORY_SPACE | command::BUS_MASTER
    );

    restore_command(&mut leases, original_state, |_, event| fake.access(event))
        .expect("original restored");
    assert_eq!(fake.command, original);
    assert_eq!(
        fake.events(),
        &[
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(memory_only)),
            Some(CommandAccess::Read),
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(
                original | command::MEMORY_SPACE | command::BUS_MASTER,
            )),
            Some(CommandAccess::Read),
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(original)),
            Some(CommandAccess::Read),
        ]
    );
}

#[test]
fn retirement_disables_and_does_not_restore_inherited_bus_master() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
    let safe_original = original & !command::BUS_MASTER;
    let memory_only = (original | command::MEMORY_SPACE) & !command::BUS_MASTER;
    let admitted = original | command::MEMORY_SPACE | command::BUS_MASTER;
    let mut fake = FakeCommand::new(original, u16::MAX);
    let mut leases = CommandLeases::<1>::new();
    let mut state = enable_memory_space(&mut leases, command_address(0), |_, event| {
        fake.access(event)
    })
    .expect("memory decode enabled with inherited BME withheld");
    admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
        .expect("isolated bus mastering admitted");

    disable_bus_master_for_retirement(&mut leases, &mut state, |_, event| fake.access(event))
        .expect("retirement fenced bus mastering");
    assert_eq!(fake.command, memory_only);
    assert_eq!(state.before, safe_original);

    restore_command(&mut leases, state, |_, event| fake.access(event))
        .expect("all safe firmware command bits restored");
    assert_eq!(fake.command, safe_original);
    assert_eq!(fake.command & command::BUS_MASTER, 0);
    assert!(fake.events().windows(3).any(|events| events
        == [
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(admitted & !command::BUS_MASTER)),
            Some(CommandAccess::Read),
        ]));
}

#[test]
fn unit_requester_fence_withholds_and_never_restores_inherited_bus_master() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
    let safe = original & !command::BUS_MASTER;
    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeCommand::new(original, u16::MAX);

    let state =
        crate::fence_bus_master(&mut functions, &authority, |_, access| fake.access(access))
            .expect("requester fenced");
    assert_eq!(fake.command, safe);
    assert_eq!(state.before, safe);
    crate::verify_bus_master_fence(&mut functions, &authority, &state, |_, access| {
        fake.access(access)
    })
    .expect("fresh exact fence readback");

    crate::restore_command(&mut functions, &authority, state, |_, access| {
        fake.access(access)
    })
    .expect("safe command restoration");
    assert_eq!(fake.command, safe);
    crate::release_function(&mut functions, authority).expect("function returned");
}

#[test]
fn requester_fence_and_fresh_verification_retain_exact_failure_authority() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | command::BUS_MASTER | OTHER;

    for fail_at in 0..=2 {
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeCommand::new(original, u16::MAX);
        fake.fail_at = Some(fail_at);
        let failure =
            crate::fence_bus_master(&mut functions, &authority, |_, access| fake.access(access))
                .expect_err("injected fence refusal");
        fake.fail_at = None;
        if let Some(state) = failure.into_state() {
            crate::restore_command(&mut functions, &authority, state, |_, access| {
                fake.access(access)
            })
            .expect("uncertain fence restored by its exact owner");
        }
        crate::release_function(&mut functions, authority).expect("function returned");
    }

    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeCommand::new(original, u16::MAX);
    let state =
        crate::fence_bus_master(&mut functions, &authority, |_, access| fake.access(access))
            .expect("requester fenced");
    fake.command ^= command::IO_SPACE;
    assert!(matches!(
        crate::verify_bus_master_fence(&mut functions, &authority, &state, |_, access| fake
            .access(access)),
        Err(EnableError::Changed(_))
    ));
    fake.command ^= command::IO_SPACE;
    crate::restore_command(&mut functions, &authority, state, |_, access| {
        fake.access(access)
    })
    .expect("fence remains exactly retryable");
    crate::release_function(&mut functions, authority).expect("function returned");
}

#[test]
fn an_uncertain_retirement_transition_can_settle_disabled() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | command::BUS_MASTER | OTHER;
    let mut fake = FakeCommand::new(original, u16::MAX);
    let mut leases = CommandLeases::<1>::new();
    let mut state = enable_memory_space(&mut leases, command_address(0), |_, event| {
        fake.access(event)
    })
    .expect("memory decode enabled");
    let memory_only = fake.command;
    admit_bus_master(&mut leases, &mut state, |_, event| fake.access(event))
        .expect("isolated bus mastering admitted");
    let admitted = fake.command;

    fake.calls = 0;
    fake.trace = [None; COMMAND_TRACE_CAPACITY];
    fake.injected_read = Some((2, 1 << 10));
    fake.fail_at = Some(3);
    assert_eq!(
        disable_bus_master_for_retirement(&mut leases, &mut state, |_, event| {
            fake.access(event)
        }),
        Err(EnableError::Restore)
    );
    assert_eq!(fake.command, memory_only);
    assert_eq!(state.installed, admitted);
    assert_eq!(state.phase, CommandPhase::Restoring(memory_only));
    assert_eq!(state.before, original);

    fake.injected_read = None;
    fake.fail_at = None;
    disable_bus_master_for_retirement(&mut leases, &mut state, |_, event| fake.access(event))
        .expect("the two recorded words settle at BME off");
    assert_eq!(fake.command, memory_only);
    assert_eq!(state.phase, CommandPhase::Installed);
    assert_eq!(state.before, original & !command::BUS_MASTER);
    restore_command(&mut leases, state, |_, event| fake.access(event))
        .expect("the settled state remains exactly releasable");
    assert_eq!(fake.command, original & !command::BUS_MASTER);
}

#[test]
fn every_retirement_access_refusal_retains_exact_retry_authority() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | command::BUS_MASTER | OTHER;

    for fail_at in 1..=4 {
        let mut functions = FunctionClaims::<1>::new();
        let authority = functions
            .claim(command_address(0), HeaderType::from_bits(0))
            .expect("function authority");
        let mut fake = FakeCommand::new(original, u16::MAX);
        let mut state =
            crate::enable_memory_space(&mut functions, &authority, |_, event| fake.access(event))
                .expect("memory decode enabled");
        crate::command_state::admit_bus_master(
            &mut functions,
            &authority,
            &mut state,
            |_, event| fake.access(event),
        )
        .expect("isolated bus mastering admitted");

        fake.calls = 0;
        fake.trace = [None; COMMAND_TRACE_CAPACITY];
        fake.injected_read = (fail_at >= 3).then_some((2, 1 << 10));
        fake.fail_at = Some(fail_at);
        assert!(
            crate::disable_bus_master_for_retirement(
                &mut functions,
                &authority,
                &mut state,
                |_, event| fake.access(event),
            )
            .is_err(),
            "access {fail_at} unexpectedly completed"
        );
        assert_eq!(state.before, original, "access {fail_at}");

        fake.injected_read = None;
        fake.fail_at = None;
        crate::disable_bus_master_for_retirement(
            &mut functions,
            &authority,
            &mut state,
            |_, event| fake.access(event),
        )
        .expect("the same command owner retries retirement");
        assert_eq!(fake.command & command::BUS_MASTER, 0, "access {fail_at}");
        crate::restore_command(&mut functions, &authority, state, |_, event| {
            fake.access(event)
        })
        .expect("safe command state remains exactly releasable");
        assert_eq!(fake.command, original & !command::BUS_MASTER);
        release_function(&mut functions, authority).expect("settled function releases");
    }
}

#[test]
fn every_enable_error_describes_its_cause() {
    assert_eq!(
        EnableError::Access.to_string(),
        "configuration access failed"
    );
    assert_eq!(
        EnableError::Refused(0x00A5).to_string(),
        "the command register read back as 0x00a5"
    );
    assert_eq!(
        EnableError::Restore.to_string(),
        "the original command could not be verified"
    );
}

#[test]
fn a_refused_bus_master_enable_restores_the_exact_command() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | OTHER;
    let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
    let observed = wanted & !command::BUS_MASTER;
    let mut fake = FakeCommand::new(original, !command::BUS_MASTER);
    let mut leases = CommandLeases::<1>::new();

    assert_eq!(
        enable_bus_master(&mut leases, command_address(0), |_, event| fake
            .access(event))
        .map_err(|failure| failure.reason()),
        Err(EnableError::Refused(observed))
    );
    assert_eq!(fake.command, original);
    assert_eq!(
        fake.events(),
        &[
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(wanted)),
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(original)),
            Some(CommandAccess::Read),
        ]
    );
}

#[test]
fn a_bus_master_enable_refuses_and_restores_an_unwritten_command_bit() {
    const OTHER: u16 = 1 << 9;
    let original = OTHER;
    let wanted = original | command::MEMORY_SPACE | command::BUS_MASTER;
    let observed = wanted | command::IO_SPACE;
    let mut fake = FakeCommand::new(original, u16::MAX);
    fake.injected_read = Some((2, command::IO_SPACE));
    let mut leases = CommandLeases::<1>::new();

    assert_eq!(
        enable_bus_master(&mut leases, command_address(0), |_, event| fake
            .access(event))
        .map_err(|failure| failure.reason()),
        Err(EnableError::Refused(observed))
    );
    assert_eq!(fake.command, original);
    assert_eq!(
        fake.events(),
        &[
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(wanted)),
            Some(CommandAccess::Read),
            Some(CommandAccess::Write(original)),
            Some(CommandAccess::Read),
        ]
    );
}

#[test]
fn every_failed_bus_master_access_restores_or_reports_uncertainty() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | OTHER;

    for fail_at in 0..=2 {
        let mut fake = FakeCommand::new(original, u16::MAX);
        fake.fail_at = Some(fail_at);
        let mut leases = CommandLeases::<1>::new();
        let failure = enable_bus_master(&mut leases, command_address(0), |_, event| {
            fake.access(event)
        })
        .expect_err("the injected access must refuse enablement");
        assert_eq!(failure.reason(), EnableError::Access, "access {fail_at}");
        assert_eq!(failure.into_state(), None, "access {fail_at}");
        assert_eq!(fake.command, original, "access {fail_at}");
    }

    let refuses_bus_master = !command::BUS_MASTER;
    let mut restore_write = FakeCommand::new(original, refuses_bus_master);
    restore_write.fail_at = Some(3);
    let mut write_leases = CommandLeases::<1>::new();
    assert_eq!(
        enable_bus_master(&mut write_leases, command_address(0), |_, event| {
            restore_write.access(event)
        })
        .expect_err("rollback uncertainty")
        .reason(),
        EnableError::Restore
    );
    assert_ne!(restore_write.command, original);

    let mut restore_read = FakeCommand::new(original, refuses_bus_master);
    restore_read.fail_at = Some(4);
    let mut read_leases = CommandLeases::<1>::new();
    assert_eq!(
        enable_bus_master(&mut read_leases, command_address(0), |_, event| {
            restore_read.access(event)
        })
        .expect_err("rollback uncertainty")
        .reason(),
        EnableError::Restore
    );
    assert_eq!(restore_read.command, original);
}

#[test]
fn a_bus_master_lease_restores_the_exact_command_or_reports_uncertainty() {
    const OTHER: u16 = 1 << 9;
    let original = command::IO_SPACE | OTHER;
    let mut restored = FakeCommand::new(original, u16::MAX);
    let mut restored_leases = CommandLeases::<1>::new();
    let state = enable_bus_master(&mut restored_leases, command_address(0), |_, event| {
        restored.access(event)
    })
    .expect("enabled");
    assert_eq!(
        restore_command(&mut restored_leases, state, |_, event| {
            restored.access(event)
        }),
        Ok(())
    );
    assert_eq!(restored.command, original);

    let mut refused = FakeCommand::new(original, u16::MAX);
    let mut refused_leases = CommandLeases::<1>::new();
    let state = enable_bus_master(&mut refused_leases, command_address(0), |_, event| {
        refused.access(event)
    })
    .expect("enabled");
    refused.writable = 0;
    let failure = restore_command(&mut refused_leases, state, |_, event| refused.access(event))
        .expect_err("a masked restore must retain its retry authority");
    assert_ne!(refused.command, original);

    refused.writable = u16::MAX;
    restore_command(&mut refused_leases, failure.into_state(), |_, event| {
        refused.access(event)
    })
    .expect("the unchanged state retries exactly");
    assert_eq!(refused.command, original);
}

#[test]
fn public_bus_master_admission_changes_the_device_and_state() {
    let at = command_address(0);
    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(at, HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeCommand::new(0, u16::MAX);
    let mut state =
        crate::enable_memory_space(&mut functions, &authority, |_, access| fake.access(access))
            .expect("memory decode stage");

    crate::command_state::admit_bus_master(&mut functions, &authority, &mut state, |_, access| {
        fake.access(access)
    })
    .expect("DMA admission");
    let admitted = command::MEMORY_SPACE | command::BUS_MASTER;
    assert_eq!(fake.command, admitted);
    assert_eq!(state.installed, admitted);
    crate::restore_command(&mut functions, &authority, state, |_, access| {
        fake.access(access)
    })
    .expect("admitted command restores exactly");
    release_function(&mut functions, authority).expect("settled function authority releases");
}

#[test]
fn public_command_commit_consumes_the_live_sublease() {
    let at = command_address(0);
    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(at, HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeCommand::new(0, u16::MAX);
    let state = crate::command_state::enable_bus_master(&mut functions, &authority, |_, access| {
        fake.access(access)
    })
    .expect("verified command transition");

    crate::commit_command(&mut functions, &authority, state, |_, access| {
        fake.access(access)
    })
    .expect("the live command generation commits");
    assert_eq!(fake.command, command::MEMORY_SPACE | command::BUS_MASTER);
    release_function(&mut functions, authority).expect("commit released the command sublease");
}
