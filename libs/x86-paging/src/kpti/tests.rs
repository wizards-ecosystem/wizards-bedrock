use super::*;

const CODE: u64 = HIGHER_HALF_START + 0x1000;
const DATA: u64 = HIGHER_HALF_START + 0x2000;
const STACK: u64 = HIGHER_HALF_START + 0x3000;

fn expected_enclave() -> [EnclavePage; 3] {
    [
        EnclavePage::new(CODE, 0x4000, EnclavePermission::Executable).unwrap(),
        EnclavePage::new(DATA, 0x5000, EnclavePermission::ReadOnly).unwrap(),
        EnclavePage::new(STACK, 0x6000, EnclavePermission::Writable).unwrap(),
    ]
}

fn observed_enclave() -> [ObservedPage; 3] {
    [
        ObservedPage::new(
            STACK,
            0x6000,
            PAGE_SIZE,
            Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE | Flags::DIRTY,
        ),
        ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::ACCESSED),
        ObservedPage::new(DATA, 0x5000, PAGE_SIZE, Flags::PRESENT | Flags::NO_EXECUTE),
    ]
}

#[test]
fn a_root_pair_is_aligned_encodable_and_distinct() {
    let pair = RootPair::new(0x1000, 0x2000).unwrap();
    assert_eq!(pair.full(), 0x1000);
    assert_eq!(pair.stripped(), 0x2000);

    assert_eq!(
        RootPair::new(1, 0x2000),
        Err(RootPairError::MisalignedFull(1))
    );
    assert_eq!(
        RootPair::new(0x1000, 0x2001),
        Err(RootPairError::MisalignedStripped(0x2001))
    );
    assert_eq!(
        RootPair::new(MAX_PHYSICAL, 0x2000),
        Err(RootPairError::FullOutOfRange(MAX_PHYSICAL))
    );
    assert_eq!(
        RootPair::new(0x1000, MAX_PHYSICAL),
        Err(RootPairError::StrippedOutOfRange(MAX_PHYSICAL))
    );
    assert_eq!(
        RootPair::new(0x1000, 0x1000),
        Err(RootPairError::SameRoot(0x1000))
    );
}

#[test]
fn paired_flushes_require_pcid_off_and_non_global_leaves() {
    assert_eq!(
        FlushPremise::new(PcidState::Enabled, &[Flags::PRESENT]),
        Err(FlushPremiseError::PcidEnabled)
    );
    assert_eq!(
        FlushPremise::new(PcidState::Disabled, &[]),
        Err(FlushPremiseError::EmptyDomain)
    );
    assert_eq!(
        FlushPremise::new(
            PcidState::Disabled,
            &[Flags::PRESENT | Flags::USER, Flags::PRESENT | Flags::GLOBAL,]
        ),
        Err(FlushPremiseError::GlobalLeaf(1))
    );
    let premise = FlushPremise::new(
        PcidState::Disabled,
        &[
            Flags::PRESENT | Flags::USER,
            Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
        ],
    )
    .unwrap();
    assert_eq!(premise.leaf_count, 2);
}

#[test]
fn a_kpti_pair_shares_one_identical_user_subtree() {
    assert!(shared_user_link_matches(Entry::ABSENT, Entry::ABSENT));
    assert!(!shared_user_link_matches(
        Entry::from_bits(Flags::WRITABLE.bits()),
        Entry::from_bits(Flags::WRITABLE.bits())
    ));

    let full = Entry::table(0x3000, crate::TablePrivilege::User).unwrap();
    let accessed = Entry::from_bits(full.bits() | Flags::ACCESSED.bits());
    assert!(shared_user_link_matches(full, accessed));

    let other = Entry::table(0x4000, crate::TablePrivilege::User).unwrap();
    let supervisor = Entry::table(0x3000, crate::TablePrivilege::Supervisor).unwrap();
    let global = Entry::from_bits(full.bits() | Flags::GLOBAL.bits());
    assert!(!shared_user_link_matches(full, Entry::ABSENT));
    assert!(!shared_user_link_matches(full, other));
    assert!(!shared_user_link_matches(full, supervisor));
    assert!(!shared_user_link_matches(full, global));
}

#[test]
fn enclave_descriptors_are_aligned_high_half_pages() {
    let page = EnclavePage::new(CODE, 0x4000, EnclavePermission::Executable).unwrap();
    assert_eq!(page.virtual_address(), CODE);
    assert_eq!(page.physical_address(), 0x4000);
    assert_eq!(page.permission(), EnclavePermission::Executable);

    assert_eq!(
        EnclavePage::new(CODE + 1, 0x4000, EnclavePermission::Executable),
        Err(EnclavePageError::MisalignedVirtual(CODE + 1))
    );
    assert_eq!(
        EnclavePage::new(0x1000, 0x4000, EnclavePermission::Executable),
        Err(EnclavePageError::OutsideHigherHalf(0x1000))
    );
    assert_eq!(
        EnclavePage::new(CODE, 0x4001, EnclavePermission::Executable),
        Err(EnclavePageError::MisalignedPhysical(0x4001))
    );
    assert_eq!(
        EnclavePage::new(CODE, MAX_PHYSICAL, EnclavePermission::Executable),
        Err(EnclavePageError::PhysicalOutOfRange(MAX_PHYSICAL))
    );
}

#[test]
fn a_kpti_user_root_contains_only_the_entry_enclave() {
    let expected = expected_enclave();
    let observed = observed_enclave();
    assert!(enclave_matches(&expected, &observed));
    assert!(!enclave_matches(&[], &[]));
    assert!(!enclave_matches(&expected, &observed[..2]));

    let mut extra = [observed[0], observed[1], observed[2], observed[2]];
    extra[3] = ObservedPage::new(
        HIGHER_HALF_START + 0x4000,
        0x7000,
        PAGE_SIZE,
        Flags::PRESENT | Flags::NO_EXECUTE,
    );
    assert!(!enclave_matches(&expected, &extra));

    let duplicate_expected = [expected[0], expected[0], expected[2]];
    assert!(!enclave_matches(&duplicate_expected, &observed));

    let aliased_expected = [
        expected[0],
        EnclavePage::new(
            DATA,
            expected[0].physical_address,
            EnclavePermission::ReadOnly,
        )
        .unwrap(),
        expected[2],
    ];
    assert!(!enclave_matches(&aliased_expected, &observed));
}

#[test]
fn a_kpti_pair_refusal_returns_every_frame() {
    let empty = PairConstructionLedger::new();
    assert!(empty.refusal_returned_all(0));
    assert!(!empty.refusal_returned_all(1));

    let one_root = empty.acquire_root().unwrap();
    assert!(one_root.refusal_returned_all(1));
    assert!(!one_root.refusal_returned_all(0));
    assert!(!one_root.refusal_returned_all(2));
    assert_eq!(one_root.publish(), None);

    let roots = one_root.acquire_root().unwrap();
    assert!(roots.refusal_returned_all(2));
    assert_eq!(roots.acquire_root(), None);
    let mut authority = roots.publish().unwrap();
    assert!(authority.refusal_returned_all(2));

    for table_frames in 1..=8 {
        authority = authority.acquire_table().unwrap();
        let expected = 2 + table_frames;
        assert!(authority.refusal_returned_all(expected));
        assert!(!authority.refusal_returned_all(expected - 1));
        assert!(!authority.refusal_returned_all(expected + 1));
    }
}

#[test]
fn a_kpti_pair_cleanup_retains_complete_authority() {
    let roots = PairConstructionLedger::new()
        .acquire_root()
        .unwrap()
        .acquire_root()
        .unwrap();
    let mut authority = roots.publish().unwrap();
    for _ in 0..5 {
        authority = authority.acquire_table().unwrap();
    }

    assert_eq!(
        authority.settle(0),
        Ok(PairCleanupState::Retained(authority))
    );
    for partial in 1..7 {
        assert_eq!(
            authority.settle(partial),
            Err(PairCleanupError::Incomplete {
                expected: 7,
                returned: partial,
            })
        );
    }
    assert_eq!(authority.settle(7), Ok(PairCleanupState::Released));

    for remaining in (0..5).rev() {
        authority = authority.release_table().unwrap();
        assert_eq!(authority.table_frames(), remaining);
        assert_eq!(
            authority.settle(0),
            Ok(PairCleanupState::Retained(authority))
        );
    }
    assert_eq!(authority.release_table(), None);
    assert_eq!(authority.settle(2), Ok(PairCleanupState::Released));
}

#[test]
fn enclave_backing_span_and_every_permission_bit_are_exact() {
    let expected = expected_enclave();
    let observed = observed_enclave();
    for (name, replacement) in [
        (
            "backing",
            ObservedPage::new(CODE, 0x7000, PAGE_SIZE, Flags::PRESENT),
        ),
        (
            "span",
            ObservedPage::new(CODE, 0x4000, 2 * PAGE_SIZE, Flags::PRESENT),
        ),
        (
            "user",
            ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::USER),
        ),
        (
            "global",
            ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::GLOBAL),
        ),
        (
            "absent",
            ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::NONE),
        ),
        (
            "cache",
            ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::NO_CACHE),
        ),
        (
            "write-through cache",
            ObservedPage::new(
                CODE,
                0x4000,
                PAGE_SIZE,
                Flags::PRESENT | Flags::WRITE_THROUGH,
            ),
        ),
        (
            "huge",
            ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::HUGE),
        ),
        (
            "owned",
            ObservedPage::new(CODE, 0x4000, PAGE_SIZE, Flags::PRESENT | Flags::OWNED),
        ),
        (
            "writable read-only",
            ObservedPage::new(
                DATA,
                0x5000,
                PAGE_SIZE,
                Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
            ),
        ),
        (
            "write-execute",
            ObservedPage::new(STACK, 0x6000, PAGE_SIZE, Flags::PRESENT | Flags::WRITABLE),
        ),
    ] {
        let mut changed = observed;
        let index = changed
            .iter()
            .position(|page| page.virtual_address == replacement.virtual_address)
            .unwrap();
        changed[index] = replacement;
        assert!(
            !enclave_matches(&expected, &changed),
            "accepted {name}: {replacement:?}"
        );
    }
}

#[test]
fn transition_and_ist_stacks_have_adjacent_guards_absent_from_both_roots() {
    let transition =
        EntryStackRange::new(EntryStackKind::Transition, STACK, 2 * PAGE_SIZE).unwrap();
    let ist =
        EntryStackRange::new(EntryStackKind::Ist, HIGHER_HALF_START + 0x9000, PAGE_SIZE).unwrap();
    assert_eq!(transition.kind(), EntryStackKind::Transition);
    assert_eq!(transition.start(), STACK);
    assert_eq!(transition.len(), 2 * PAGE_SIZE);
    assert!(!transition.is_empty());
    assert_eq!(transition.lower_guard(), STACK - PAGE_SIZE);

    let full = [ObservedPage::new(
        transition.start(),
        0x6000,
        PAGE_SIZE,
        Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
    )];
    let stripped = [ObservedPage::new(
        ist.start(),
        0x7000,
        PAGE_SIZE,
        Flags::PRESENT | Flags::WRITABLE | Flags::NO_EXECUTE,
    )];
    assert!(stack_guards_are_absent(
        &[transition, ist],
        &full,
        &stripped
    ));

    for mapped_guard in [transition.lower_guard(), ist.lower_guard()] {
        let guard = [ObservedPage::new(
            mapped_guard,
            0x8000,
            PAGE_SIZE,
            Flags::PRESENT | Flags::NO_EXECUTE,
        )];
        assert!(!stack_guards_are_absent(
            &[transition, ist],
            &guard,
            &stripped
        ));
        assert!(!stack_guards_are_absent(&[transition, ist], &full, &guard));
    }
    assert!(!stack_guards_are_absent(&[], &full, &stripped));
    let invalid_observation = [ObservedPage::new(
        transition.start(),
        0x6001,
        PAGE_SIZE,
        Flags::PRESENT,
    )];
    assert!(!stack_guards_are_absent(
        &[transition, ist],
        &invalid_observation,
        &stripped
    ));
}

#[test]
fn entry_stack_geometry_refuses_partial_or_unrepresentable_ranges() {
    assert_eq!(
        EntryStackRange::new(EntryStackKind::Transition, STACK + 1, PAGE_SIZE),
        Err(EntryStackRangeError::MisalignedStart(STACK + 1))
    );
    assert_eq!(
        EntryStackRange::new(EntryStackKind::Transition, STACK, 0),
        Err(EntryStackRangeError::Empty)
    );
    assert_eq!(
        EntryStackRange::new(EntryStackKind::Transition, STACK, PAGE_SIZE + 1),
        Err(EntryStackRangeError::MisalignedLength(PAGE_SIZE + 1))
    );
    assert_eq!(
        EntryStackRange::new(EntryStackKind::Transition, HIGHER_HALF_START, PAGE_SIZE),
        Err(EntryStackRangeError::OutsideHigherHalf(HIGHER_HALF_START))
    );
    assert_eq!(
        EntryStackRange::new(
            EntryStackKind::Ist,
            u64::MAX - (PAGE_SIZE - 1),
            2 * PAGE_SIZE,
        ),
        Err(EntryStackRangeError::EndOverflow)
    );
}

#[test]
fn entry_transition_reaches_rust_only_in_the_required_order() {
    let ordered = [
        EntryEvent::LoadFullRoot,
        EntryEvent::TransferToKernelStack,
        EntryEvent::ScrubTransitionStack,
        EntryEvent::CallRust,
    ];
    let mut state = EntryTransition::new();
    for event in ordered {
        state = state.advance(event).unwrap();
    }
    assert!(state.is_complete());
    assert_eq!(state.advance(EntryEvent::CallRust), None);

    for forbidden in [
        EntryEvent::TransferToKernelStack,
        EntryEvent::ScrubTransitionStack,
        EntryEvent::CallRust,
    ] {
        assert_eq!(EntryTransition::new().advance(forbidden), None);
    }
}

#[test]
fn exit_transition_validates_then_loads_stripped_and_executes_verw() {
    let ordered = [
        ExitEvent::ValidateUserState,
        ExitEvent::ScrubTransitionStack,
        ExitEvent::CopyUserState,
        ExitEvent::LastSecretAccess,
        ExitEvent::LoadStrippedRoot,
        ExitEvent::Verw,
        ExitEvent::ReturnToUser,
    ];
    let mut state = ExitTransition::new();
    for event in ordered {
        state = state.advance(event).unwrap();
    }
    assert!(state.is_complete());
    assert_eq!(state.advance(ExitEvent::ReturnToUser), None);

    for forbidden in [
        ExitEvent::ScrubTransitionStack,
        ExitEvent::CopyUserState,
        ExitEvent::LastSecretAccess,
        ExitEvent::LoadStrippedRoot,
        ExitEvent::Verw,
        ExitEvent::ReturnToUser,
    ] {
        assert_eq!(ExitTransition::new().advance(forbidden), None);
    }

    let mut before_verw = ExitTransition::new();
    for event in ordered[..5].iter().copied() {
        before_verw = before_verw.advance(event).unwrap();
    }
    assert_eq!(before_verw.advance(ExitEvent::ReturnToUser), None);
    assert!(!before_verw.is_complete());
}
