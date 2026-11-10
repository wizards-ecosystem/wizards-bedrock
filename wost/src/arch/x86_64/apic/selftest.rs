use super::*;
use super::{InitializationObserver, InitializationRegisters};

#[cfg(feature = "selftest")]
static INIT_OBSERVATIONS: AtomicU8 = AtomicU8::new(0);

#[cfg(feature = "selftest")]
static INIT_OBSERVATION_FAILED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
const PUBLICATION_OBSERVATION: u8 = 1 << InitStage::ALL.len();

#[cfg(feature = "selftest")]
const ALL_INIT_OBSERVATIONS: u8 = PUBLICATION_OBSERVATION | (PUBLICATION_OBSERVATION - 1);

#[cfg(feature = "selftest")]
impl InitializationBoundary {
    const ALL: [Self; 6] = [
        Self::Stage(InitStage::Mode),
        Self::Stage(InitStage::Version),
        Self::Stage(InitStage::MaskSources),
        Self::Stage(InitStage::ConfigureSources),
        Self::Stage(InitStage::Identity),
        Self::Publication,
    ];

    const fn bit(self) -> u8 {
        match self {
            Self::Stage(stage) => 1 << stage_index(stage),
            Self::Publication => PUBLICATION_OBSERVATION,
        }
    }
}

#[cfg(feature = "selftest")]
fn is_pending(access: &impl InitializationRegisters, vector: InterruptVector) -> bool {
    let requests = pending_requests(access);
    x86_apic::pending::contains(&requests, vector)
}

/// Exercise the production pending-vector audit with a real queued self-IPI.
#[cfg(feature = "selftest")]
pub fn pending_audit_refuses_queued_self_interrupt_for_test(
    mut delivery_count: impl FnMut() -> usize,
) -> bool {
    const ATTEMPTS: usize = 1_000_000;

    if !interrupt::are_enabled() {
        return false;
    }
    let controller = controller();
    let Some(expected) = controller.published_vector(controller.self_interrupt_vector) else {
        return false;
    };
    let before = delivery_count();
    let excluded = interrupt::disable();
    excluded.commit_disabled();

    let sent = controller.send_self_interrupt().is_ok();
    let queued = (0..ATTEMPTS).any(|_| {
        core::hint::spin_loop();
        ready_registers().is_some_and(|access| is_pending(&access, expected))
    });
    // A timer may become pending after exclusion. The production door must
    // refuse whichever vector is highest, while `queued` above proves that
    // the self-IPI this witness sent is independently present in IRR.
    let refusal = matches!(
        enable_after_pending_audit(),
        Err(PendingInterruptError::Pending(_))
    );
    let stayed_excluded = !interrupt::are_enabled() && delivery_count() == before;

    if !interrupt::are_enabled() {
        interrupt::enable();
    }
    let arrived = (0..ATTEMPTS).any(|_| {
        core::hint::spin_loop();
        delivery_count() == before + 1
    });
    sent && queued && refusal && stayed_excluded && arrived && interrupt::are_enabled()
}

/// The timer's local vector entry, read back from the hardware rather than
/// remembered from what was written.
#[cfg(feature = "selftest")]
pub fn timer_entry() -> Entry {
    Entry::from_bits(low_u32(read(register::LVT_TIMER)))
}

/// Whether the APIC is software-enabled and therefore delivering.
#[cfg(feature = "selftest")]
pub fn is_enabled() -> bool {
    controller().is_enabled()
}

#[cfg(feature = "selftest")]
fn lifecycle() -> Lifecycle {
    controller().lifecycle()
}

#[cfg(feature = "selftest")]
pub(super) fn record_stage_observation(stage: InitStage, transaction: &InitTransaction) {
    let state = lifecycle();
    let valid = state == Lifecycle::Initializing
        && transaction.lifecycle() == Lifecycle::Initializing
        && interrupt_state() == Interrupts::Excluded
        && state.admit() == Err(BeginError::InProgress);
    INIT_OBSERVATION_FAILED.fetch_or(!valid, Ordering::Relaxed);
    INIT_OBSERVATIONS.fetch_or(1 << stage_index(stage), Ordering::Relaxed);
}

#[cfg(feature = "selftest")]
pub(super) fn record_publication_observation(transaction: &InitTransaction) {
    let state = lifecycle();
    let valid = state == Lifecycle::Initializing
        && transaction.lifecycle() == Lifecycle::Ready
        && interrupt_state() == Interrupts::Excluded
        && state.admit() == Err(BeginError::InProgress);
    INIT_OBSERVATION_FAILED.fetch_or(!valid, Ordering::Relaxed);
    INIT_OBSERVATIONS.fetch_or(PUBLICATION_OBSERVATION, Ordering::Relaxed);
}

#[cfg(feature = "selftest")]
const fn stage_index(stage: InitStage) -> u32 {
    match stage {
        InitStage::Mode => 0,
        InitStage::Version => 1,
        InitStage::MaskSources => 2,
        InitStage::ConfigureSources => 3,
        InitStage::Identity => 4,
    }
}

/// Whether the real boot transaction hid every partial stage from observers.
#[cfg(feature = "selftest")]
pub fn initialization_was_atomic_for_test() -> bool {
    INIT_OBSERVATIONS.load(Ordering::Acquire) == ALL_INIT_OBSERVATIONS
        && !INIT_OBSERVATION_FAILED.load(Ordering::Acquire)
        && lifecycle() == Lifecycle::Ready
}

/// Results from injecting every late APIC initialization refusal.
#[cfg(feature = "selftest")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitializationFailureProbe {
    unavailable_and_terminal: bool,
    ignored_spurious_write_is_terminal: bool,
    delivery_waited_for_unwind: bool,
    advertised_sources_masked: bool,
    source_mask_refusals_are_terminal: bool,
}

#[cfg(feature = "selftest")]
impl InitializationFailureProbe {
    /// Whether every refused stage stayed unpublished and rejected retry.
    pub const fn unavailable_and_terminal(self) -> bool {
        self.unavailable_and_terminal
    }

    /// Whether an ignored software-enable write refused and poisoned setup.
    pub const fn ignored_spurious_write_is_terminal(self) -> bool {
        self.ignored_spurious_write_is_terminal
    }

    /// Whether each pending self-interrupt arrived only after exclusion unwound.
    pub const fn delivery_waited_for_unwind(self) -> bool {
        self.delivery_waited_for_unwind
    }

    /// Whether six-entry and CMCI-capable inventories mask exactly their sources.
    pub const fn advertised_sources_masked(self) -> bool {
        self.advertised_sources_masked
    }

    /// Whether incomplete, unknown and unverified source inventories poison setup.
    pub const fn source_mask_refusals_are_terminal(self) -> bool {
        self.source_mask_refusals_are_terminal
    }
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy)]
enum SpuriousWrite {
    Retain,
    Ignore,
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy)]
enum LvtFault {
    None,
    IgnoreMask(usize),
    ClearMaskReadback(usize),
}

#[cfg(feature = "selftest")]
struct FixtureRegisters<'probe> {
    reads: &'probe AtomicUsize,
    writes: &'probe AtomicUsize,
    spurious: &'probe AtomicUsize,
    spurious_write: SpuriousWrite,
    version: u32,
    lvt_masked: &'probe AtomicU8,
    lvt_covered: &'probe AtomicU8,
    lvt_fault: LvtFault,
}

#[cfg(feature = "selftest")]
impl InitializationRegisters for FixtureRegisters<'_> {
    fn read(&self, register: Register) -> u64 {
        self.reads.fetch_add(1, Ordering::Relaxed);
        if register == register::VERSION {
            u64::from(self.version)
        } else if register == register::ID {
            0xA5
        } else if register == register::SPURIOUS_INTERRUPT_VECTOR {
            self.spurious.load(Ordering::Relaxed) as u64
        } else if let Some(index) = lvt_index(register) {
            let masked = self.lvt_masked.load(Ordering::Relaxed) & (1 << index) != 0;
            let corrupted = matches!(
                self.lvt_fault,
                LvtFault::ClearMaskReadback(fault) if fault == index
            );
            u64::from(Entry::from_bits(u32::from(masked && !corrupted) << 16).bits())
        } else {
            0
        }
    }

    fn write(&self, register: Register, value: u64) {
        self.writes.fetch_add(1, Ordering::Relaxed);
        if let Some(index) = lvt_index(register) {
            let bit = 1 << index;
            if Entry::from_bits(value as u32).is_masked() {
                if !matches!(self.lvt_fault, LvtFault::IgnoreMask(fault) if fault == index) {
                    self.lvt_masked.fetch_or(bit, Ordering::Relaxed);
                    self.lvt_covered.fetch_or(bit, Ordering::Relaxed);
                }
            } else {
                self.lvt_masked.fetch_and(!bit, Ordering::Relaxed);
            }
        }
        if register == register::SPURIOUS_INTERRUPT_VECTOR
            && matches!(self.spurious_write, SpuriousWrite::Retain)
        {
            self.spurious.store(value as usize, Ordering::Relaxed);
        }
    }
}

#[cfg(feature = "selftest")]
fn lvt_index(wanted: Register) -> Option<usize> {
    register::LOCAL_VECTOR_TABLE
        .iter()
        .position(|register| *register == wanted)
}

#[cfg(feature = "selftest")]
struct FailureObserver<'probe, DeliveryCount> {
    target: InitializationBoundary,
    mode_entered: &'probe AtomicBool,
    reads: &'probe AtomicUsize,
    writes: &'probe AtomicUsize,
    lvt_masked: &'probe AtomicU8,
    lvt_covered: &'probe AtomicU8,
    delivery_count: &'probe mut DeliveryCount,
    reached: u8,
    body_completed: bool,
    gates_held: bool,
    delivery_excluded: bool,
    queued_at: Option<usize>,
}

#[cfg(feature = "selftest")]
impl<DeliveryCount: FnMut() -> usize> InitializationObserver
    for FailureObserver<'_, DeliveryCount>
{
    fn reached(
        &mut self,
        boundary: InitializationBoundary,
        fixture: Controller<'_>,
    ) -> Result<(), ApicError> {
        if boundary != self.target {
            return Ok(());
        }

        let (expected_reads, expected_writes, expected_masked, expected_coverage) = match boundary {
            InitializationBoundary::Stage(InitStage::Mode) => (0, 0, 0, 0),
            InitializationBoundary::Stage(InitStage::Version) => (1, 0, 0, 0),
            InitializationBoundary::Stage(InitStage::MaskSources) => (7, 7, 0x3F, 0x3F),
            InitializationBoundary::Stage(InitStage::ConfigureSources) => (9, 10, 0x1F, 0x3F),
            InitializationBoundary::Stage(InitStage::Identity)
            | InitializationBoundary::Publication => (10, 10, 0x1F, 0x3F),
        };
        self.reached |= boundary.bit();
        self.body_completed = self.mode_entered.load(Ordering::Acquire)
            && self.reads.load(Ordering::Acquire) == expected_reads
            && self.writes.load(Ordering::Acquire) == expected_writes
            && self.lvt_masked.load(Ordering::Acquire) == expected_masked
            && self.lvt_covered.load(Ordering::Acquire) == expected_coverage;

        let delivered_before = (self.delivery_count)();
        let public_gates = !fixture.is_enabled()
            && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable);
        let reentry = matches!(
            Initialization::begin(fixture),
            Err(ApicError::InitializationInProgress)
        );
        let excluded = !interrupt::are_enabled();
        let queued = controller().send_self_interrupt().is_ok();
        let stayed_pending = (self.delivery_count)() == delivered_before;
        self.gates_held = public_gates && reentry;
        self.delivery_excluded = excluded && queued && stayed_pending;
        self.queued_at = queued.then_some(delivered_before);
        Err(ApicError::NotResponding)
    }
}

/// Exercise the production APIC transaction with isolated failure state.
///
/// Register reads and writes use bounded counters because replaying controller
/// setup would disturb the live singleton. The transaction owner, stage
/// runner, public-operation gates, interrupt flag and queued self-IPI are the
/// production paths; `delivery_count` observes that real interrupt arriving.
#[cfg(feature = "selftest")]
pub fn initialization_failures_for_test(
    vectors: Vectors,
    mut delivery_count: impl FnMut() -> usize,
) -> InitializationFailureProbe {
    const DELIVERY_ATTEMPTS: usize = 1_000_000;

    let live = controller();
    let mut unavailable_and_terminal = interrupt::are_enabled()
        && live.published_vector(live.self_interrupt_vector) == Some(vectors.self_interrupt)
        && live.published_vector(live.timer_vector) == Some(vectors.timer)
        && live.published_vector(live.spurious_vector) == Some(vectors.spurious);
    let mut delivery_waited_for_unwind = unavailable_and_terminal;
    let mut reached = 0u8;

    for target in InitializationBoundary::ALL {
        let state = AtomicU8::new(Lifecycle::Cold.bits());
        let self_interrupt_vector = AtomicU8::new(vectors.self_interrupt.get());
        let timer_vector = AtomicU8::new(vectors.timer.get());
        let spurious_vector = AtomicU8::new(vectors.spurious.get());
        let fixture = Controller {
            state: &state,
            self_interrupt_vector: &self_interrupt_vector,
            timer_vector: &timer_vector,
            spurious_vector: &spurious_vector,
        };
        let mode_entered = AtomicBool::new(false);
        let reads = AtomicUsize::new(0);
        let writes = AtomicUsize::new(0);
        let spurious = AtomicUsize::new(0);
        let lvt_masked = AtomicU8::new(0);
        let lvt_covered = AtomicU8::new(0);

        let (outcome, body_completed, gates_held, delivery_excluded, queued_at, observed) = {
            let mut observer = FailureObserver {
                target,
                mode_entered: &mode_entered,
                reads: &reads,
                writes: &writes,
                lvt_masked: &lvt_masked,
                lvt_covered: &lvt_covered,
                delivery_count: &mut delivery_count,
                reached: 0,
                body_completed: false,
                gates_held: false,
                delivery_excluded: false,
                queued_at: None,
            };
            let outcome = initialize_controller(
                fixture,
                vectors,
                Entry::fixed(vectors.error),
                Spurious::enabled(vectors.spurious),
                || {
                    mode_entered.store(true, Ordering::Release);
                    Ok((
                        Base::from_bits(0),
                        FixtureRegisters {
                            reads: &reads,
                            writes: &writes,
                            spurious: &spurious,
                            spurious_write: SpuriousWrite::Retain,
                            version: 0x0005_0014,
                            lvt_masked: &lvt_masked,
                            lvt_covered: &lvt_covered,
                            lvt_fault: LvtFault::None,
                        },
                    ))
                },
                &mut observer,
            );
            (
                outcome,
                observer.body_completed,
                observer.gates_held,
                observer.delivery_excluded,
                observer.queued_at,
                observer.reached,
            )
        };
        reached |= observed;

        let poisoned = fixture.lifecycle() == Lifecycle::Poisoned;
        let unavailable = !fixture.is_enabled()
            && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable);
        let retry_refused = matches!(Initialization::begin(fixture), Err(ApicError::Poisoned));
        unavailable_and_terminal &= outcome == Err(ApicError::NotResponding)
            && body_completed
            && gates_held
            && poisoned
            && unavailable
            && retry_refused;
        delivery_waited_for_unwind &= delivery_excluded;

        let arrived_after_unwind = queued_at.is_some_and(|before| {
            (0..DELIVERY_ATTEMPTS).any(|_| {
                core::hint::spin_loop();
                delivery_count() == before + 1
            })
        });
        delivery_waited_for_unwind &= interrupt::are_enabled() && arrived_after_unwind;
    }

    unavailable_and_terminal &= reached == ALL_INIT_OBSERVATIONS && live.is_enabled();
    InitializationFailureProbe {
        unavailable_and_terminal,
        ignored_spurious_write_is_terminal: ignored_spurious_write_is_terminal(vectors),
        delivery_waited_for_unwind,
        advertised_sources_masked: advertised_sources_masked(vectors),
        source_mask_refusals_are_terminal: source_mask_refusals_are_terminal(vectors),
    }
}

#[cfg(feature = "selftest")]
fn ignored_spurious_write_is_terminal(vectors: Vectors) -> bool {
    let state = AtomicU8::new(Lifecycle::Cold.bits());
    let self_interrupt_vector = AtomicU8::new(0);
    let timer_vector = AtomicU8::new(0);
    let spurious_vector = AtomicU8::new(0);
    let fixture = Controller {
        state: &state,
        self_interrupt_vector: &self_interrupt_vector,
        timer_vector: &timer_vector,
        spurious_vector: &spurious_vector,
    };
    let reads = AtomicUsize::new(0);
    let writes = AtomicUsize::new(0);
    let spurious = AtomicUsize::new(0);
    let lvt_masked = AtomicU8::new(0);
    let lvt_covered = AtomicU8::new(0);
    let entry_interrupts = interrupt::are_enabled();
    let mut observer = Unobserved;
    let outcome = initialize_controller(
        fixture,
        vectors,
        Entry::fixed(vectors.error),
        Spurious::enabled(vectors.spurious),
        || {
            Ok((
                Base::from_bits(0),
                FixtureRegisters {
                    reads: &reads,
                    writes: &writes,
                    spurious: &spurious,
                    spurious_write: SpuriousWrite::Ignore,
                    version: 0x0005_0014,
                    lvt_masked: &lvt_masked,
                    lvt_covered: &lvt_covered,
                    lvt_fault: LvtFault::None,
                },
            ))
        },
        &mut observer,
    );

    outcome == Err(ApicError::NotResponding)
        && reads.load(Ordering::Relaxed) == 9
        && writes.load(Ordering::Relaxed) == 10
        && lvt_covered.load(Ordering::Relaxed) == 0x3F
        && spurious.load(Ordering::Relaxed) == 0
        && fixture.lifecycle() == Lifecycle::Poisoned
        && self_interrupt_vector.load(Ordering::Relaxed) == 0
        && timer_vector.load(Ordering::Relaxed) == 0
        && spurious_vector.load(Ordering::Relaxed) == 0
        && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable)
        && matches!(Initialization::begin(fixture), Err(ApicError::Poisoned))
        && interrupt::are_enabled() == entry_interrupts
}

#[cfg(feature = "selftest")]
fn advertised_sources_masked(vectors: Vectors) -> bool {
    let mut complete = true;
    for (maximum, expected) in [(5u8, 0x3Fu8), (6u8, 0x7Fu8)] {
        let state = AtomicU8::new(Lifecycle::Cold.bits());
        let self_interrupt_vector = AtomicU8::new(0);
        let timer_vector = AtomicU8::new(0);
        let spurious_vector = AtomicU8::new(0);
        let fixture = Controller {
            state: &state,
            self_interrupt_vector: &self_interrupt_vector,
            timer_vector: &timer_vector,
            spurious_vector: &spurious_vector,
        };
        let reads = AtomicUsize::new(0);
        let writes = AtomicUsize::new(0);
        let spurious = AtomicUsize::new(0);
        let lvt_masked = AtomicU8::new(0);
        let lvt_covered = AtomicU8::new(0);
        let mut observer = Unobserved;
        let outcome = initialize_controller(
            fixture,
            vectors,
            Entry::fixed(vectors.error),
            Spurious::enabled(vectors.spurious),
            || {
                Ok((
                    Base::from_bits(0),
                    FixtureRegisters {
                        reads: &reads,
                        writes: &writes,
                        spurious: &spurious,
                        spurious_write: SpuriousWrite::Retain,
                        version: (u32::from(maximum) << 16) | 0x14,
                        lvt_masked: &lvt_masked,
                        lvt_covered: &lvt_covered,
                        lvt_fault: LvtFault::None,
                    },
                ))
            },
            &mut observer,
        );

        complete &= outcome.is_ok()
            && fixture.lifecycle() == Lifecycle::Ready
            && lvt_covered.load(Ordering::Relaxed) == expected
            && lvt_masked.load(Ordering::Relaxed) == (expected & !(1 << 5))
            && self_interrupt_vector.load(Ordering::Relaxed) == vectors.self_interrupt.get()
            && timer_vector.load(Ordering::Relaxed) == vectors.timer.get()
            && spurious_vector.load(Ordering::Relaxed) == vectors.spurious.get();
    }
    complete
}

#[cfg(feature = "selftest")]
fn source_mask_refusals_are_terminal(vectors: Vectors) -> bool {
    let mut terminal = source_fixture_refuses(vectors, 4, LvtFault::None)
        & source_fixture_refuses(vectors, 7, LvtFault::None);
    for index in 0..register::LOCAL_VECTOR_TABLE.len() {
        terminal &= source_fixture_refuses(vectors, 6, LvtFault::IgnoreMask(index));
        terminal &= source_fixture_refuses(vectors, 6, LvtFault::ClearMaskReadback(index));
    }
    terminal
}

#[cfg(feature = "selftest")]
fn source_fixture_refuses(vectors: Vectors, maximum: u8, fault: LvtFault) -> bool {
    let state = AtomicU8::new(Lifecycle::Cold.bits());
    let self_interrupt_vector = AtomicU8::new(0);
    let timer_vector = AtomicU8::new(0);
    let spurious_vector = AtomicU8::new(0);
    let fixture = Controller {
        state: &state,
        self_interrupt_vector: &self_interrupt_vector,
        timer_vector: &timer_vector,
        spurious_vector: &spurious_vector,
    };
    let reads = AtomicUsize::new(0);
    let writes = AtomicUsize::new(0);
    let spurious = AtomicUsize::new(0);
    let lvt_masked = AtomicU8::new(0);
    let lvt_covered = AtomicU8::new(0);
    let entry_interrupts = interrupt::are_enabled();
    let mut observer = Unobserved;
    let outcome = initialize_controller(
        fixture,
        vectors,
        Entry::fixed(vectors.error),
        Spurious::enabled(vectors.spurious),
        || {
            Ok((
                Base::from_bits(0),
                FixtureRegisters {
                    reads: &reads,
                    writes: &writes,
                    spurious: &spurious,
                    spurious_write: SpuriousWrite::Retain,
                    version: (u32::from(maximum) << 16) | 0x14,
                    lvt_masked: &lvt_masked,
                    lvt_covered: &lvt_covered,
                    lvt_fault: fault,
                },
            ))
        },
        &mut observer,
    );

    outcome == Err(ApicError::NotResponding)
        && fixture.lifecycle() == Lifecycle::Poisoned
        && self_interrupt_vector.load(Ordering::Relaxed) == 0
        && timer_vector.load(Ordering::Relaxed) == 0
        && spurious_vector.load(Ordering::Relaxed) == 0
        && spurious.load(Ordering::Relaxed) == 0
        && fixture.send_self_interrupt() == Err(DeliveryError::Unavailable)
        && matches!(Initialization::begin(fixture), Err(ApicError::Poisoned))
        && interrupt::are_enabled() == entry_interrupts
}
