use super::*;

#[test]
fn function_claims_validate_headers_and_aliases_before_commit() {
    let at = Address::new(3, 4, 5, header::CLASS).expect("valid");
    let mut registry = FunctionClaims::<2>::new();
    assert_eq!(
        registry.claim(at, HeaderType::from_bits(2)),
        Err(FunctionClaimError::Header(2))
    );
    assert_eq!(registry.len(), 0);
    let authority = registry
        .claim(at, HeaderType::from_bits(0))
        .expect("a type-zero function is claimable");
    assert_eq!(authority.bar_registers(), BARS);
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.len(), 1);
    assert!(registry.is_claimed(at.at(header::BAR0).expect("valid alias")));
    assert_eq!(
        registry.claim(
            at.at(header::VENDOR).expect("valid alias"),
            HeaderType::from_bits(1)
        ),
        Err(FunctionClaimError::Taken)
    );
    assert_eq!(registry.len(), 1, "duplicate refusal changed the holder");
    assert!(registry.holds(&authority));
}

#[test]
fn function_authority_is_registry_bound_and_releases_exactly() {
    let first = Address::new(3, 4, 5, header::CLASS).expect("valid");
    let second = Address::new(3, 4, 6, header::VENDOR).expect("valid");
    let mut registry = FunctionClaims::<2>::new();
    let mut other = FunctionClaims::<2>::new();
    let first_authority = registry
        .claim(first, HeaderType::from_bits(0))
        .expect("first authority");
    let second_authority = registry
        .claim(second, HeaderType::from_bits(1))
        .expect("second authority");
    assert_eq!(second_authority.bar_registers(), BRIDGE_BARS);
    let foreign = other
        .claim(first, HeaderType::from_bits(0))
        .expect("the other registry has its own authority");
    let foreign = registry
        .release(foreign)
        .expect_err("a foreign token cannot release this registry");
    assert!(other.holds(&foreign));
    assert_eq!(registry.len(), 2, "a refused release changed the registry");

    registry
        .release(first_authority)
        .expect("the exact authority releases");
    assert!(!registry.is_claimed(first));
    assert!(registry.is_claimed(second));
    assert_eq!(registry.len(), 1);
    let replacement = registry
        .claim(first, HeaderType::from_bits(0))
        .expect("release made room for a new generation");
    assert_eq!(registry.len(), 2);
    registry.release(replacement).expect("replacement release");
    registry
        .release(second_authority)
        .expect("second exact release");
    other.release(foreign).expect("foreign issuer release");
}

#[test]
fn configuration_cycles_require_the_issuing_function_registry() {
    let at = command_address(0);
    let mut issuer = FunctionClaims::<2>::new();
    let authority = issuer
        .claim(at, HeaderType::from_bits(0))
        .expect("issuing function authority");
    let other_authority = issuer
        .claim(command_address(1), HeaderType::from_bits(0))
        .expect("other function authority");
    let mut foreign = FunctionClaims::<1>::new();
    let mut foreign_accesses = 0;

    let refusal = crate::command_state::enable_bus_master(&mut foreign, &authority, |_, _| {
        foreign_accesses += 1;
        Some(0)
    })
    .expect_err("another registry cannot spend the function authority");
    assert_eq!(refusal.reason(), EnableError::Stale);
    assert_eq!(refusal.into_state(), None);
    assert_eq!(foreign_accesses, 0, "a foreign token reached the device");
    assert_eq!(foreign.len(), 0);
    assert!(issuer.holds(&authority));

    let mut fake = FakeCommand::new(0, u16::MAX);
    let state = crate::command_state::enable_bus_master(&mut issuer, &authority, |_, access| {
        fake.access(access)
    })
    .expect("the issuing registry reaches its function");
    let mut state = state;
    let issued_accesses = fake.calls;
    assert_eq!(
        crate::disable_bus_master_for_retirement(
            &mut issuer,
            &other_authority,
            &mut state,
            |_, access| fake.access(access),
        ),
        Err(EnableError::Stale)
    );
    assert_eq!(
        fake.calls, issued_accesses,
        "a foreign registry reached retirement configuration"
    );
    crate::disable_bus_master_for_retirement(&mut issuer, &authority, &mut state, |_, access| {
        fake.access(access)
    })
    .expect("the issuing registry fences its function");
    crate::restore_command(&mut issuer, &authority, state, |_, access| {
        fake.access(access)
    })
    .expect("the issuing registry restores its command");
    release_function(&mut issuer, other_authority).expect("other function release");
    release_function(&mut issuer, authority).expect("settled function authority releases");
}

#[test]
fn function_release_waits_for_every_command_and_probe_sublease() {
    let at = command_address(0);
    let mut functions = FunctionClaims::<1>::new();
    let authority = functions
        .claim(at, HeaderType::from_bits(0))
        .expect("function authority");
    let mut fake = FakeCommand::new(0, !command::BUS_MASTER);
    fake.fail_at = Some(4);
    let failure =
        crate::command_state::enable_bus_master(&mut functions, &authority, |_, access| {
            fake.access(access)
        })
        .expect_err("rollback uncertainty keeps its command sublease");
    let state = failure.into_state().expect("uncertain command owner");

    let failure = release_function(&mut functions, authority)
        .expect_err("an uncertain command prevents function release");
    assert_eq!(failure.reason(), FunctionReleaseError::Active);
    let authority = failure.into_authority();
    assert!(functions.holds(&authority));
    assert_eq!(
        functions.claim(at, HeaderType::from_bits(0)),
        Err(FunctionClaimError::Taken)
    );

    fake.fail_at = None;
    fake.writable = u16::MAX;
    crate::restore_command(&mut functions, &authority, state, |_, access| {
        fake.access(access)
    })
    .expect("the uncertain command restored");
    let probe = functions
        .commands
        .begin_probe(&authority)
        .expect("BAR probe sublease");
    let failure = release_function(&mut functions, authority)
        .expect_err("a live BAR probe prevents function release");
    assert_eq!(failure.reason(), FunctionReleaseError::Active);
    let authority = failure.into_authority();
    functions
        .commands
        .finish_probe(&authority, probe)
        .expect("the exact BAR sublease settles");
    release_function(&mut functions, authority).expect("settled function authority releases");
    assert_eq!(functions.len(), 0);
}

#[test]
fn command_and_probe_subleases_cannot_cross_function_authorities() {
    let mut functions = FunctionClaims::<2>::new();
    let first = functions
        .claim(command_address(0), HeaderType::from_bits(0))
        .expect("first function authority");
    let second = functions
        .claim(command_address(1), HeaderType::from_bits(0))
        .expect("second function authority");
    let mut fake = FakeCommand::new(0, u16::MAX);
    let state = crate::command_state::enable_bus_master(&mut functions, &first, |_, access| {
        fake.access(access)
    })
    .expect("first command transition");
    let calls = fake.calls;
    let failure = crate::commit_command(&mut functions, &second, state, |_, access| {
        fake.access(access)
    })
    .expect_err("another function cannot consume command authority");
    assert_eq!(failure.reason(), EnableError::Stale);
    assert_eq!(fake.calls, calls, "a foreign function reached the device");
    crate::restore_command(&mut functions, &first, failure.into_state(), |_, access| {
        fake.access(access)
    })
    .expect("the issuing function retained its command owner");

    let probe = functions
        .commands
        .begin_probe(&first)
        .expect("first probe owner");
    let probe = functions
        .commands
        .finish_probe(&second, probe)
        .expect_err("another function cannot settle the probe owner");
    functions
        .commands
        .finish_probe(&first, probe)
        .expect("the issuing function settles the probe");
    release_function(&mut functions, first).expect("first release");
    release_function(&mut functions, second).expect("second release");
}
