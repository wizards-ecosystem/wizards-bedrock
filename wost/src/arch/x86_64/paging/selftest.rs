use super::*;

use super::activation::{Activation, activation_state};
use super::roots::{enclave_permission, observe_stripped_higher_half};
use super::walk::canonical_mapping_address;

impl UserMapError {
    /// Recover the unchanged run for a retry.
    #[cfg(feature = "selftest")]
    pub fn into_run(self) -> USegment {
        match self {
            Self::Mapping { run, .. } => run,
        }
    }
}

impl StackCleanup {
    #[cfg(feature = "selftest")]
    const fn address(&self) -> u64 {
        match self {
            Self::One(slot) => slot.stack_base(),
            Self::Prefix { layout, remaining } => match remaining.checked_sub(1) {
                Some(index) => match layout.slot(index) {
                    Some(slot) => slot.stack_base(),
                    None => layout.base(),
                },
                None => layout.base(),
            },
        }
    }
}

impl AddressSpace {
    /// Exercise map admission against one already-pinned stack guard.
    #[cfg(feature = "selftest")]
    pub fn pinned_stack_refuses_map_for_test(&mut self, slot: &Slot) -> Option<bool> {
        let roots = self.roots();
        let guard = slot.guard_base();
        let stack = slot.stack_base();
        let top = slot.stack_top().checked_sub(1)?;
        let expected_pin = protected_stack_overlap(roots, guard, PAGE_SIZE)?;
        let before_guard = self.lookup(guard);
        let before_stack = self.lookup(stack)?;
        let before_top = self.lookup(top)?;
        if before_guard.is_some() || expected_pin != stack {
            return Some(false);
        }

        let range = MappingRange::new(guard, before_stack.0, PAGE_SIZE).ok()?;
        let table_mark = self.tables.len();
        let owned_before = self.owned_pages;
        let result = self.map_range(
            guard,
            before_stack.0,
            PAGE_SIZE,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
            Grain::Pages,
        );
        let refused = matches!(result, Err(MapError::Pinned(address)) if address == stack);
        if result.is_ok() {
            self.undo(&range, PAGE_SIZE, Grain::Pages);
            self.rollback_tables(table_mark);
        }

        Some(
            refused
                && self.lookup(guard) == before_guard
                && self.lookup(stack) == Some(before_stack)
                && self.lookup(top) == Some(before_top)
                && self.tables.len() == table_mark
                && self.owned_pages == owned_before,
        )
    }

    /// Whether the paired user root contains exactly its admitted enclave.
    #[cfg(feature = "selftest")]
    pub fn user_root_contains_only_entry_enclave_for_test(&self) -> bool {
        let Some(roots) = self.roots else {
            return false;
        };
        let Some(stripped) = roots.stripped_root() else {
            return false;
        };
        if !self.entry_enclave_ready || self.reach != Reach::UserHalfOnly {
            return false;
        }
        let Some(enclave) = super::kpti::entry_enclave() else {
            return false;
        };
        let mappings = enclave.mappings();
        let Some(pages) = mappings.iter().try_fold(0usize, |total, mapping| {
            let pages = usize::try_from(mapping.section().mapped_len() / PAGE_SIZE).ok()?;
            total.checked_add(pages)
        }) else {
            return false;
        };
        let mut expected = Vec::new();
        if expected.try_reserve_exact(pages).is_err() {
            return false;
        }
        for mapping in mappings {
            let section = mapping.section();
            let mut address = section.start;
            while address < section.end {
                let Some((physical, _, _)) = walk(roots.full_root(), address) else {
                    return false;
                };
                let Ok(page) = EnclavePage::new(
                    address,
                    physical,
                    enclave_permission(mapping.stripped_access()),
                ) else {
                    return false;
                };
                expected.push(page);
                let Some(next) = address.checked_add(PAGE_SIZE) else {
                    return false;
                };
                address = next;
            }
        }
        let Ok(observed) = observe_stripped_higher_half(stripped, pages) else {
            return false;
        };
        if !enclave_matches(&expected, &observed) {
            return false;
        }
        for guard in enclave.guards() {
            let mut address = guard.start;
            while address < guard.end {
                if translate_in(roots.full_root(), address).is_some()
                    || translate_in(stripped, address).is_some()
                {
                    return false;
                }
                let Some(next) = address.checked_add(PAGE_SIZE) else {
                    return false;
                };
                address = next;
            }
        }
        if visit_present_leaves(stripped, 0, ENTRIES / 2, |address, _, flags, _| {
            if !flags.contains(Flags::USER) || flags.contains(Flags::GLOBAL) {
                return Err(MapError::InvalidEntryEnclave(address));
            }
            Ok(())
        })
        .is_err()
        {
            return false;
        }
        paired_flush_premise(roots).is_ok()
    }

    /// Bind after introducing only hardware `ACCESSED` drift in one shared root entry.
    #[cfg(feature = "selftest")]
    pub fn bind_with_accessed_drift_for_test(&mut self) -> Option<Result<Binding, ActivateError>> {
        self.bind_with_root_flag_drift_for_test(Flags::ACCESSED)
    }

    /// Bind after introducing cache-policy drift in one shared root entry.
    #[cfg(feature = "selftest")]
    pub fn bind_with_cache_drift_for_test(&mut self) -> Option<Result<Binding, ActivateError>> {
        self.bind_with_root_flag_drift_for_test(Flags::NO_CACHE)
    }

    #[cfg(feature = "selftest")]
    fn bind_with_root_flag_drift_for_test(
        &mut self,
        flag: Flags,
    ) -> Option<Result<Binding, ActivateError>> {
        if self.is_reclaimed() || self.reach != Reach::UserHalfOnly || !is_active() {
            return None;
        }
        let index = Level::Pml4.index(Self::bind as *const () as u64);
        let expected_slot = entry_slot(active_root(), index).ok()?;
        let found_slot = entry_slot(self.root(), index).ok()?;
        let expected = EntrySlot::read(&expected_slot);
        let found = EntrySlot::read(&found_slot);
        if !expected.is_present() || !expected.table_identity_matches(found) {
            return None;
        }
        let bits = if expected.flags().contains(flag) {
            found.bits() & !flag.bits()
        } else {
            found.bits() | flag.bits()
        };
        EntrySlot::write(&found_slot, Entry::from_bits(bits));
        let result = self.bind();
        EntrySlot::write(&found_slot, found);
        Some(result)
    }

    #[cfg(feature = "selftest")]
    /// Run one machine check under this compatible user root, then restore CR3.
    pub fn with_root_for_test<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        if self.is_reclaimed() || self.reach != Reach::UserHalfOnly || !is_active() {
            return None;
        }

        let disabled = interrupt::disable();
        let current = cpu::page_table_base();
        let compatible = current == active_root()
            && current != self.root()
            && (ENTRIES / 2..ENTRIES).all(|index| {
                let (Ok(expected), Ok(found)) =
                    (entry_slot(current, index), entry_slot(self.root(), index))
                else {
                    return false;
                };
                let expected = EntrySlot::read(&expected);
                let found = EntrySlot::read(&found);
                expected.table_identity_matches(found)
            });
        if !compatible {
            disabled.restore();
            return None;
        }

        // SAFETY: the whole higher half was just compared with the current root, and activation
        // froze new top-level entries there; `self` owns its live, aligned lower-half root here.
        unsafe { cpu::set_page_table_base(self.root()) };
        let result = operation();
        // SAFETY: `current` was the live root immediately before the temporary
        // switch, remains owned by the kernel, and maps this code and stack.
        unsafe { cpu::set_page_table_base(current) };
        disabled.restore();
        Some(result)
    }

    /// Device mappings this space currently owns.
    #[cfg(feature = "selftest")]
    pub fn device_mappings(&self) -> usize {
        self.devices.iter().flatten().count()
    }

    /// Continue the exact stack cleanup represented by `rollback`.
    #[cfg(feature = "selftest")]
    pub fn resume_stack_rollback(
        &mut self,
        rollback: StackRollback,
    ) -> Result<usize, StackRollback> {
        let StackRollback {
            returned_pages,
            roots,
            pending,
            ..
        } = rollback;
        if roots != self.roots() {
            return Err(StackRollback {
                reason: MapError::NotYours(pending.address()),
                returned_pages,
                roots,
                pending,
            });
        }
        match pending {
            StackCleanup::One(slot) => self.rollback_stack(&slot, returned_pages),
            StackCleanup::Prefix { layout, remaining } => {
                self.rollback_stack_prefix(&layout, remaining, returned_pages)
            }
        }
    }

    /// Prove physical zero is represented as a live root, not empty state.
    #[cfg(feature = "selftest")]
    pub fn zero_root_is_live_for_test(&mut self) -> bool {
        let saved = self.roots;
        self.roots = Some(AddressSpaceRoots::shared(0));
        let live = !self.is_reclaimed() && self.root() == 0;
        self.roots = saved;
        live
    }

    /// Exercise the permanent-root half independently of the current CR3.
    #[cfg(feature = "selftest")]
    pub fn permanent_root_blocks_reclaim_for_test(&self) -> bool {
        self.roots
            .is_some_and(|roots| Self::roots_are_active(roots, roots.full_root() ^ PAGE_SIZE))
    }

    /// Whether the device ledger independently blocks table reclaim.
    #[cfg(feature = "selftest")]
    pub fn devices_block_reclaim_for_test(&self) -> bool {
        matches!(self.device_reclaim_error(), Some(ReclaimError::Devices(_)))
    }

    /// Whether reservation refuses an existing huge top-level entry unchanged.
    #[cfg(feature = "selftest")]
    pub fn rejects_huge_shared_topology_for_test(&mut self) -> bool {
        let Ok(table) = Entry::table(self.root(), TablePrivilege::Supervisor) else {
            return false;
        };
        self.rejects_existing_shared_topology_for_test(Entry::from_bits(
            table.bits() | Flags::HUGE.bits(),
        ))
    }

    /// Whether reservation refuses an existing user top-level entry unchanged.
    #[cfg(feature = "selftest")]
    pub fn rejects_user_shared_topology_for_test(&mut self) -> bool {
        let Ok(table) = Entry::table(self.root(), TablePrivilege::User) else {
            return false;
        };
        self.rejects_existing_shared_topology_for_test(table)
    }

    #[cfg(feature = "selftest")]
    fn rejects_existing_shared_topology_for_test(&mut self, entry: Entry) -> bool {
        let Ok(slot) = entry_slot(self.root(), Level::Pml4.index(DEVICE_WINDOW_START)) else {
            return false;
        };
        if EntrySlot::read(&slot).is_present() {
            return false;
        }
        let mark = self.tables.len();
        EntrySlot::write(&slot, entry);
        let result = self.reserve_shared_kernel_topology();
        let unchanged = EntrySlot::read(&slot) == entry && self.tables.len() == mark;
        EntrySlot::write(&slot, Entry::ABSENT);
        matches!(result, Err(MapError::Conflict(at)) if at == DEVICE_WINDOW_START) && unchanged
    }

    /// Whether reservation creates a supervisor-only top-level table.
    #[cfg(feature = "selftest")]
    pub fn reserves_supervisor_shared_topology_for_test(&mut self) -> bool {
        let Ok(slot) = entry_slot(self.root(), Level::Pml4.index(DEVICE_WINDOW_START)) else {
            return false;
        };
        if EntrySlot::read(&slot).is_present() {
            return false;
        }
        let mark = self.tables.len();
        if self.reserve_shared_kernel_topology().is_err() {
            return false;
        }
        let entry = EntrySlot::read(&slot);
        entry.is_present()
            && !entry.is_huge()
            && !entry.flags().contains(Flags::USER)
            && mark.checked_add(1) == Some(self.tables.len())
            && self.tables.last().is_some_and(|table| table.link == entry)
    }

    /// Try to allocate one absent higher-half top-level table after activation.
    #[cfg(feature = "selftest")]
    pub fn kernel_topology_is_frozen_for_test(&mut self) -> bool {
        for index in ENTRIES / 2..ENTRIES {
            let Ok(slot) = entry_slot(self.root(), index) else {
                return false;
            };
            if EntrySlot::read(&slot).is_present() {
                continue;
            }
            let virtual_address = 0xffff_0000_0000_0000 | (index as u64) << 39;
            let mark = self.tables.len();
            return match self.take_owned_table(
                virtual_address,
                Level::Pml4,
                TablePrivilege::Supervisor,
            ) {
                Err(MapError::KernelTopologyFrozen(found)) => {
                    found == virtual_address
                        && self.tables.len() == mark
                        && !EntrySlot::read(&slot).is_present()
                }
                Ok((_, link)) => {
                    EntrySlot::write(&slot, link);
                    self.rollback_tables(mark);
                    false
                }
                Err(_) => false,
            };
        }
        true
    }
}

/// Doors refused for a reclaimed space, counted in each guard: a recycled table can answer alike.
#[cfg(feature = "selftest")]
pub(super) static RECLAIMED_REFUSALS: AtomicUsize = AtomicUsize::new(0);

/// How many doors have refused a reclaimed space.
#[cfg(feature = "selftest")]
pub fn reclaimed_refusals() -> usize {
    RECLAIMED_REFUSALS.load(Ordering::Acquire)
}

#[cfg(feature = "selftest")]
const PROBE_INITIAL_ROOT: PhysicalAddress = 0x1234_5000;

#[cfg(feature = "selftest")]
static PROBE_STATE: AtomicU8 = AtomicU8::new(ActivationState::Cold.bits());

#[cfg(feature = "selftest")]
static PROBE_ROOT: AtomicU64 = AtomicU64::new(PROBE_INITIAL_ROOT);

#[cfg(feature = "selftest")]
static PROBE_EXPECTED_ROOT: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "selftest")]
static PROBE_ARMED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "selftest")]
static PROBE_OBSERVATIONS: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "selftest")]
static PROBE_PARTIAL_OBSERVATIONS: AtomicUsize = AtomicUsize::new(0);

/// Results from pausing both paging-activation publication boundaries.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActivationPublicationProbe {
    initializing_is_excluded: bool,
    root_precedes_ready: bool,
}

#[cfg(feature = "selftest")]
impl ActivationPublicationProbe {
    /// Whether the initializing state remained unpublished under exclusion.
    pub const fn initializing_is_excluded(self) -> bool {
        self.initializing_is_excluded
    }

    /// Whether the root was published before readiness became observable.
    pub const fn root_precedes_ready(self) -> bool {
        self.root_precedes_ready
    }
}

#[cfg(feature = "selftest")]
#[derive(Clone, Copy)]
enum ProbeBoundary {
    BeforeRoot,
    AfterRoot,
}

/// Pause both activation boundaries with one pending self-interrupt each.
#[cfg(feature = "selftest")]
pub fn activation_publication_for_test() -> ActivationPublicationProbe {
    const FIRST_ROOT: PhysicalAddress = 0x2345_6000;
    const SECOND_ROOT: PhysicalAddress = 0x3456_7000;

    let initializing_is_excluded = probe_activation(ProbeBoundary::BeforeRoot, FIRST_ROOT);
    let root_precedes_ready = probe_activation(ProbeBoundary::AfterRoot, SECOND_ROOT);
    ActivationPublicationProbe {
        initializing_is_excluded,
        root_precedes_ready,
    }
}

#[cfg(feature = "selftest")]
fn probe_activation(boundary: ProbeBoundary, expected_root: PhysicalAddress) -> bool {
    const ATTEMPTS: usize = 1_000_000;

    if !interrupt::are_enabled()
        || PROBE_ARMED.load(Ordering::Acquire)
        || activation_state(&PROBE_STATE) != ActivationState::Cold
    {
        return false;
    }
    PROBE_ROOT.store(PROBE_INITIAL_ROOT, Ordering::Release);
    PROBE_EXPECTED_ROOT.store(expected_root, Ordering::Relaxed);
    let observations_before = PROBE_OBSERVATIONS.load(Ordering::Acquire);
    let partial_before = PROBE_PARTIAL_OBSERVATIONS.load(Ordering::Acquire);

    let mut boundary_holds = true;
    let mut sent = false;
    let activation = match boundary {
        ProbeBoundary::BeforeRoot => Activation::begin_with(&PROBE_STATE, &PROBE_ROOT, || {
            boundary_holds &= activation_state(&PROBE_STATE) == ActivationState::Initializing
                && PROBE_ROOT.load(Ordering::Acquire) == PROBE_INITIAL_ROOT
                && !interrupt::are_enabled();
            PROBE_ARMED.store(true, Ordering::Release);
            sent = super::apic::send_self_interrupt().is_ok();
            boundary_holds &= PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before;
        }),
        ProbeBoundary::AfterRoot => Activation::begin(&PROBE_STATE, &PROBE_ROOT),
    };
    let Ok(activation) = activation else {
        PROBE_ARMED.store(false, Ordering::Release);
        return false;
    };
    match boundary {
        ProbeBoundary::BeforeRoot => activation.commit(expected_root),
        ProbeBoundary::AfterRoot => {
            boundary_holds &= activation_state(&PROBE_STATE) == ActivationState::Initializing
                && PROBE_ROOT.load(Ordering::Acquire) == PROBE_INITIAL_ROOT
                && !interrupt::are_enabled();
            activation.commit_with(expected_root, || {
                boundary_holds &= activation_state(&PROBE_STATE) == ActivationState::Initializing
                    && PROBE_ROOT.load(Ordering::Acquire) == expected_root
                    && !interrupt::are_enabled();
                PROBE_ARMED.store(true, Ordering::Release);
                sent = super::apic::send_self_interrupt().is_ok();
                boundary_holds &= PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before;
            });
        }
    }

    let arrived = (0..ATTEMPTS).any(|_| {
        core::hint::spin_loop();
        PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before + 1
    });
    PROBE_ARMED.store(false, Ordering::Release);
    let holds = sent
        && arrived
        && boundary_holds
        && PROBE_OBSERVATIONS.load(Ordering::Acquire) == observations_before + 1
        && PROBE_PARTIAL_OBSERVATIONS.load(Ordering::Acquire) == partial_before
        && activation_state(&PROBE_STATE) == ActivationState::Ready
        && PROBE_ROOT.load(Ordering::Acquire) == expected_root
        && interrupt::are_enabled();
    PROBE_ROOT.store(PROBE_INITIAL_ROOT, Ordering::Relaxed);
    PROBE_STATE.store(ActivationState::Cold.bits(), Ordering::Release);
    holds
}

/// Record what a self-interrupt observed at an activation pause.
#[cfg(feature = "selftest")]
pub fn observe_activation_publication_for_test() {
    if !PROBE_ARMED.load(Ordering::Acquire) {
        return;
    }
    let expected_root = PROBE_EXPECTED_ROOT.load(Ordering::Relaxed);
    if activation_state(&PROBE_STATE) != ActivationState::Ready
        || PROBE_ROOT.load(Ordering::Acquire) != expected_root
    {
        PROBE_PARTIAL_OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
    }
    PROBE_OBSERVATIONS.fetch_add(1, Ordering::Release);
}

#[cfg(feature = "selftest")]
const STACK_REFUSAL_DISABLED: usize = usize::MAX;

#[cfg(feature = "selftest")]
pub(super) static STACK_MAPPINGS_BEFORE_REFUSAL: AtomicUsize =
    AtomicUsize::new(STACK_REFUSAL_DISABLED);

#[cfg(feature = "selftest")]
pub(super) static STACK_ROLLBACKS_BEFORE_REFUSAL: AtomicUsize =
    AtomicUsize::new(STACK_REFUSAL_DISABLED);

#[cfg(feature = "selftest")]
pub(super) static MAPPING_WRITES_BEFORE_REFUSAL: AtomicUsize =
    AtomicUsize::new(STACK_REFUSAL_DISABLED);

#[cfg(feature = "selftest")]
pub(super) static MAPPING_UNDOS_BEFORE_REFUSAL: AtomicUsize =
    AtomicUsize::new(STACK_REFUSAL_DISABLED);

#[cfg(feature = "selftest")]
pub(super) static DEVICE_UNMAPS_BEFORE_REFUSAL: AtomicUsize =
    AtomicUsize::new(STACK_REFUSAL_DISABLED);

#[cfg(feature = "selftest")]
pub(super) fn stack_refusal(counter: &AtomicUsize) -> bool {
    matches!(
        counter.fetch_update(
            Ordering::AcqRel,
            Ordering::Acquire,
            |remaining| match remaining {
                STACK_REFUSAL_DISABLED => None,
                0 => Some(0),
                count => Some(count - 1),
            }
        ),
        Ok(0)
    )
}

#[cfg(feature = "selftest")]
pub(super) fn mapping_refusal(counter: &AtomicUsize) -> bool {
    stack_refusal(counter)
}

#[cfg(feature = "selftest")]
pub(super) fn one_shot_mapping_refusal(counter: &AtomicUsize) -> bool {
    counter
        .compare_exchange(
            0,
            STACK_REFUSAL_DISABLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_ok()
}

/// Refuse a range mapping after `writes` leaf entries.
#[cfg(feature = "selftest")]
pub fn refuse_mapping_after_for_test(writes: usize) {
    MAPPING_WRITES_BEFORE_REFUSAL.store(writes, Ordering::Release);
}

/// Refuse range rollback after `clears` leaf entries.
#[cfg(feature = "selftest")]
pub fn refuse_mapping_undo_after_for_test(clears: usize) {
    MAPPING_UNDOS_BEFORE_REFUSAL.store(clears, Ordering::Release);
}

/// Refuse one device-unmap preflight without consuming its authority.
#[cfg(feature = "selftest")]
pub fn refuse_device_unmap_for_test() {
    DEVICE_UNMAPS_BEFORE_REFUSAL.store(0, Ordering::Release);
}

/// Clear range mapping and rollback refusals.
#[cfg(feature = "selftest")]
pub fn restore_mapping_refusals_for_test() {
    MAPPING_WRITES_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
    MAPPING_UNDOS_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
}

/// Refuse stack construction after `successes` mapped stacks.
#[cfg(feature = "selftest")]
pub fn refuse_stack_mapping_after_for_test(successes: usize) {
    STACK_MAPPINGS_BEFORE_REFUSAL.store(successes, Ordering::Release);
}

/// Refuse stack rollback after `successes` returned stacks.
#[cfg(feature = "selftest")]
pub fn refuse_stack_rollback_after_for_test(successes: usize) {
    STACK_ROLLBACKS_BEFORE_REFUSAL.store(successes, Ordering::Release);
}

/// Clear stack construction and rollback refusals.
#[cfg(feature = "selftest")]
pub fn restore_stack_refusals_for_test() {
    STACK_MAPPINGS_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
    STACK_ROLLBACKS_BEFORE_REFUSAL.store(STACK_REFUSAL_DISABLED, Ordering::Release);
}

/// Volatile-read one byte from a live owned mapping for a TLB machine check.
#[cfg(feature = "selftest")]
pub fn observe_owned_byte_for_test(address: u64) -> Option<u8> {
    let address = CanonicalAddress::new(address)?;
    let disabled = interrupt::disable();
    let Some((_, flags)) = translate(address.get()) else {
        disabled.restore();
        return None;
    };
    if !flags.contains(Flags::OWNED) || flags.contains(Flags::USER) {
        disabled.restore();
        return None;
    }
    // SAFETY: the live walk under interrupt exclusion proved this canonical
    // byte is backed by owned supervisor RAM in the current address space.
    let value = unsafe { (address.get() as *const u8).read_volatile() };
    disabled.restore();
    Some(value)
}

impl TaskStackPin {
    /// Replace a noncurrent stack's lower leaf during a hardware-use check.
    #[cfg(feature = "selftest")]
    pub(crate) fn rejects_lower_leaf_substitution_for_test(&self, index: usize) -> Option<bool> {
        let identity = self.stacks.get(index)?;
        identity.rejects_page_substitution_for_test(0, || self.stack_top(index).is_none())
    }
}

impl StackIdentity {
    #[cfg(feature = "selftest")]
    fn rejects_page_substitution_for_test(
        &self,
        page: usize,
        rejected: impl FnOnce() -> bool,
    ) -> Option<bool> {
        let pages = usize::try_from(self.slot.stack_len() / PAGE_SIZE).ok()?;
        if pages < 2 || page >= pages {
            return None;
        }
        let offset = u64::try_from(page).ok()?.checked_mul(PAGE_SIZE)?;
        let target_address = self.slot.stack_base().checked_add(offset)?;
        let target_end = target_address.checked_add(PAGE_SIZE)?;
        let other = if page == 0 { pages - 1 } else { 0 };
        let replacement_offset = u64::try_from(other).ok()?.checked_mul(PAGE_SIZE)?;
        let replacement_address = self.slot.stack_base().checked_add(replacement_offset)?;
        let pointer = cpu::stack_pointer();
        if (target_address..target_end).contains(&pointer) {
            return None;
        }

        let disabled = interrupt::disable();
        let result = (|| {
            let root = current_mapping_root(self.roots)?;
            if self.invalid_page_in(root).is_some() {
                return Some(false);
            }
            let target_before = walk(root, target_address)?;
            let replacement_before = walk(root, replacement_address)?;
            let expected_target = self.physical_base.checked_add(offset)?;
            let expected_replacement = self.physical_base.checked_add(replacement_offset)?;
            if target_before.0 != expected_target
                || replacement_before.0 != expected_replacement
                || target_before.2 != PAGE_SIZE
                || replacement_before.2 != PAGE_SIZE
            {
                return Some(false);
            }

            let slot = leaf_in(root, target_address).ok()?;
            let original = EntrySlot::read(&slot);
            let changed = Entry::page(Level::Pt, expected_replacement, original.flags()).ok()?;
            EntrySlot::write(&slot, changed);
            cpu::invalidate_page(canonical_mapping_address(target_address));

            let changed_walk = walk(root, target_address);
            let rejected = rejected();

            EntrySlot::write(&slot, original);
            cpu::invalidate_page(canonical_mapping_address(target_address));

            Some(
                changed_walk == Some((expected_replacement, target_before.1, target_before.2))
                    && rejected
                    && walk(root, target_address) == Some(target_before)
                    && walk(root, replacement_address) == Some(replacement_before)
                    && self.invalid_page_in(root).is_none(),
            )
        })();
        disabled.restore();
        result
    }
}

/// Replace one permanent stack's lower leaf and require its identity to refuse it.
#[cfg(feature = "selftest")]
pub fn permanent_stack_rejects_lower_leaf_substitution_for_test(base: u64) -> Option<bool> {
    let identity = PERMANENT_STACKS
        .lock_leaf()
        .iter()
        .flatten()
        .find(|identity| identity.slot.stack_base() == base)
        .copied()?;
    identity.rejects_page_substitution_for_test(0, || {
        current_mapping_root(identity.roots)
            .is_some_and(|root| identity.invalid_page_in(root) == Some(base))
    })
}

/// A permanent stack's dead span, painted so a later scan finds the deepest word written.
///
/// The paint is non-zero because a stack probe stores zero, and non-canonical so
/// no saved pointer can equal it.
#[cfg(feature = "selftest")]
#[derive(Debug)]
pub struct PaintedStack {
    identity: StackIdentity,
    span: core::ops::Range<u64>,
}

#[cfg(feature = "selftest")]
impl PaintedStack {
    const PAINT: u64 = 0x5741_5A44_5354_4B21;
    const WORD: u64 = size_of::<u64>() as u64;
    /// Room below the painter for its own calls, which a debug build makes for every word.
    const CLEARANCE: u64 = 1024;
    /// The scan runs no deeper than the paint, so half that room keeps the scan's calls clear.
    const SCAN_CLEARANCE: u64 = Self::CLEARANCE / 2;

    /// The lowest painted word anything has written since the paint.
    ///
    /// `None` when every word still holds the paint, or unless this frame runs on
    /// the painted stack in the same address space, at least half a kilobyte
    /// above the span.
    pub fn deepest_touch(&self) -> Option<u64> {
        let pointer = cpu::stack_pointer();
        let reach = self
            .identity
            .slot
            .span_below(pointer, Self::SCAN_CLEARANCE)?;
        if reach.end < self.span.end || current_mapping_root(self.identity.roots).is_none() {
            return None;
        }
        let mut address = self.span.start;
        while address < self.span.end {
            // SAFETY: the word is in the painted span of the pinned stack this frame runs on,
            // at least `SCAN_CLEARANCE` below it, so it is mapped and holds no live value.
            if unsafe { (address as *const u64).read_volatile() } != Self::PAINT {
                return Some(address);
            }
            address += Self::WORD;
        }
        None
    }
}

/// Paint the dead span of the permanent stack this code is running on.
///
/// Finds the live-root permanent stack that holds the stack pointer, re-walks
/// its guard and pages, and fills every word from its base to a clearance below
/// this frame.
#[cfg(feature = "selftest")]
pub fn paint_current_permanent_stack_for_test() -> Option<PaintedStack> {
    let pointer = cpu::stack_pointer();
    let identity = PERMANENT_STACKS
        .lock_leaf()
        .iter()
        .flatten()
        .find(|identity| identity.slot.usage_at(pointer).is_some())
        .copied()?;
    let root = current_mapping_root(identity.roots)?;
    if identity.invalid_page_in(root).is_some() {
        return None;
    }
    let span = identity.slot.span_below(pointer, PaintedStack::CLEARANCE)?;
    let mut address = span.start;
    while address < span.end {
        // SAFETY: the walk above proved this pinned stack mapped, owned and writable, and
        // `span_below` keeps the word between its base and `CLEARANCE` below this frame.
        unsafe { (address as *mut u64).write_volatile(PaintedStack::PAINT) };
        address += PaintedStack::WORD;
    }
    Some(PaintedStack { identity, span })
}

impl MappedStack {
    /// Check this token while a compatible user root is actually loaded.
    #[cfg(feature = "selftest")]
    pub fn is_installed_under_for_test(&self, space: &AddressSpace) -> Option<bool> {
        space.with_root_for_test(|| self.is_installed())
    }

    /// Replace one live leaf for the duration of an installation check.
    #[cfg(feature = "selftest")]
    pub fn rejects_substituted_page_for_test(&self, page: usize) -> Option<bool> {
        self.identity
            .rejects_page_substitution_for_test(page, || !self.is_installed())
    }
}

impl Unmapped {
    /// Hand back kernel-only frames, or `None` for ones ring 3 could reach, which must be cleared
    /// through [`USegment::reclaim`](crate::mm::USegment::reclaim).
    #[cfg(feature = "selftest")]
    pub fn into_kernel(mut self) -> Option<Segment> {
        if self.was_user {
            return None;
        }
        self.frames.take()
    }

    /// Hand back what ring 3 could reach as user frames; only [`USegment::reclaim`] makes them
    /// kernel frames, clearing the program's bytes.
    #[cfg(feature = "selftest")]
    pub fn into_user(mut self) -> Option<USegment> {
        self.was_user
            .then(|| self.frames.take())
            .flatten()
            .map(Into::into)
    }

    /// Whether ring 3 could reach the range this took away.
    #[cfg(feature = "selftest")]
    pub const fn was_user(&self) -> bool {
        self.was_user
    }
}

/// User frames cleared on the way back, counted without assuming the allocator's next frame.
#[cfg(feature = "selftest")]
pub(super) static CLEARED_ON_DROP: AtomicUsize = AtomicUsize::new(0);

/// Frames cleared on their way back from a user mapping.
#[cfg(feature = "selftest")]
pub fn cleared_on_drop() -> usize {
    CLEARED_ON_DROP.load(Ordering::Acquire)
}

#[cfg(feature = "selftest")]
enum DeviceSubstitution {
    Physical,
    CachePolicy,
}

impl DeviceMapping {
    /// Claim while a compatible user root is actually loaded by the CPU.
    #[cfg(feature = "selftest")]
    pub fn claim_releasable_under_for_test(
        self,
        space: &AddressSpace,
    ) -> (bool, Result<ReleasableDevice, Self>) {
        let mut mapping = Some(self);
        let outcome = space.with_root_for_test(|| {
            let Some(mapping) = mapping.take() else {
                panic!("a device mapping test consumed its token twice");
            };
            mapping.claim_releasable()
        });
        match outcome {
            Some(outcome) => (true, outcome),
            None => {
                let Some(mapping) = mapping else {
                    panic!("a device mapping test lost its unconsumed token");
                };
                (false, Err(mapping))
            }
        }
    }

    /// Substitute one physical leaf while exercising the real claim door.
    #[cfg(feature = "selftest")]
    pub fn rejects_physical_substitution_for_test(self, page: usize) -> (Self, bool) {
        self.rejects_substitution_for_test(page, DeviceSubstitution::Physical)
    }

    /// Substitute one leaf's cache policy while exercising the real claim door.
    #[cfg(feature = "selftest")]
    pub fn rejects_cache_substitution_for_test(self, page: usize) -> (Self, bool) {
        self.rejects_substitution_for_test(page, DeviceSubstitution::CachePolicy)
    }

    #[cfg(feature = "selftest")]
    fn rejects_substitution_for_test(
        self,
        page: usize,
        substitution: DeviceSubstitution,
    ) -> (Self, bool) {
        let Ok(page_count) = usize::try_from(self.pages / PAGE_SIZE) else {
            return (self, false);
        };
        if page_count < 2 || page >= page_count {
            return (self, false);
        }
        let Ok(page) = u64::try_from(page) else {
            return (self, false);
        };
        let Some(offset) = page.checked_mul(PAGE_SIZE) else {
            return (self, false);
        };
        let Some(address) = self.virtual_base.checked_add(offset) else {
            return (self, false);
        };
        let physical_base = self.physical - self.physical % PAGE_SIZE;
        let Some(expected) = physical_base.checked_add(offset) else {
            return (self, false);
        };
        let other = if page == 0 { page_count - 1 } else { 0 };
        let Ok(other) = u64::try_from(other) else {
            return (self, false);
        };
        let Some(replacement_offset) = other.checked_mul(PAGE_SIZE) else {
            return (self, false);
        };
        let Some(replacement) = physical_base.checked_add(replacement_offset) else {
            return (self, false);
        };

        let disabled = interrupt::disable();
        let Some(root) = current_mapping_root(self.roots) else {
            disabled.restore();
            return (self, false);
        };
        let Ok(slot) = leaf_in(root, address) else {
            disabled.restore();
            return (self, false);
        };
        let original = EntrySlot::read(&slot);
        if !page_identity_matches(
            original.address(),
            original.flags(),
            PAGE_SIZE,
            expected,
            self.expected_flags,
        ) {
            disabled.restore();
            return (self, false);
        }
        let changed = match substitution {
            DeviceSubstitution::Physical => Entry::page(Level::Pt, replacement, original.flags()),
            DeviceSubstitution::CachePolicy => {
                if !original.flags().contains(Flags::NO_CACHE)
                    || original.flags().contains(Flags::WRITE_THROUGH)
                {
                    disabled.restore();
                    return (self, false);
                }
                Entry::page(
                    Level::Pt,
                    expected,
                    original
                        .flags()
                        .difference(Flags::NO_CACHE)
                        .union(Flags::WRITE_THROUGH),
                )
            }
        };
        let Ok(changed) = changed else {
            disabled.restore();
            return (self, false);
        };

        EntrySlot::write(&slot, changed);
        cpu::invalidate_page(canonical_mapping_address(address));
        let (mapping, rejected) = match self.claim_releasable() {
            Err(mapping) => (mapping, true),
            Ok(ReleasableDevice {
                mapping,
                registers: _,
            }) => (mapping, false),
        };
        EntrySlot::write(&slot, original);
        cpu::invalidate_page(canonical_mapping_address(address));
        let restored = mapping.is_live();
        disabled.restore();
        (mapping, rejected && restored)
    }
}
