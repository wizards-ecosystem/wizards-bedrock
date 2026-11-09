use super::*;

#[test]
fn every_lifecycle_error_renders_text() {
    for text in [
        BeginError::InProgress.to_string(),
        BeginError::Ready.to_string(),
        BeginError::Poisoned.to_string(),
        ProgressError::InterruptsEnabled.to_string(),
        ProgressError::OutOfOrder.to_string(),
        ProgressError::Incomplete.to_string(),
        ProgressError::Settled.to_string(),
    ] {
        assert!(text.contains("local apic"), "{text:?}");
    }
}

#[test]
fn every_stored_state_round_trips() {
    for state in [
        Lifecycle::Cold,
        Lifecycle::Initializing,
        Lifecycle::Ready,
        Lifecycle::Poisoned,
    ] {
        assert_eq!(Lifecycle::from_bits(state.bits()), Some(state));
    }
    assert_eq!(Lifecycle::from_bits(4), None);
    assert_eq!(Lifecycle::from_bits(u8::MAX), None);
}

#[test]
fn only_cold_hardware_can_begin() {
    assert_eq!(Lifecycle::Cold.admit(), Ok(()));
    assert_eq!(Lifecycle::Initializing.admit(), Err(BeginError::InProgress));
    assert_eq!(Lifecycle::Ready.admit(), Err(BeginError::Ready));
    assert_eq!(Lifecycle::Poisoned.admit(), Err(BeginError::Poisoned));
}

#[test]
fn every_late_refusal_is_unpublished_and_terminal() {
    for failed_stage in 0..=InitStage::ALL.len() {
        let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();

        for &stage in &InitStage::ALL[..failed_stage] {
            assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
            assert!(!transaction.lifecycle().is_ready());
        }

        assert_eq!(transaction.refuse(), Lifecycle::Poisoned);
        assert!(!transaction.lifecycle().is_ready());
        assert_eq!(
            InitTransaction::begin(transaction.lifecycle()),
            Err(BeginError::Poisoned),
            "failed stage {failed_stage}"
        );
    }
}

#[test]
fn every_intermediate_observation_refuses_reentry_and_readiness() {
    let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();

    for &stage in &InitStage::ALL {
        assert!(!transaction.lifecycle().is_ready());
        assert_eq!(
            InitTransaction::begin(transaction.lifecycle()),
            Err(BeginError::InProgress)
        );
        assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
    }
    assert!(!transaction.lifecycle().is_ready());
    assert_eq!(
        InitTransaction::begin(transaction.lifecycle()),
        Err(BeginError::InProgress)
    );
    assert_eq!(
        transaction.complete(Interrupts::Excluded),
        Ok(Lifecycle::Ready)
    );
    assert!(transaction.lifecycle().is_ready());
}

#[test]
fn every_hardware_boundary_and_publication_require_interrupt_exclusion() {
    for lost_at in 0..=InitStage::ALL.len() {
        let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();

        for &stage in &InitStage::ALL[..lost_at] {
            assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
        }

        let refusal = if let Some(&stage) = InitStage::ALL.get(lost_at) {
            transaction.enter(stage, Interrupts::Enabled)
        } else {
            transaction.complete(Interrupts::Enabled).map(|_| ())
        };
        assert_eq!(refusal, Err(ProgressError::InterruptsEnabled));
        assert_eq!(transaction.lifecycle(), Lifecycle::Poisoned);
    }
}

#[test]
fn publication_requires_every_stage_in_exact_order() {
    for completed in 0..InitStage::ALL.len() {
        let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();
        for &stage in &InitStage::ALL[..completed] {
            assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
        }
        assert_eq!(
            transaction.complete(Interrupts::Excluded),
            Err(ProgressError::Incomplete),
            "completed stages {completed}"
        );
        assert_eq!(transaction.lifecycle(), Lifecycle::Poisoned);
    }

    for expected_at in 0..InitStage::ALL.len() {
        for wrong in InitStage::ALL
            .iter()
            .copied()
            .filter(|stage| Some(stage) != InitStage::ALL.get(expected_at))
        {
            let mut transaction = InitTransaction::begin(Lifecycle::Cold).unwrap();
            for &stage in &InitStage::ALL[..expected_at] {
                assert_eq!(transaction.enter(stage, Interrupts::Excluded), Ok(()));
            }
            assert_eq!(
                transaction.enter(wrong, Interrupts::Excluded),
                Err(ProgressError::OutOfOrder),
                "expected stage {expected_at}, got {wrong:?}"
            );
            assert_eq!(transaction.lifecycle(), Lifecycle::Poisoned);
        }
    }
}
