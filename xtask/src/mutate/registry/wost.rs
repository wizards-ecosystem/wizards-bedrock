use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 161,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.preflight(&range, flags, grain)? == MappingState::Identical {\n            return Ok(());\n        }",
        replace: "        let _ = self.preflight(&range, flags, grain);",
        breaks: "mapping rollback clears only leaves this transaction created",
        reddens: Caught::Check("a refused remap preserves every older entry"),
    },
    Mutation {
        id: 162,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if let Some(stack) = protected_stack_overlap(self.roots(), virtual_base, len) {",
        replace: "        if let Some(stack) = protected_stack_overlap(self.roots(), virtual_base, len).filter(|_| false) {",
        breaks: "the safe map path can admit a mapping over a retained stack or guard",
        reddens: Caught::Check("protected stacks refuse overlapping maps"),
    },
    Mutation {
        id: 163,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        self.shared.push(keepalive);",
        replace: "        drop(keepalive);",
        breaks: "a shared mapping retains ownership of every frame it reaches",
        reddens: Caught::Check("a shared mapping keeps its frame"),
    },
    // `map_run` restores `OWNED`, so only the `map_boot` strip is load-bearing.
    Mutation {
        id: 164,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            flags.difference(NOT_THE_CALLERS),
            Grain::Largest,",
        replace: "            flags.difference(Flags::HUGE.union(Flags::GLOBAL)),
            Grain::Largest,",
        breaks: "a boot mapping cannot launder a frame into the allocator",
        reddens: Caught::Check("a boot mapping cannot claim ownership"),
    },
    Mutation {
        id: 165,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            reach: Reach::UserHalfOnly,",
        replace: "            reach: Reach::Everywhere,",
        breaks: "a user space cannot write the kernel's half",
        reddens: Caught::Check("a user space cannot reach the kernel half"),
    },
    Mutation {
        id: 166,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if flags.contains(Flags::USER) {\n            return Err(MapError::NotYours(virtual_base));\n        }\n        self.map_run(virtual_base, run, flags)",
        replace: "        self.map_run(virtual_base, run, flags)",
        breaks: "kernel memory cannot be published to ring 3",
        reddens: Caught::Check("kernel memory cannot be published to ring three"),
    },
    Mutation {
        id: 167,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if flags.contains(Flags::USER) {\n            return Err(MapError::NotYours(virtual_base));\n        }\n        if flags.intersection(CACHE_TYPE) != region.cache_type() {",
        replace: "        if flags.intersection(CACHE_TYPE) != region.cache_type() {",
        breaks: "boot evidence cannot publish kernel memory to ring 3",
        reddens: Caught::Check("a boot mapping cannot publish kernel memory"),
    },
    Mutation {
        id: 168,
        file: "wost/src/mm/mod.rs",
        find: "        if segment.zero() {\n            return Some(segment);\n        }",
        replace: "        return Some(segment);\n        #[allow(unreachable_code)]\n        if segment.zero() {\n            return Some(segment);\n        }",
        breaks: "a reclaimed frame is cleared before the kernel sees it",
        reddens: Caught::Check("a user page comes back cleared"),
    },
    Mutation {
        id: 170,
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if xstate::is_enabled() {",
        replace: "    if false && xstate::is_enabled() {",
        breaks: "extended state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        id: 171,
        file: "wost/src/arch/x86_64/task.rs",
        find: "    table.slots[current].thread_base = leaving;",
        replace: "",
        breaks: "thread-local storage follows the thread",
        reddens: Caught::Check("thread local storage follows the thread"),
    },
    Mutation {
        id: 172,
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if live {\n        Err(bound)\n    } else {\n        Ok(Released { roots })\n    }",
        replace: "    if false {\n        Err(bound)\n    } else {\n        Ok(Released { roots })\n    }",
        breaks: "a live task retains its address-space release authority",
        reddens: Caught::Check("a bound space is not reclaimed"),
    },
    Mutation {
        id: 173,
        file: "wost/src/arch/x86_64/task.rs",
        find: "        unsafe { cpu::set_page_table_base(wanted) };",
        replace: "",
        breaks: "a thread's address space is installed by the switch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::task_switch_loads_cr3_under_its_guard_before_context_state",
        },
    },
    // A ring-3 copy-window target must retire rather than enter kernel recovery (D-126).
    Mutation {
        id: 176,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "        code_selector: frame.code_selector(),",
        replace: "        code_selector: 0,",
        breaks: "the user-copy recovery adapter fabricates its saved code selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        id: 177,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if frames.zero() {",
        replace: "            if false && frames.zero() {",
        breaks: "a dropped user range is cleared before its frames go back",
        reddens: Caught::Check("a user page comes back cleared"),
    },
    Mutation {
        id: 180,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.bound {
            match released.as_ref() {",
        replace: "        if false && self.bound {
            match released.as_ref() {",
        breaks: "a bound space is not reclaimed under its own thread",
        reddens: Caught::Check("a bound space is not reclaimed"),
    },
    Mutation {
        id: 181,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if !is_active() || self.bound {",
        replace: "        if !is_active() {",
        breaks: "one address space can issue a second live binding",
        reddens: Caught::Check("one space has one binding"),
    },
    Mutation {
        id: 190,
        file: "wost/src/arch/x86_64/port.rs",
        find: "    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive,",
        replace: "    if !reserve(first, size_of::<T>()) {
        return None;
    }
    if !reserve(second, size_of::<U>()) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive,",
        breaks: "two claims are taken all at once or not at all",
        reddens: Caught::Check("a refused pair of claims strands neither"),
    },
    Mutation {
        id: 191,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "fn enable_write_protection() {
    write_cr0(read_cr0() | CR0_WRITE_PROTECT);
}",
        replace: "fn enable_write_protection() {
    write_cr0(read_cr0());
}",
        breaks: "the kernel enables supervisor write protection itself",
        reddens: Caught::Check("supervisor writes obey read-only pages"),
    },
    Mutation {
        id: 192,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "    unsafe { msr::write(EFER, msr::read(EFER) & !EFER_SYSCALL) };",
        replace: "",
        breaks: "the syscall door is shut before ring 3 runs",
        reddens: Caught::Check("ring three has no fast-syscall door"),
    },
    Mutation {
        id: 193,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pub const fn is_reclaimed(&self) -> bool {
        self.roots.is_none()
    }",
        replace: "    pub const fn is_reclaimed(&self) -> bool {
        match self.roots {
            None => true,
            Some(roots) => roots.full_root() == 0,
        }
    }",
        breaks: "a reclaimed space is not walked as physical frame zero",
        reddens: Caught::Check("a reclaimed space answers nothing"),
    },
    Mutation {
        id: 195,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "    BYTES.store(bytes, Ordering::Release);",
        replace: "    BYTES.store(bytes / 2, Ordering::Release);",
        breaks: "a save area is sized for the form that writes it",
        reddens: Caught::Check("a save stays inside its area"),
    },
    Mutation {
        id: 196,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"mov word ptr [rsp], {kernel_data}\",\n    \"verw word ptr [rsp]\",\n    \".if {record_clears}\",",
        replace: "    \"mov word ptr [rsp], {kernel_data}\",\n    \".if {record_clears}\",",
        breaks: "the buffer flush is the instruction, not its neighbour",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::buffer_clear_assembly_writes_are_guarded",
        },
    },
    // A register file mapped write-back, which an emulator forgives and
    // hardware does not (`012` §2).
    Mutation {
        id: 261,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            .union(Flags::NO_CACHE);",
        replace: "            .union(Flags::NONE);",
        breaks: "a device window is uncacheable",
        reddens: Caught::Check("the controller's registers are uncacheable"),
    },
    // Return a retained DMA segment to the frame allocator. Safe access is
    // still gone, so only the allocator count can distinguish this defect.
    Mutation {
        id: 299,
        file: "wost/src/mm/mod.rs",
        find: "        if let Some(segment) = self.segment.take() {\n            core::mem::forget(segment);\n        }",
        replace: "        if let Some(segment) = self.segment.take() {\n            drop(segment);\n        }",
        breaks: "retained dma memory never returns to the allocator",
        reddens: Caught::Check("uncertain dma memory is never recycled"),
    },
    // Forget every ownership entry after D-117's old fixed ceiling. The table
    // frames remain live and reachable, but reclamation silently loses them.
    Mutation {
        id: 302,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "for table in self.tables.drain(..) {",
        replace: "for table in self.tables.drain(..).take(32) {",
        breaks: "a large address space returns every table frame",
        reddens: Caught::Check("a large space returns every table frame"),
    },
    // A second fabricated launch reuses the stack address retired by the first.
    Mutation {
        id: 305,
        file: "wost/src/arch/x86_64/task.rs",
        find: "        let stack_index = table.slots[slot].stack_index;\n        table.slots[slot] = Slot {\n            stack_index,\n            ..Slot::VACANT\n        };",
        replace: "        table.slots[slot] = Slot::VACANT;",
        breaks: "a retired construction preserves its reusable kernel stack",
        reddens: Caught::Check("an exited slot is reused"),
    },
    Mutation {
        id: 313,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.owned_pages != 0 {",
        replace: "        if false && self.owned_pages != 0 {",
        breaks: "reachable owned pages prevent page-table reclamation",
        reddens: Caught::Check("cleanup refusals retain reachable tables"),
    },
    Mutation {
        id: 314,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            drop(returned);\n            remaining -= 1;",
        replace: "            drop(returned);\n            remaining = 0;",
        breaks: "stack rollback loses its exact remaining cleanup authority",
        reddens: Caught::Check("partial stack rollback is retryable"),
    },
    Mutation {
        id: 315,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if let Err(cause) = self.verify_guard(slot) {\n            return match self.rollback_stack(slot, 0) {\n                Ok(_) => Err(StackMapError::Mapping(cause)),\n                Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),\n            };\n        }",
        replace: "        if let Err(cause) = self.verify_guard(slot) {\n            let _ = self.rollback_stack(slot, 0);\n            return Err(StackMapError::Mapping(cause));\n        }",
        breaks: "guard rollback refusal retains typed ownership and progress",
        reddens: Caught::Check("guard rollback refusal retains the stack"),
    },
    Mutation {
        id: 319,
        file: "wost/src/mm/mod.rs",
        find: "        let released = FRAMES.lock_leaf().free(self.index);",
        replace: "        let released = FRAMES.lock_leaf().free(usize::MAX);",
        breaks: "an impossible frame-ledger refusal fails stopped",
        reddens: Caught::Panic {
            marker: "frame ownership ledger refused a live handle",
            why: "the corrupted live handle must stop before ownership can be forgotten",
        },
    },
    Mutation {
        id: 320,
        file: "wost/src/mm/mod.rs",
        find: "        let released = FRAMES.lock_leaf().free_contiguous(self.first, self.count);",
        replace: "        let released = FRAMES.lock_leaf().free_contiguous(usize::MAX, self.count);",
        breaks: "an impossible segment-ledger refusal fails stopped",
        reddens: Caught::Panic {
            marker: "segment ownership ledger refused a live handle",
            why: "the corrupted live segment must stop before ownership can be forgotten",
        },
    },
    // Erase what makes a forgotten guard distinct from an ordinary holder;
    // the lock still excludes, so the witness must read the reason.
    Mutation {
        id: 323,
        file: "wost/src/sync.rs",
        find: "        self.lock.forgotten_for_test.store(true, Ordering::Release);",
        replace: "        self.lock.forgotten_for_test.store(false, Ordering::Release);",
        breaks: "deliberately forgotten ownership is distinct from ordinary contention",
        reddens: Caught::Check("forgotten lock ownership is distinct from contention"),
    },
    // Leave the failed controller's ownership record behind; the sibling
    // still maps, so only the selection ledger sees the incomplete retirement.
    Mutation {
        id: 325,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let Some(slot) = self.devices.get_mut(mapping.device_slot) else {\n            panic!(\"a preflighted device ledger slot disappeared\");\n        };\n        *slot = None;",
        replace: "        let _ = mapping.device_slot;",
        breaks: "device-window teardown releases its reusable ledger slot",
        reddens: Caught::Check("device window reuses arbitrary-order gaps"),
    },
    // User roots copy top-level entries only once, so every future shared
    // device mapping needs its root slot before the first activation.
    Mutation {
        id: 326,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    space\n        .reserve_shared_kernel_topology()\n        .map_err(ActivateError::SharedTopology)?;",
        replace: "    if false {\n        space\n            .reserve_shared_kernel_topology()\n            .map_err(ActivateError::SharedTopology)?;\n    }",
        breaks: "late device mappings share top-level topology with every user root",
        reddens: Caught::Check("late device mappings reach earlier user roots"),
    },
    // Once roots have copied the higher half, a new active-root PML4 entry
    // would exist in only one of them.
    Mutation {
        id: 327,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if level == Level::Pml4\n            && !is_user_address(virtual_address)\n            && is_active()\n            && root == TableRoot::Full\n            && self.root() == active_root()\n        {",
        replace: "        if false\n            && level == Level::Pml4\n            && !is_user_address(virtual_address)\n            && is_active()\n            && root == TableRoot::Full\n            && self.root() == active_root()\n        {",
        breaks: "the active kernel cannot add an unshared higher-half root entry",
        reddens: Caught::Check("active kernel topology refuses a new shared root"),
    },
    Mutation {
        id: 357,
        file: "wost/src/arch/x86_64/port.rs",
        find: "pub struct PortSpan {\n    base: u16,\n    len: usize,\n}",
        replace: "pub struct PortSpan {\n    base: u16,\n    len: usize,\n}\n\nimpl PortSpan {\n    pub const fn base(&self) -> u16 {\n        self.base\n    }\n}",
        breaks: "the dynamic port ownership lease regains an uncalled register API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        id: 416,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "this processor has no x2apic mode, and xapic needs page tables",
        replace: "this processor has no x2apic mode",
        breaks: "the no-x2APIC attribution grammar drifts from the emitted warning",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 451,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if current_mapping_root(roots).is_none() {",
        replace: "        if false && current_mapping_root(roots).is_none() {",
        breaks: "a recycled inactive root can mint hardware stack authority",
        reddens: Caught::Check("the double-fault stack cannot be unmapped"),
    },
    Mutation {
        id: 452,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pinned.iter().flatten().find_map(|identity| {",
        replace: "    pinned.iter().skip(1).flatten().find_map(|identity| {",
        breaks: "the first permanent stack can lose its mapping protection",
        reddens: Caught::Check("the double-fault stack cannot be unmapped"),
    },
    Mutation {
        id: 453,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pinned.iter().flatten().find_map(|identity| {",
        replace: "    pinned.iter().take(1).flatten().find_map(|identity| {",
        breaks: "a later permanent stack can lose its mapping protection",
        reddens: Caught::Check("the boot thread stands on a guarded stack"),
    },
    Mutation {
        id: 455,
        file: "wost/src/mmio.rs",
        find: "    pub(crate) const unsafe fn new(base: *mut u8, len: usize) -> Self {",
        replace: "    pub(crate) const fn new(base: *mut u8, len: usize) -> Self {",
        breaks: "the raw MMIO constructor loses its caller safety contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        id: 456,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "    unsafe { core::arch::asm!(\"pushfq\", \"pop {}\", out(reg) flags, options(preserves_flags)) };",
        replace: "    unsafe { core::arch::asm!(\"pushfq\", \"pop {}\", out(reg) flags, options(nomem, preserves_flags)) };",
        breaks: "the flags probe claims no memory access while using a stack slot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        id: 457,
        file: "wost/src/arch/x86_64/segment.rs",
        find: "            \"ltr {selector:x}\",\n            selector = in(reg) TASK_STATE.bits(),\n            options(nostack, preserves_flags),",
        replace: "            \"ltr {selector:x}\",\n            selector = in(reg) TASK_STATE.bits(),\n            options(nomem, nostack, preserves_flags),",
        breaks: "the task-register load hides its GDT descriptor update from the compiler",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        id: 458,
        file: "wost/src/arch/x86_64/segment.rs",
        find: "            \"mov ss, {selector:x}\",\n            \"mov ds, {selector:x}\",\n            \"mov es, {selector:x}\",\n            selector = in(reg) KERNEL_DATA.bits(),\n            options(nostack, preserves_flags),",
        replace: "            \"mov ss, {selector:x}\",\n            \"mov ds, {selector:x}\",\n            \"mov es, {selector:x}\",\n            selector = in(reg) KERNEL_DATA.bits(),\n            options(nomem, nostack, preserves_flags),",
        breaks: "segment reload hides its GDT descriptor reads from the compiler",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        id: 485,
        file: "wost/src/arch/x86_64/shared_memory.rs",
        find: "        asm!(\n            \"mov dword ptr [{address}], {value:e}\",\n            address = in(reg) address,\n            value = in(reg) value,\n            options(nostack, preserves_flags),\n        );",
        replace: "        asm!(\n            \"mov dword ptr [{address}], {value:e}\",\n            address = in(reg) address,\n            value = in(reg) value,\n            options(nostack, preserves_flags, nomem),\n        );",
        breaks: "a published queue index can become visible before its payload",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::release_publication_excludes_an_old_payload",
        },
    },
    Mutation {
        id: 486,
        file: "wost/src/arch/x86_64/shared_memory.rs",
        find: "        asm!(\n            \"mov {value:e}, dword ptr [{address}]\",\n            address = in(reg) address,\n            value = out(reg) value,\n            options(nostack, preserves_flags),\n        );",
        replace: "        asm!(\n            \"mov {value:e}, dword ptr [{address}]\",\n            address = in(reg) address,\n            value = out(reg) value,\n            options(nostack, preserves_flags, nomem),\n        );",
        breaks: "a shared payload read can move before its published index",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::acquire_consumption_excludes_an_early_payload_read",
        },
    },
    Mutation {
        id: 487,
        file: "wost/src/mm/mod.rs",
        find: "        let value = unsafe { shared_memory::load_acquire_u32(address) };",
        replace: "        let value = unsafe { (address as *const u32).read_volatile() };",
        breaks: "a hostile ring-3 writer races a non-atomic volatile publication load",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 488,
        file: "wost/src/mm/mod.rs",
        find: "        unsafe { shared_memory::store_release_u32(address, value.to_le()) };",
        replace: "        unsafe { (address as *mut u32).write_volatile(value.to_le()) };",
        breaks: "a hostile ring-3 writer races a non-atomic volatile publication store",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 489,
        file: "wost/src/mm/mod.rs",
        find: "        unsafe { destination.write_volatile(byte) };",
        replace: "        unsafe { destination.write(byte) };",
        breaks: "the compiler can merge or elide shared payload writes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::every_shared_payload_copy_is_volatile",
        },
    },
    Mutation {
        id: 493,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub struct InterruptState {
    owner: usize,
    level: usize,
    live: bool,
    stays_here: PhantomData<*mut ()>,
}",
        replace: "pub struct InterruptState {
    owner: usize,
    level: usize,
    live: bool,
    stays_here: PhantomData<()>,
}",
        breaks: "a spin-lock guard cannot restore another CPU's interrupt flag",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::guard_types_do_not_cross_cpus",
        },
    },
    Mutation {
        id: 494,
        file: "wost/src/sync.rs",
        find: "pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
    stays_here: PhantomData<*const ()>,
}",
        replace: "pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
    stays_here: PhantomData<()>,
}",
        breaks: "a thread-lock guard cannot be released on another CPU",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::guard_types_do_not_cross_cpus",
        },
    },
    Mutation {
        id: 495,
        file: "wost/src/sync.rs",
        find: "impl<T> Drop for SpinLockGuard<'_, T> {",
        replace: "#[cfg(any())]\nimpl<T> Drop for SpinLockGuard<'_, T> {",
        breaks: "a live spin guard keeps its thread-context borrow until Drop",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::a_live_spin_guard_keeps_the_thread_context_borrowed",
        },
    },
    Mutation {
        id: 520,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::X87 as u32);",
        breaks: "x87 state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        id: 521,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::SSE as u32);",
        breaks: "SSE state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        id: 522,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::AVX as u32);",
        breaks: "AVX state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        id: 564,
        file: "wost/src/boot/limine_glue.rs",
        find: "    let region = unsafe { MmioRegion::new(fb.addr(), layout.span()) };",
        replace: "    let region = unsafe { MmioRegion::new(fb.addr(), layout.span() - 4) };",
        breaks: "a validated framebuffer lease omits its terminal word",
        reddens: Caught::Panic {
            marker: "validated framebuffer access left its mapping",
            why: "the framebuffer is the boot console and a contradictory validated MMIO lease must stop before its shadow can claim an unwritten device word",
        },
    },
    Mutation {
        id: 571,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    assert!(index < ENTRIES, \"a level's index is always below 512\");",
        replace: "    debug_assert!(index < ENTRIES, \"a level's index is always below 512\");",
        breaks: "release page-table slot formation loses its index bound",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 572,
        file: "wost/src/arch/x86_64/segment.rs",
        find: "fn write_stack(at: usize, top: u64) {\n    assert!(",
        replace: "fn write_stack(at: usize, top: u64) {\n    debug_assert!(",
        breaks: "release TSS stack writes lose their alignment precondition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 593,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "                self.undo(&range, offset, grain);\n                self.rollback_tables(table_mark);",
        replace: "                self.undo(&range, offset, grain);",
        breaks: "a failed mapping leaves its newly linked empty tables allocated",
        reddens: Caught::Check("a partial mapping failure returns its tables"),
    },
    Mutation {
        id: 596,
        file: "wost/src/mm/heap.rs",
        find: "        let Some(live) = self.live.checked_sub(1) else {",
        replace: "        let Some(live) = self.live.checked_sub(usize::MAX) else {",
        breaks: "a live slab slot is silently refused at deallocation",
        reddens: Caught::Panic {
            marker: "heap refused a live slot return",
            why: "valid GlobalAlloc provenance makes a slot-ledger refusal an internal contradiction, so deallocation must stop before ownership is forgotten",
        },
    },
    Mutation {
        id: 597,
        file: "wost/src/mm/heap.rs",
        find: "                        heap.large_frames.checked_sub(count),",
        replace: "                        heap.large_frames.checked_sub(usize::MAX),",
        breaks: "a live large allocation is silently refused by frame accounting",
        reddens: Caught::Panic {
            marker: "heap refused a live frame return",
            why: "valid GlobalAlloc provenance proves the large-frame ledger contains this run, so refusal is terminal before the segment can be forgotten",
        },
    },
    Mutation {
        id: 598,
        file: "wost/src/mm/heap.rs",
        find: "                let Some(physical) = physical_of(pointer) else {",
        replace: "                let Some(physical) = Option::<Paddr>::None else {",
        breaks: "a live direct-map allocation loses its physical provenance",
        reddens: Caught::Panic {
            marker: "heap could not recover a live allocation's physical address",
            why: "GlobalAlloc deallocation receives a pointer previously returned by this heap, so losing its direct-map provenance cannot be recovered as a leak",
        },
    },
    Mutation {
        id: 599,
        file: "wost/src/boot/mod.rs",
        find: "        let words = self.words();",
        replace: "        let words = self.words() - 1;",
        breaks: "a nonempty framebuffer shadow omits its terminal device word",
        reddens: Caught::Check("the console has a complete shadow to scroll from"),
    },
    Mutation {
        id: 603,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            let Some((candidate_physical, flags)) = translate_in(candidate, address) else {\n                return Some(address);\n            };",
        replace: "            let Some((candidate_physical, flags)) = translate_in(candidate, address) else {\n                return Some(address);\n            };\n            let flags = flags.union(Flags::USER);",
        breaks: "activation accepts a user-accessible kernel-image leaf",
        reddens: Caught::Check("the loaded page table is ours"),
    },
    Mutation {
        id: 604,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            match walk(root, virtual_address) {",
        replace: "            match walk(root, virtual_address).map(|(mapped, flags, span)| (mapped, flags.union(Flags::NO_CACHE), span)) {",
        breaks: "activation accepts cache-policy drift in the direct map",
        reddens: Caught::Check("the loaded page table is ours"),
    },
    Mutation {
        id: 608,
        file: "wost/src/mm/mod.rs",
        find: "    #[must_use = \"a refused shared-memory write changed no bytes\"]\n    pub fn write_volatile_at",
        replace: "    pub fn write_volatile_at",
        breaks: "a shared-memory operation refusal can become silently discardable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::fallible_operation_bools_stay_must_use",
        },
    },
    Mutation {
        id: 610,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "        let leaf = cpuid(0x4000_0000, 0);",
        replace: "        let leaf = cpuid(0x4000_0001, 0);",
        breaks: "the running CPU's exact QEMU TCG identity is not consumed",
        reddens: Caught::Panic {
            marker: "CPU state does not authenticate a qualified DMA-handoff platform",
            why: "the corrupted hypervisor leaf invalidates the authenticated DMA handoff before \
                  the self-test kernel can start",
        },
    },
    Mutation {
        id: 692,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        self.may_reach(virtual_base, len)?;\n        if let Some(pinned) = protected_stack_overlap(self.roots(), virtual_base, len) {",
        replace: "        self.may_reach(virtual_base, len)?;\n        if let Some(pinned) = permanent_stack_overlap(self.roots(), virtual_base, len) {",
        breaks: "unmap can clear a live task stack or its guard",
        reddens: Caught::Check("unmapping refuses what it does not own"),
    },
    Mutation {
        id: 693,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if current_slot == Some(slot) {",
        replace: "    if current_slot == Some(slot) && cpu::page_table_base() != active_root() {",
        breaks: "an active cleared leaf can remain cached until its frame is reused",
        reddens: Caught::Check("an active unmap exposes only its replacement"),
    },
    Mutation {
        id: 694,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if current_slot == Some(slot) {",
        replace: "    if cpu::page_table_base() == active_root() {",
        breaks: "root identity hides a shared higher-half leaf reached by the current CR3",
        reddens: Caught::Check("a shared kernel unmap evicts the live user root"),
    },
    Mutation {
        id: 695,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if current_slot == Some(slot) {",
        replace: "    if true {",
        breaks: "clearing an inactive lower-root leaf evicts an unrelated live translation",
        reddens: Caught::Check("an inactive lower-root unmap preserves the live observer"),
    },
    Mutation {
        id: 696,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "const NOT_THE_CALLERS: Flags = Flags::OWNED.union(Flags::HUGE).union(Flags::GLOBAL);",
        replace: "const NOT_THE_CALLERS: Flags = Flags::OWNED.union(Flags::HUGE);",
        breaks: "safe mappings can preserve a caller-supplied global translation",
        reddens: Caught::Check("caller global is removed from safe mappings"),
    },
    Mutation {
        id: 697,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "    assert!(\n        !page_table_context_ids_are_enabled(),\n        \"page-table switching requires CR4.PCIDE to remain clear\"\n    );",
        replace: "    assert!(\n        page_table_context_ids_are_enabled(),\n        \"page-table switching requires CR4.PCIDE to remain clear\"\n    );",
        breaks: "page-table switching accepts PCIDE and rejects the required clear state",
        reddens: Caught::Panic {
            marker: "page-table switching requires CR4.PCIDE to remain clear",
            why: "the CR3-flush contract requires PCID to remain disabled before any address-space switch",
        },
    },
    Mutation {
        id: 698,
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if let Some(address) = arena.invalid_page() {",
        replace: "    if let Some(address) = arena.invalid_page().filter(|_| false) {",
        breaks: "task arena validation ignores an unmapped stack or guard",
        reddens: Caught::Check("thread stacks are guarded below"),
    },
    Mutation {
        id: 699,
        file: "wost/src/arch/x86_64/task.rs",
        find: "    let layout = match validate_arena(&arena) {",
        replace: "    let layout = match Ok::<stack_layout::StackLayout, TaskError>(*arena.layout()) {",
        breaks: "task initialization can publish and pin an arena it never validated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::task_init_validates_the_arena_before_publication_and_pinning",
        },
    },
    Mutation {
        id: 713,
        file: "wost/src/mm/mod.rs",
        find: "impl Drop for USegment {\n    fn drop(&mut self) {\n        let Some(mut segment) = self.segment.take() else {\n            return;\n        };\n        if !segment.zero() {",
        replace: "impl Drop for USegment {\n    fn drop(&mut self) {\n        let Some(mut segment) = self.segment.take() else {\n            return;\n        };\n        if false {",
        breaks: "a failed external-byte zeroing attempt still returns the page to the allocator",
        reddens: Caught::Check("external user bytes are zeroed before free"),
    },
    Mutation {
        id: 714,
        file: "wost/src/mm/mod.rs",
        find: "impl Drop for USegment {",
        replace: "#[cfg(any())]\nimpl Drop for USegment {",
        breaks: "natural user-segment drop returns external bytes without zeroing them",
        reddens: Caught::Check("external user bytes are zeroed before free"),
    },
    Mutation {
        id: 715,
        file: "wost/src/mm/mod.rs",
        find: "        if !segment.zero() {\n            #[cfg(feature = \"selftest\")]\n            let address = segment.address();\n            core::mem::forget(segment);\n            #[cfg(feature = \"selftest\")]\n            observe_external_run_withheld_for_test(address);\n        }",
        replace: "        if !segment.zero() {\n            #[cfg(feature = \"selftest\")]\n            let address = segment.address();\n            drop(segment);\n            #[cfg(feature = \"selftest\")]\n            observe_external_run_withheld_for_test(address);\n        }",
        breaks: "a refused external clear returns a dirty run after deciding to withhold it",
        reddens: Caught::Check("a refused external clear withholds its run"),
    },
    Mutation {
        id: 718,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    fn device_reclaim_error(&self) -> Option<ReclaimError> {\n        let mappings = self.devices.iter().flatten().count();\n        (mappings != 0).then_some(ReclaimError::Devices(mappings))\n    }",
        replace: "    fn device_reclaim_error(&self) -> Option<ReclaimError> {\n        let _mappings = self.devices.iter().flatten().count();\n        None\n    }",
        breaks: "a live device ledger no longer prevents its page tables from being reclaimed",
        reddens: Caught::Check("active device tables cannot be reclaimed"),
    },
    Mutation {
        id: 719,
        file: "wost/src/mm/mod.rs",
        find: "    for run in map.allocatable_runs() {\n        ignored += frames.insert(run.base(), run.bytes()).ignored;\n    }",
        replace: "    for region in map.regions().filter(|region| region.kind.is_allocatable()) {\n        ignored += frames.insert(region.base, region.len).ignored;\n    }",
        breaks: "raw usable claims seed frames that an overlapping reserved claim excludes",
        reddens: Caught::Check("the frame allocator admits only exclusively usable frames"),
    },
    Mutation {
        id: 720,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            Err(reason) => Err(UserMapError::Mapping { reason, run }),",
        replace: "            Err(_reason) => {\n                run.commit_mapping();\n                Ok(())\n            }",
        breaks: "a refused user mapping consumes dirty-page ownership as if the mapping committed",
        reddens: Caught::Check("external user bytes are zeroed before free"),
    },
    Mutation {
        id: 722,
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if !disabled.were_enabled() {",
        replace: "    if false {",
        breaks: "a user return frame can enable interrupts that were not admitted before transition",
        reddens: Caught::Check("unadmitted interrupts cannot be enabled by a user transition"),
    },
    Mutation {
        id: 724,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn fault_window() -> (u64, u64) {",
        replace: "pub fn fault_window() -> (u64, u64) {",
        breaks: "the user-copy fault window reaches the shipped wost graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 736,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    if EOI_CLAIMED\n        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)\n        .is_err()\n    {",
        replace: "    if EOI_CLAIMED\n        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)\n        .is_err()\n        && false\n    {",
        breaks: "one APIC delivery can mint more than one completion authority",
        reddens: Caught::Check("one delivery mints one completion authority"),
    },
    Mutation {
        id: 737,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "#[derive(Debug)]\npub struct DisarmedTimer",
        replace: "#[derive(Debug, Clone, Copy)]\npub struct DisarmedTimer",
        breaks: "a timer-disarm proof can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_completion_and_disarm_authorities_are_linear",
        },
    },
    Mutation {
        id: 771,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "            Lifecycle::Cold.bits(),\n            Lifecycle::Initializing.bits(),\n            Ordering::AcqRel,",
        replace: "            Lifecycle::Cold.bits(),\n            Lifecycle::Ready.bits(),\n            Ordering::AcqRel,",
        breaks: "the live APIC publishes readiness before its hardware transaction begins",
        reddens: Caught::Check(
            "local apic initialization stayed unpublished and interrupt-excluded",
        ),
    },
    Mutation {
        id: 774,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "impl Drop for EoiToken<'_> {\n    fn drop(&mut self) {\n        EOI_CLAIMED.store(false, Ordering::Release);",
        replace: "impl Drop for EoiToken<'_> {\n    fn drop(&mut self) {\n        EOI_CLAIMED.store(true, Ordering::Release);",
        breaks: "dropping an EOI token leaves its completion latch claimed",
        reddens: Caught::Panic {
            marker: "vector 0x41 has no matching APIC in-service authority",
            why: "the leaked EOI claim prevents the next ping from owning its in-service source",
        },
    },
    Mutation {
        id: 780,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let interrupts = Some(interrupt::disable());",
        replace: "        let interrupts = None;",
        breaks: "paging activation publishes across an interruptible partial transaction",
        reddens: Caught::Check("paging activation stays unpublished under interrupt exclusion"),
    },
    Mutation {
        id: 781,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            Ok(_) => {\n                let activation = Self {",
        replace: "            Ok(_) => {\n                state.store(ActivationState::Ready.bits(), Ordering::Release);\n                let activation = Self {",
        breaks: "paging activation publishes readiness before its CR3 transaction commits",
        reddens: Caught::Check("paging activation stays unpublished under interrupt exclusion"),
    },
    Mutation {
        id: 782,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        this.root.store(root, Ordering::Release);\n        after_root();\n        let Some(ready) = ActivationState::Initializing.complete() else {\n            panic!(\"paging initialization cannot complete\");\n        };\n        this.committed = true;\n        this.state.store(ready.bits(), Ordering::Release);",
        replace: "        let Some(ready) = ActivationState::Initializing.complete() else {\n            panic!(\"paging initialization cannot complete\");\n        };\n        this.state.store(ready.bits(), Ordering::Release);\n        this.root.store(root, Ordering::Release);\n        after_root();\n        this.committed = true;",
        breaks: "paging activation publishes readiness before its active root",
        reddens: Caught::Check("paging activation publishes its root before readiness"),
    },
    Mutation {
        id: 783,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    unsafe { cpu::set_page_table_base(root) };\n    activation.commit(root);",
        replace: "    activation.commit(root);\n    unsafe { cpu::set_page_table_base(root) };",
        breaks: "paging activation publishes readiness before loading its page-table root",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::paging_activation_loads_cr3_before_publishing_ready",
        },
    },
    Mutation {
        id: 784,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if space.is_reclaimed() {",
        replace: "    if false && space.is_reclaimed() {",
        breaks: "paging activation reads a root from an already reclaimed address space",
        reddens: Caught::Panic {
            marker: "a reclaimed address space has no root",
            why: "activation reads the removed root before the first self-test verdict",
        },
    },
    Mutation {
        id: 785,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub fn invalidate_page(address: CanonicalAddress) {\n    // SAFETY: construction of `CanonicalAddress` proves the instruction's\n    // operand is canonical. `invlpg` dereferences no memory and only evicts\n    // this CPU's cached translation.\n    unsafe {\n        core::arch::asm!(\"invlpg [{}]\", in(reg) address.get(), options(nostack, preserves_flags))\n    };\n    #[cfg(feature = \"selftest\")]\n    PAGE_INVALIDATIONS.fetch_add(1, Ordering::Relaxed);\n}",
        replace: "pub trait InvalidationAddress {\n    /// Expose the instruction operand.\n    fn raw(self) -> u64;\n}\n\nimpl InvalidationAddress for CanonicalAddress {\n    fn raw(self) -> u64 {\n        self.get()\n    }\n}\n\nimpl InvalidationAddress for u64 {\n    fn raw(self) -> u64 {\n        self\n    }\n}\n\n/// Drop any cached translation for an admitted address from this CPU's TLB.\npub fn invalidate_page(address: impl InvalidationAddress) {\n    let raw = address.raw();\n    // SAFETY: the weakened mutation admits this operand without validation.\n    unsafe {\n        core::arch::asm!(\"invlpg [{}]\", in(reg) raw, options(nostack, preserves_flags))\n    };\n    #[cfg(feature = \"selftest\")]\n    PAGE_INVALIDATIONS.fetch_add(1, Ordering::Relaxed);\n}",
        breaks: "the INVLPG door accepts a raw noncanonical address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::invlpg_requires_canonical_address_authority",
        },
    },
    Mutation {
        id: 793,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "    let NvmeBusMasterAuthority { bar, _observed: _ } = authority;\n    if interrupts.function() != domain.function() {\n        return Err(EnableError::Stale);\n    }\n    let mut configuration = CONFIGURATION.lock_leaf();",
        replace: "    let NvmeBusMasterAuthority { bar, _observed: _ } = authority;\n    if interrupts.function() != domain.function() {\n        return Err(EnableError::Stale);\n    }\n    let mut configuration = CONFIGURATION\n        .try_lock_leaf()\n        .expect(\"the configuration port pair is available\");",
        breaks: "one PCI command transaction does not hold the configuration port pair exactly once",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_command_transactions_hold_the_port_pair_once",
        },
    },
    Mutation {
        id: 817,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let Some(current) = current_mapping_root(self.roots) else {\n            return false;\n        };",
        replace: "        if !is_active() || active_root() != self.roots.full_root() {\n            return false;\n        }\n        let current = self.roots.full_root();",
        breaks: "a device token authenticates its recorded root instead of the root actually loaded",
        reddens: Caught::Check("a stale shared root cannot authenticate a device mapping"),
    },
    Mutation {
        id: 818,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        current_mapping_root(self.identity.roots)\n            .is_some_and(|current| self.identity.invalid_page_in(current).is_none())",
        replace: "        is_active()\n            && active_root() == self.identity.roots.full_root()\n            && self\n                .identity\n                .invalid_page_in(self.identity.roots.full_root())\n                .is_none()",
        breaks: "a stack token authenticates its recorded root instead of the root actually loaded",
        reddens: Caught::Check("a stale shared root cannot authenticate a stack"),
    },
    Mutation {
        id: 819,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let mut stack = self.slot.stack_base();",
        replace: "        let mut stack = self.slot.stack_base() + PAGE_SIZE;",
        breaks: "a retained stack identity skips its first leaf",
        reddens: Caught::Check("the boot stack rejects a lower-leaf substitution"),
    },
    Mutation {
        id: 820,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        let identity = self.stacks.get(index)?;\n        if !is_active() {\n            return None;\n        }\n        let current = cpu::page_table_base();\n        identity\n            .invalid_page_in(current)\n            .is_none()\n            .then_some(KernelStackTop(identity))\n    }",
        replace: "    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        self.stacks.get(index).map(KernelStackTop)\n    }",
        breaks: "a hardware-use stack pointer is derived without rewalking its retained identity",
        reddens: Caught::Check("a noncurrent task stack rejects a lower-leaf substitution"),
    },
    Mutation {
        id: 821,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            match walk(root, stack) {\n                Some((found, flags, span))\n                    if page_identity_matches(found, flags, span, physical, expected_flags) => {}\n                _ => return Some(stack),\n            }",
        replace: "            if offset != PAGE_SIZE {\n                match walk(root, stack) {\n                    Some((found, flags, span))\n                        if page_identity_matches(found, flags, span, physical, expected_flags) => {}\n                    _ => return Some(stack),\n                }\n            }",
        breaks: "a retained stack identity skips its middle leaf",
        reddens: Caught::Check("a stack checks its middle leaf"),
    },
    Mutation {
        id: 822,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        while stack < self.slot.stack_top() {",
        replace: "        while stack < self.slot.stack_top() - PAGE_SIZE {",
        breaks: "a retained stack identity skips its last leaf",
        reddens: Caught::Check("a stack checks its last leaf"),
    },
    Mutation {
        id: 823,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let mut offset = 0u64;",
        replace: "        let mut offset = PAGE_SIZE;",
        breaks: "a retained device identity skips its first leaf",
        reddens: Caught::Check("a device mapping checks its first leaf"),
    },
    Mutation {
        id: 824,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        replace: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            let expected = if offset == PAGE_SIZE {\n                mapped\n            } else {\n                expected\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        breaks: "a retained device identity trusts its middle physical leaf",
        reddens: Caught::Check("a device mapping checks its middle leaf"),
    },
    Mutation {
        id: 825,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        replace: "            let Some((mapped, flags, span)) = walk(current, virtual_address) else {\n                return false;\n            };\n            let expected = if offset.checked_add(PAGE_SIZE) == Some(self.pages) {\n                mapped\n            } else {\n                expected\n            };\n            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        breaks: "a retained device identity trusts its last physical leaf",
        reddens: Caught::Check("a device mapping checks its last leaf"),
    },
    Mutation {
        id: 826,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if !page_identity_matches(mapped, flags, span, expected, self.expected_flags) {",
        replace: "            if !page_identity_matches(mapped, flags, span, expected, flags) {",
        breaks: "a retained device identity trusts the cache policy it observes",
        reddens: Caught::Check("a device mapping checks its cache policy"),
    },
    Mutation {
        id: 828,
        file: "wost/src/arch/x86_64/task.rs",
        find: "    unsafe { extended.write_bytes(0, xstate::CEILING) };",
        replace: "    unsafe { extended.write_bytes(0, 0) };",
        breaks: "a recycled task slot retains its previous extended-state bytes",
        reddens: Caught::Check("an exited slot is reused"),
    },
    Mutation {
        id: 834,
        file: "wost/src/mm/mod.rs",
        find: "impl Shared {\n    /// Consume the allocation into volatile byte-only access.",
        replace: "impl Shared {\n    /// Write through the unpartitioned shared owner.\n    #[must_use]\n    pub fn write_volatile_at(&mut self, offset: usize, bytes: &[u8]) -> bool {\n        write_shared_volatile(&self.0, offset, bytes)\n    }\n\n    /// Consume the allocation into volatile byte-only access.",
        breaks: "an unpartitioned shared allocation retains a raw byte-write door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        id: 835,
        file: "wost/src/mm/mod.rs",
        find: "        if !self.layout.contains_payload(offset, bytes.len()) {",
        replace: "        if false && !self.layout.contains_payload(offset, bytes.len()) {",
        breaks: "a shared ring payload write can enter its publication-word partition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::every_shared_payload_copy_is_volatile",
        },
    },
    Mutation {
        id: 847,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    fn published_vector(self, source: &AtomicU8) -> Option<InterruptVector> {\n        if !self.is_enabled() {\n            return None;\n        }\n        InterruptVector::new(source.load(Ordering::Relaxed))",
        replace: "    fn published_vector(self, source: &AtomicU8) -> Option<InterruptVector> {\n        let vector = source.load(Ordering::Relaxed);\n        if !self.is_enabled() {\n            return None;\n        }\n        InterruptVector::new(vector)",
        breaks: "an APIC consumer reads a dependent vector before acquiring Ready",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_ready_publication_precedes_dependent_vector_reads",
        },
    },
    Mutation {
        id: 848,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "        let Some(vector) = self.published_vector(self.self_interrupt_vector) else {\n            return Err(DeliveryError::Unavailable);\n        };",
        replace: "        let vector = self.self_interrupt_vector.load(Ordering::Relaxed);",
        breaks: "self-interrupt delivery reads its vector outside ordered Ready publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_ready_publication_precedes_dependent_vector_reads",
        },
    },
    Mutation {
        id: 849,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "        self.controller.state.store(ready.bits(), Ordering::Release);\n        if let Some(interrupts) = self.interrupts.take() {\n            interrupts.restore();\n        }",
        replace: "        if let Some(interrupts) = self.interrupts.take() {\n            interrupts.restore();\n        }\n        self.controller.state.store(ready.bits(), Ordering::Release);",
        breaks: "APIC readiness publishes after initialization interrupt exclusion unwinds",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_ready_publication_precedes_dependent_vector_reads",
        },
    },
    Mutation {
        id: 850,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "            self.controller\n                .state\n                .store(self.transaction.refuse().bits(), Ordering::Release);",
        replace: "            self.controller\n                .state\n                .store(Lifecycle::Cold.bits(), Ordering::Release);",
        breaks: "a failed live APIC transaction becomes retryable after its owner drops",
        reddens: Caught::Check(
            "every late or ignored-write apic refusal stays unavailable and terminal",
        ),
    },
    Mutation {
        id: 851,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    InitializationRegisters::write(\n        &registers,\n        register::SPURIOUS_INTERRUPT_VECTOR,\n        spurious.bits().into(),\n    );\n    if InitializationRegisters::read(&registers, register::SPURIOUS_INTERRUPT_VECTOR)\n        != u64::from(spurious.bits())\n    {\n        return Err(ApicError::NotResponding);\n    }",
        replace: "    InitializationRegisters::write(\n        &registers,\n        register::SPURIOUS_INTERRUPT_VECTOR,\n        spurious.bits().into(),\n    );",
        breaks: "an ignored APIC software-enable write can publish readiness",
        reddens: Caught::Check(
            "every late or ignored-write apic refusal stays unavailable and terminal",
        ),
    },
    Mutation {
        id: 852,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "        self.lifecycle().is_ready().then_some(Registers)",
        replace: "        matches!(self.lifecycle(), Lifecycle::Initializing | Lifecycle::Ready)\n            .then_some(Registers)",
        breaks: "the central APIC register gate admits an unpublished initializing controller",
        reddens: Caught::Check(
            "every late or ignored-write apic refusal stays unavailable and terminal",
        ),
    },
    Mutation {
        id: 853,
        file: "wost/src/mm/mod.rs",
        find: "        let Ok(range) = AccessRange::repeated(offset, bytes.len(), 1, self.len) else {",
        replace: "        let Ok(range) =\n            AccessRange::repeated(offset, bytes.len(), 1, self.len.saturating_sub(1))\n        else {",
        breaks: "DMA byte writes refuse the exact final byte of their owned run",
        reddens: Caught::Check("dma byte access is exact and bounded"),
    },
    Mutation {
        id: 860,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "#[cfg(feature = \"selftest\")]\n#[must_use = \"a refused resume changed no trap frame\"]\npub(super) fn resume_write_protection(",
        replace: "#[cfg(feature = \"selftest\")]\n#[must_use = \"a refused write-protection resume changed no trap frame\"]\npub(super) fn resume_write_protection(",
        breaks: "the write-protection recovery door loses its pinned must-use contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::fallible_operation_bools_stay_must_use",
        },
    },
    Mutation {
        id: 869,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "        frame_error_code: frame.error_code(),",
        replace: "        frame_error_code: frame.vector(),",
        breaks: "the user-copy adapter substitutes the trap vector for the saved error word",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        id: 870,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    let at = frame.instruction_pointer();",
        replace: "    let mut at = frame.instruction_pointer();",
        breaks: "the user-copy adapter can rewrite its saved live instruction pointer",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        id: 871,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;",
        replace: "    let start = core::ptr::addr_of!(wost_copy_fault_end) as u64;",
        breaks: "the user-copy recovery window starts at its exclusive end label",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        id: 872,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "        faulting_address: faulting_address(),",
        replace: "        faulting_address: target,",
        breaks: "the write-protection adapter substitutes its expected target for live CR2",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        id: 873,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "target.checked_add(1)",
        replace: "target.wrapping_add(1)",
        breaks: "the write-protection recovery target end can wrap through zero",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::each_recovery_adapter_binds_the_live_architectural_values",
        },
    },
    Mutation {
        id: 874,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    // SAFETY: as above. Only `usercopy` can construct the target, from its\n    // in-crate fixup label, so `iretq` cannot be redirected by another caller.\n    unsafe { core::ptr::write(slot, target.address()) };",
        replace: "    // SAFETY: this mutation writes a value already read from the live frame.\n    unsafe { core::ptr::write(slot, frame.instruction_pointer()) };",
        breaks: "the user-copy resume writer ignores its opaque fixup authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_requires_its_opaque_authority",
        },
    },
    Mutation {
        id: 875,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    LIVE_FRAME.load(Ordering::Relaxed) == frame.origin",
        replace: "    LIVE_FRAME.load(Ordering::Relaxed) != frame.origin",
        breaks: "live-frame authority authenticates every origin except the active one",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
        },
    },
    Mutation {
        id: 876,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub(super) fn resume_usercopy(frame: &TrapFrame, target: super::usercopy::RecoveryTarget) -> bool {\n    if !is_live_frame(frame) {\n        return false;\n    }",
        replace: "pub(super) fn resume_usercopy(frame: &TrapFrame, target: super::usercopy::RecoveryTarget) -> bool {\n    if false && !is_live_frame(frame) {\n        return false;\n    }",
        breaks: "the user-copy resume writer can reach a stale trap frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
        },
    },
    Mutation {
        id: 877,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub(super) fn resume_write_protection(\n    frame: &TrapFrame,\n    target: super::cpu::WriteProtectionRecoveryTarget,\n) -> bool {\n    if !is_live_frame(frame) {\n        return false;\n    }",
        replace: "pub(super) fn resume_write_protection(\n    frame: &TrapFrame,\n    target: super::cpu::WriteProtectionRecoveryTarget,\n) -> bool {\n    if false && !is_live_frame(frame) {\n        return false;\n    }",
        breaks: "the write-protection resume writer can reach a stale trap frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
        },
    },
    Mutation {
        id: 878,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);",
        replace: "pub(super) struct RecoveryTarget(pub(super) u64);",
        breaks: "the user-copy fixup address field becomes constructible across its parent module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 879,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);",
        replace: "#[derive(Clone)]\npub(super) struct RecoveryTarget(u64);",
        breaks: "user-copy recovery authority gains derived duplication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 880,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "impl RecoveryTarget {\n    fn fixup() -> Self {",
        replace: "impl RecoveryTarget {\n    const ZERO: Self = Self(0);\n\n    fn fixup() -> Self {",
        breaks: "user-copy recovery authority gains an associated raw-address constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 881,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);\n\nimpl RecoveryTarget {",
        replace: "pub(super) struct RecoveryTarget(u64);\n\nconst FORGED_RECOVERY_TARGET: RecoveryTarget = unsafe { core::mem::transmute(0u64) };\n\nimpl RecoveryTarget {",
        breaks: "user-copy recovery authority gains a module-level typed forgery surface",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 882,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "pub(super) struct RecoveryTarget(u64);\n\nimpl RecoveryTarget {",
        replace: "pub(super) struct RecoveryTarget(u64);\n\nuse self::RecoveryTarget as ForgedRecoveryTarget;\n\nimpl RecoveryTarget {",
        breaks: "user-copy recovery authority gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 883,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub(super) struct WriteProtectionRecoveryTarget(u64);",
        replace: "pub(super) struct WriteProtectionRecoveryTarget(pub(super) u64);",
        breaks: "the write-protection fixup field becomes constructible across its parent module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 884,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)",
        replace: "        Self(core::ptr::addr_of!(wost_copy_fault_end) as u64)",
        breaks: "user-copy recovery authority is minted from the end of the fault window instead of the linker fixup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 885,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }",
        replace: "    pub(super) const fn address(&self) -> u64 {\n        0\n    }",
        breaks: "user-copy recovery authority observes a value other than its private address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 886,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}",
        replace: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}\n\nimpl self::RecoveryTarget {\n    fn from_raw(address: u64) -> Self {\n        Self(address)\n    }\n}",
        breaks: "a qualified inherent impl restores a raw user-copy recovery constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 887,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}",
        replace: "impl RecoveryTarget {\n    fn fixup() -> Self {\n        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)\n    }\n\n    pub(super) const fn address(&self) -> u64 {\n        self.0\n    }\n}\n\nmod hostile_nested_recovery_constructor {\n    fn from_raw(value: u64) -> super::RecoveryTarget {\n        super::RecoveryTarget(value)\n    }\n}",
        breaks: "a nested module restores a raw user-copy recovery constructor",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 888,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: r#"impl RecoveryTarget {
    fn fixup() -> Self {
        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)
    }

    pub(super) const fn address(&self) -> u64 {
        self.0
    }
}

/// Resume a faulting user copy at its fixup, if that is what faulted.
///
/// A mapping may disappear between the walk and copy. Nothing outside the
/// private two-instruction interval is recoverable. `false` leaves the frame
/// unchanged.
#[must_use = "a refused user-copy recovery changed no trap frame"]
pub fn recover(frame: &interrupt::TrapFrame, fault: trap_cause::PageFault) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;
    let end = core::ptr::addr_of!(wost_copy_fault_end) as u64;
    let attempt = trap_cause::RecoveryAttempt {
        vector: frame.vector(),
        frame_error_code: frame.error_code(),
        code_selector: frame.code_selector(),
        instruction_pointer: at,
        page_fault: fault,
        faulting_address: cpu::faulting_address(),
    };
    let rule = trap_cause::RecoveryRule {
        fault: trap_cause::RecoveryFault::SupervisorData,
        fault_addresses: 0..USER_END,
        instruction_addresses: start..end,
    };
    if !attempt.is_admitted_by(&rule) {
        return false;
    }
    interrupt::resume_usercopy(frame, RecoveryTarget::fixup())
}"#,
        replace: r#"impl RecoveryTarget {
    fn fixup() -> Self {
        Self(core::ptr::addr_of!(wost_copy_fixup) as u64)
    }

    pub(super) const fn address(&self) -> u64 {
        self.0
    }
}

fn hostile_resume(frame: &interrupt::TrapFrame) -> bool {
    interrupt::resume_usercopy(frame, RecoveryTarget::fixup())
}

/// Resume a faulting user copy at its fixup, if that is what faulted.
///
/// A mapping may disappear between the walk and copy. Nothing outside the
/// private two-instruction interval is recoverable. `false` leaves the frame
/// unchanged.
#[must_use = "a refused user-copy recovery changed no trap frame"]
pub fn recover(frame: &interrupt::TrapFrame, fault: trap_cause::PageFault) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_copy_fault_start) as u64;
    let end = core::ptr::addr_of!(wost_copy_fault_end) as u64;
    let attempt = trap_cause::RecoveryAttempt {
        vector: frame.vector(),
        frame_error_code: frame.error_code(),
        code_selector: frame.code_selector(),
        instruction_pointer: at,
        page_fault: fault,
        faulting_address: cpu::faulting_address(),
    };
    let rule = trap_cause::RecoveryRule {
        fault: trap_cause::RecoveryFault::SupervisorData,
        fault_addresses: 0..USER_END,
        instruction_addresses: start..end,
    };
    if !attempt.is_admitted_by(&rule) {
        return false;
    }
    hostile_resume(frame)
}"#,
        breaks: "recovery fixup authority escapes its validated adapter through a private helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "recovery_contracts::recovery_authority_cannot_be_built_from_a_raw_address",
        },
    },
    Mutation {
        id: 925,
        file: "wost/src/sync.rs",
        find: "    fn acquire(&self, #[cfg(feature = \"selftest\")] kind: LockKind) -> Option<SpinLockGuard<'_, T>> {",
        replace: "    fn acquire(&self, #[cfg(feature = \"selftest\")] kind: bool) -> Option<SpinLockGuard<'_, T>> {",
        breaks: "spin-lock discipline becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 926,
        file: "wost/src/arch/x86_64/percpu.rs",
        find: "        #[cfg(feature = \"selftest\")] kind: LockKind,\n        #[cfg(feature = \"selftest\")] at: &'static core::panic::Location<'static>,",
        replace: "        #[cfg(feature = \"selftest\")] kind: bool,\n        #[cfg(feature = \"selftest\")] at: &'static core::panic::Location<'static>,",
        breaks: "per-CPU lock acquisition accounting becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 927,
        file: "wost/src/arch/x86_64/percpu.rs",
        find: "    pub(crate) fn released_lock(&self, #[cfg(feature = \"selftest\")] kind: LockKind) {",
        replace: "    pub(crate) fn released_lock(&self, #[cfg(feature = \"selftest\")] kind: bool) {",
        breaks: "per-CPU lock release accounting becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 988,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    fn map_device_range(\n        &mut self,\n        admitted: &AdmittedDeviceRange,\n    ) -> Result<DeviceMapping, MapError> {",
        replace: "    fn map_device_range(\n        &mut self,\n        admitted: &AdmittedDeviceRange,\n    ) -> Result<DeviceMapping, MapError> {\n        let _fixed_projection = [None::<DeviceExtent>; MAX_DEVICE_MAPPINGS];",
        breaks: "device mapping rebuilds a maximum-sized ownership projection on the kernel stack",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::an_address_space_keeps_its_bounded_ledger_off_the_stack",
        },
    },
    Mutation {
        id: 989,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let virtual_extent =\n            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;\n\n        let expected_flags = Flags::PRESENT\n            .union(Flags::WRITABLE)\n            .union(Flags::NO_EXECUTE)\n            .union(Flags::NO_CACHE);\n        self.map_range(\n            virtual_base,\n            base,\n            pages,\n            expected_flags.difference(Flags::PRESENT),\n            Grain::Pages,\n        )?;",
        replace: "        let expected_flags = Flags::PRESENT\n            .union(Flags::WRITABLE)\n            .union(Flags::NO_EXECUTE)\n            .union(Flags::NO_CACHE);\n        self.map_range(\n            virtual_base,\n            base,\n            pages,\n            expected_flags.difference(Flags::PRESENT),\n            Grain::Pages,\n        )?;\n        let virtual_extent =\n            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;",
        breaks: "device mapping keeps fallible extent construction after page-table commit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::an_address_space_keeps_its_bounded_ledger_off_the_stack",
        },
    },
    Mutation {
        id: 990,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "pub(crate) struct TaskStackPin {\n    stacks: Vec<StackIdentity>,\n}\n\n/// A task-stack top borrowed from a pinned arena identity.",
        replace: "pub(crate) struct TaskStackPin {\n    stacks: Vec<StackIdentity>,\n}\n\nimpl Drop for TaskStackPin {\n    fn drop(&mut self) {\n        let mut pinned = TASK_STACK_PIN.lock_leaf();\n        *pinned = None;\n    }\n}\n\n/// A task-stack top borrowed from a pinned arena identity.",
        breaks: "dropping the task stack owner restores unmap authority over hardware-retained stacks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::a_kernel_stack_top_cannot_outlive_its_pin",
        },
    },
    Mutation {
        id: 991,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "pub(crate) struct KernelStackTop<'pin>(&'pin StackIdentity);\n\nimpl KernelStackTop<'_> {\n    /// The address hardware may retain in `RSP0`.\n    pub(crate) const fn address(self) -> u64 {\n        self.0.slot.stack_top()\n    }\n}\n\nimpl TaskStackPin {\n    /// The pinned stack at `index`, as retained-hardware authority.\n    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        let identity = self.stacks.get(index)?;\n        if !is_active() {\n            return None;\n        }\n        let current = cpu::page_table_base();\n        identity\n            .invalid_page_in(current)\n            .is_none()\n            .then_some(KernelStackTop(identity))\n    }",
        replace: "pub(crate) struct KernelStackTop<'pin> {\n    address: u64,\n    _pin: core::marker::PhantomData<&'pin TaskStackPin>,\n}\n\nimpl KernelStackTop<'_> {\n    /// The address hardware may retain in `RSP0`.\n    pub(crate) const fn address(self) -> u64 {\n        self.address\n    }\n}\n\nimpl TaskStackPin {\n    /// The pinned stack at `index`, as retained-hardware authority.\n    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {\n        let identity = self.stacks.get(index)?;\n        if !is_active() {\n            return None;\n        }\n        let current = cpu::page_table_base();\n        identity\n            .invalid_page_in(current)\n            .is_none()\n            .then_some(KernelStackTop {\n                address: identity.slot.stack_top(),\n                _pin: core::marker::PhantomData,\n            })\n    }",
        breaks: "kernel stack-top authority carries only an address and phantom pin instead of borrowing its identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::a_kernel_stack_top_cannot_outlive_its_pin",
        },
    },
    Mutation {
        id: 992,
        file: "wost/src/arch/x86_64/paging.rs",
        find: r#"            None => {
                self.devices
                    .try_reserve(1)
                    .map_err(|_| MapError::OutOfMemory)?;
                self.devices.len()
            }
        };

        let Some(device_window) =
            AddressRange::new(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)
        else {
            return Err(MapError::OutOfMemory);
        };
        let virtual_base = first_fit_by(device_window, pages, PAGE_SIZE, &self.devices, |extent| {
            extent.map(|extent| extent.virtual_range)
        })
        .ok_or(MapError::OutOfMemory)?;
        let virtual_extent =
            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::NO_CACHE);
        self.map_range(
            virtual_base,
            base,
            pages,
            expected_flags.difference(Flags::PRESENT),
            Grain::Pages,
        )?;"#,
        replace: r#"            None => self.devices.len(),
        };

        let Some(device_window) =
            AddressRange::new(DEVICE_WINDOW_START, DEVICE_WINDOW_END - DEVICE_WINDOW_START)
        else {
            return Err(MapError::OutOfMemory);
        };
        let virtual_base = first_fit_by(device_window, pages, PAGE_SIZE, &self.devices, |extent| {
            extent.map(|extent| extent.virtual_range)
        })
        .ok_or(MapError::OutOfMemory)?;
        let virtual_extent =
            AddressRange::new(virtual_base, pages).ok_or(MapError::InvalidDeviceRange(physical))?;

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::NO_CACHE);
        self.map_range(
            virtual_base,
            base,
            pages,
            expected_flags.difference(Flags::PRESENT),
            Grain::Pages,
        )?;
        if device_slot == self.devices.len() {
            self.devices
                .try_reserve(1)
                .map_err(|_| MapError::OutOfMemory)?;
        }"#,
        breaks: "device-ledger allocation can fail only after page-table mapping commits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::an_address_space_keeps_its_bounded_ledger_off_the_stack",
        },
    },
    Mutation {
        id: 1007,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "const FUNCTION_CLAIMS: usize = pci_config::FUNCTION_AUTHORITY_LIMIT;",
        replace: "const FUNCTION_CLAIMS: usize = pci_config::FUNCTION_AUTHORITY_LIMIT;\n\nfn write_command() {}",
        breaks: "the PCI adapter regrows an unchecked command-register write helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1008,
        file: "wost/src/mm/mod.rs",
        find: "pub const CEILING: u64 = 16 << 30;",
        replace: "pub const CEILING: u64 = 16 << 30;\n\nfn first_region() {}",
        breaks: "physical-memory admission regrows a speculative first-region shortcut",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1009,
        file: "wost/src/mm/mod.rs",
        find: "pub const CEILING: u64 = 16 << 30;",
        replace: "pub const CEILING: u64 = 16 << 30;\n\nfn past_every_region() {}",
        breaks: "physical-memory admission regrows a speculative region-tail shortcut",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1012,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "#[derive(Debug)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {\n    authority: FunctionAuthority,\n    identity: u32,\n}",
        replace: "#[derive(Debug)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {\n    authority: FunctionAuthority,\n    identity: u32,\n}\n\nimpl Clone for FunctionClaim {\n    fn clone(&self) -> Self {\n        // SAFETY: the source is one valid aligned claim; this defect deliberately leaves both copies live.\n        unsafe { core::ptr::read(self) }\n    }\n}",
        breaks: "hardware PCI function authority exposes a safe cloning route",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1013,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "#[derive(Debug)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {\n    authority: FunctionAuthority,\n    identity: u32,\n}",
        replace: "#[derive(Debug)]\n#[must_use = \"dropping PCI function authority permanently retains its claim\"]\npub struct FunctionClaim {\n    authority: FunctionAuthority,\n    identity: u32,\n}\n\nimpl FunctionClaim {\n    /// Return a second live copy of this function claim.\n    pub fn duplicate(&self) -> Self {\n        // SAFETY: the source is one valid aligned claim; this defect deliberately leaves both copies live.\n        unsafe { core::ptr::read(self) }\n    }\n}",
        breaks: "hardware PCI function authority exposes a safe duplication door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1024,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub fn read(at: Address) -> Option<u32> {",
        replace: "pub(crate) fn read(at: Address) -> Option<u32> {",
        breaks: "read-only PCI enumeration disappears behind crate visibility",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1025,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub fn claim_function(at: Address) -> Result<FunctionClaim, ClaimError> {",
        replace: "pub(crate) fn claim_function(at: Address) -> Result<FunctionClaim, ClaimError> {",
        breaks: "the sole public PCI function-authority claim door disappears",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1161,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub(super) unsafe fn set_page_table_base(address: u64) {",
        replace: "pub(crate) unsafe fn set_page_table_base(address: u64) {",
        breaks: "the page-table-base instruction door is visible beyond its architecture module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1162,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn save(area: *mut u8) {",
        replace: "pub(crate) unsafe fn save(area: *mut u8) {",
        breaks: "the extended-state save instruction door is visible beyond its architecture module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1163,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn restore(area: *const u8) {",
        replace: "pub(crate) unsafe fn restore(area: *const u8) {",
        breaks: "the extended-state restore instruction door is visible beyond its architecture module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1164,
        file: "wost/src/arch/x86_64/task.rs",
        find: "fn write_thread_base(base: CanonicalAddress) {",
        replace: "pub(super) fn write_thread_base(base: CanonicalAddress) {",
        breaks: "the checked thread-base writer is visible outside its sole caller module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1165,
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "pub(super) unsafe fn set_page_table_base(address: u64) {",
        replace: "pub(super) fn set_page_table_base(address: u64) {",
        breaks: "the page-table-base raw instruction loses its explicit unsafe contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1166,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn save(area: *mut u8) {",
        replace: "pub(super) fn save(area: *mut u8) {",
        breaks: "the extended-state save instruction loses its explicit unsafe contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1167,
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "pub(super) unsafe fn restore(area: *const u8) {",
        replace: "pub(super) fn restore(area: *const u8) {",
        breaks: "the extended-state restore instruction loses its explicit unsafe contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1168,
        file: "wost/src/arch/x86_64/task.rs",
        find: "fn write_thread_base(base: CanonicalAddress) {",
        replace: "unsafe fn write_thread_base(base: CanonicalAddress) {",
        breaks: "the checked thread-base writer becomes a second raw instruction door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1169,
        file: "wost/src/arch/x86_64/msr.rs",
        find: "pub(super) unsafe fn write(msr: u32, value: u64) {",
        replace: "pub unsafe fn write(msr: u32, value: u64) {",
        breaks: "an unreviewed unrestricted public unsafe instruction door enters wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::unsafe_instruction_doors_match_their_compiled_caller_scope",
        },
    },
    Mutation {
        id: 1171,
        file: "wost/src/arch/x86_64/task.rs",
        find: "fn final_extended_state_is_isolated() -> bool {\n    xstate::user_state_is_isolated()\n}",
        replace: "fn final_extended_state_is_isolated() -> bool {\n    xstate::is_enabled()\n}",
        breaks: "the final ring-three crossing trusts remembered extended-state enablement instead of the live isolation predicate",
        reddens: Caught::Check("missing extended state closes the user boundary"),
    },
    Mutation {
        id: 1174,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "pub struct EoiToken<'frame> {\n    vector: InterruptVector,\n    frame: &'frame TrapFrame,\n    interrupts: Option<interrupt::InterruptState>,\n}\n\nimpl fmt::Debug for EoiToken<'_> {",
        replace: "pub struct EoiToken<'frame> {\n    vector: InterruptVector,\n    frame: &'frame TrapFrame,\n    interrupts: Option<interrupt::InterruptState>,\n}\n\nimpl Clone for EoiToken<'_> {\n    fn clone(&self) -> Self {\n        Self {\n            vector: self.vector,\n            frame: self.frame,\n            interrupts: None,\n        }\n    }\n}\n\nimpl fmt::Debug for EoiToken<'_> {",
        breaks: "an APIC completion authority can be cloned while its original remains live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_completion_and_disarm_authorities_are_linear",
        },
    },
    Mutation {
        id: 1175,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "#[derive(Debug)]\npub struct DisarmedTimer {",
        replace: "#[derive(Debug, Clone)]\npub struct DisarmedTimer {",
        breaks: "a timer-disarm proof can be cloned without satisfying Copy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic_completion_and_disarm_authorities_are_linear",
        },
    },
    Mutation {
        id: 1176,
        file: "wost/src/mm/mod.rs",
        find: "#[derive(Debug)]\npub struct Shared(Arc<SharedAllocation>);",
        replace: "#[derive(Debug, Clone)]\npub struct Shared(Arc<SharedAllocation>);",
        breaks: "an unpartitioned shared allocation can be cloned into overlapping access modes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        id: 1177,
        file: "wost/src/mm/mod.rs",
        find: "#[derive(Debug)]\npub struct SharedBytes(Arc<SharedAllocation>);",
        replace: "#[derive(Debug, Clone)]\npub struct SharedBytes(Arc<SharedAllocation>);",
        breaks: "a volatile-byte shared-memory partition can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        id: 1178,
        file: "wost/src/mm/mod.rs",
        find: "#[derive(Debug)]\npub struct SharedRing<const WORDS: usize> {",
        replace: "#[derive(Debug, Clone)]\npub struct SharedRing<const WORDS: usize> {",
        breaks: "a ring-partitioned shared allocation can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::shared_memory_access_requires_one_partition",
        },
    },
    Mutation {
        id: 1265,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        replace: "/// A mutated general unchecked copy door.\npub fn copy_from_user_unwalked(_from: u64, _into: &mut [u8]) -> Result<(), UserCopyError> {\n    Ok(())\n}\n\n#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        breaks: "the removed general unwalked user-copy door returns to the shipped API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 1266,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        replace: "pub fn drive_fault_recovery_for_test() -> bool {",
        breaks: "the fixed user-copy recovery driver reaches the shipped wost graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 1267,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    let exact = left == scratch.len() && scratch == UNCHANGED;",
        replace: "    let exact = left == 0 && scratch == UNCHANGED;",
        breaks: "the fixed-hole driver accepts a completed copy instead of the exact full remainder",
        reddens: Caught::Check("a faulting user copy resumes and reports"),
    },
    Mutation {
        id: 1268,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "    drop(window);\n    let exact = left == scratch.len() && scratch == UNCHANGED;",
        replace: "    drop(window);\n    scratch[0] ^= 1;\n    let exact = left == scratch.len() && scratch == UNCHANGED;",
        breaks: "the fixed-hole driver changes its private destination before reporting",
        reddens: Caught::Check("a faulting user copy resumes and reports"),
    },
    Mutation {
        id: 1269,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "            Some((_, flags)) if flags.contains(Flags::USER) => {",
        replace: "            Some((_, flags)) => {",
        breaks: "a walked user copy accepts a supervisor-only lower leaf",
        reddens: Caught::Check("bytes cross the boundary only where they may"),
    },
    Mutation {
        id: 1282,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "            .read_u32(register::CC)",
        replace: "            .read_u32(register::CSTS)",
        breaks: "NVMe DMA authority reads readiness twice instead of controller enable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::nvme_bus_master_authority_is_fresh_linear_and_mapping_bound",
        },
    },
    Mutation {
        id: 1283,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "#[derive(Debug)]\n#[must_use = \"NVMe quiescence authority must be consumed by DMA admission\"]\npub struct NvmeBusMasterAuthority<'bar> {",
        replace: "#[derive(Debug, Clone)]\n#[must_use = \"NVMe quiescence authority must be consumed by DMA admission\"]\npub struct NvmeBusMasterAuthority<'bar> {",
        breaks: "one fresh NVMe observation can authorize more than one DMA transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1284,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub struct NvmeBusMasterAuthority<'bar> {\n    bar: &'bar MappedBar,\n    _observed: DisabledRegisters,\n}",
        replace: "pub struct NvmeBusMasterAuthority<'bar> {\n    pub bar: &'bar MappedBar,\n    pub _observed: DisabledRegisters,\n}",
        breaks: "safe callers can forge NVMe DMA authority from unrelated values",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1285,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub fn admit_bus_master(\n    authority: NvmeBusMasterAuthority<'_>,",
        replace: "pub fn enable_bus_master(at: Address) -> Result<(), EnableError> {\n    let mut configuration = CONFIGURATION.lock_leaf();\n    let (address, data) = configuration.ports.as_mut().ok_or(EnableError::Access)?;\n    let command = at.at(header::COMMAND).ok_or(EnableError::Stale)?;\n    Port::write(address, command.bits());\n    let current = Port::read(data) as u16;\n    let wanted = current | pci_config::command::MEMORY_SPACE | pci_config::command::BUS_MASTER;\n    Port::write(address, command.bits());\n    Port::write(data, u32::from(wanted));\n    Port::write(address, command.bits());\n    let found = Port::read(data) as u16;\n    if found == wanted { Ok(()) } else { Err(EnableError::Refused(found)) }\n}\n\npub fn admit_bus_master(\n    authority: NvmeBusMasterAuthority<'_>,",
        breaks: "a raw PCI address can enable DMA without mapping or quiescence authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1305,
        file: "wost/src/mm/mod.rs",
        find: "        if !self.layout.contains_publication_word(offset) {\n            return None;\n        }",
        replace: "        if false && !self.layout.contains_publication_word(offset) {\n            return None;\n        }",
        breaks: "a publication load can address bytes outside its immutable word partition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 1306,
        file: "wost/src/mm/mod.rs",
        find: "        if !self.layout.contains_publication_word(offset) {\n            return false;\n        }",
        replace: "        if false && !self.layout.contains_publication_word(offset) {\n            return false;\n        }",
        breaks: "a publication store can address bytes outside its immutable word partition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 1307,
        file: "wost/src/mm/mod.rs",
        find: "        let end = offset.checked_add(size_of::<u32>())?;",
        replace: "        let end = offset.wrapping_add(size_of::<u32>());",
        breaks: "publication-word end arithmetic can wrap into an apparently in-range address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 1308,
        file: "wost/src/mm/mod.rs",
        find: "        if end as u64 > self.len() {\n            return None;\n        }",
        replace: "        if false && end as u64 > self.len() {\n            return None;\n        }",
        breaks: "a publication word can extend beyond its retained allocation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 1309,
        file: "wost/src/mm/mod.rs",
        find: "        let address = base.addr().checked_add(offset)?;",
        replace: "        let address = base.addr().wrapping_add(offset);",
        breaks: "publication pointer arithmetic can wrap away from its checked direct-map base",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 1310,
        file: "wost/src/mm/mod.rs",
        find: "        if !address.is_multiple_of(align_of::<u32>()) {\n            return None;\n        }",
        replace: "        if false && !address.is_multiple_of(align_of::<u32>()) {\n            return None;\n        }",
        breaks: "a misaligned publication word reaches an indivisibility-dependent instruction",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::publication_words_cross_the_hostile_alias_boundary_without_a_rust_object",
        },
    },
    Mutation {
        id: 1311,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if current.is_huge() || current.flags().contains(Flags::USER) {",
        replace: "            if current.flags().contains(Flags::USER) {",
        breaks: "shared topology accepts a huge top-level entry as a table",
        reddens: Caught::Check("shared topology rejects hostile root entries"),
    },
    Mutation {
        id: 1312,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if current.is_huge() || current.flags().contains(Flags::USER) {",
        replace: "            if current.is_huge() {",
        breaks: "shared topology accepts a user-accessible top-level table",
        reddens: Caught::Check("shared topology rejects hostile root entries"),
    },
    Mutation {
        id: 1313,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            self.take_owned_table(DEVICE_WINDOW_START, Level::Pml4, TablePrivilege::Supervisor)?;",
        replace: "            self.take_owned_table(DEVICE_WINDOW_START, Level::Pml4, TablePrivilege::User)?;",
        breaks: "shared topology reserves a user-accessible device-window root",
        reddens: Caught::Check("shared topology rejects hostile root entries"),
    },
    Mutation {
        id: 1316,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if flags.intersection(CACHE_TYPE) != Flags::NONE {",
        replace: "    if flags.contains(CACHE_TYPE) {",
        breaks: "a shared mapping admits an individual non-write-back cache policy alias",
        reddens: Caught::Check("shared mappings reject cache aliases"),
    },
    Mutation {
        id: 1335,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"push rax\",\n    \"mov ax, {kernel_data}\",\n    \"mov ss, ax\",",
        replace: "    \"push rax\",",
        breaks: "a ring-three trap can carry a null stack selector into kernel work",
        reddens: Caught::Check("ring-three entry restores the kernel stack selector"),
    },
    Mutation {
        id: 1336,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "#[cfg(feature = \"selftest\")]\npub const SAVED_FRAME_BYTES: u64 = size_of::<RawTrapFrame>() as u64;",
        replace: "pub const SAVED_FRAME_BYTES: u64 = size_of::<RawTrapFrame>() as u64;",
        breaks: "a machine-only hardware observer reaches the shipped wost API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 1337,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub(super) const RECORD_BUFFER_CLEARS: usize = cfg!(feature = \"selftest\") as usize;",
        replace: "pub(super) const RECORD_BUFFER_CLEARS: usize = 1;",
        breaks: "buffer-clear evidence writes reach shipped return paths",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::buffer_clear_state_is_selftest_only",
        },
    },
    Mutation {
        id: 1339,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \".if {record_clears}\",\n    \"jnz 3f\",\n    \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        replace: "    \".if 1\",\n    \"jnz 3f\",\n    \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        breaks: "the trap-return buffer-clear tally ignores its self-test selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::buffer_clear_assembly_writes_are_guarded",
        },
    },
    Mutation {
        id: 1340,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \".if {record_clears}\",\n    \"jnz 9f\",\n    \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        replace: "    \".if 1\",\n    \"jnz 9f\",\n    \"inc qword ptr [rip + BUFFER_CLEARS]\",",
        breaks: "the first-user-entry buffer-clear tally ignores its self-test selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::buffer_clear_assembly_writes_are_guarded",
        },
    },
    Mutation {
        id: 1341,
        file: "wost/src/mm/mod.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn free_frames() -> usize {",
        replace: "pub fn free_frames() -> usize {",
        breaks: "the free-frame observer reaches the shipped wost API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 1342,
        file: "wost/src/mmio.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    pub(crate) const fn base_address(&self) -> *mut u8 {",
        replace: "    pub(crate) const fn base_address(&self) -> *mut u8 {",
        breaks: "the framebuffer MMIO base observer reaches shipped wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1343,
        file: "wost/src/mm/mod.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    #[must_use = \"a refused segment read copied no bytes\"]\n    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {",
        replace: "    #[must_use = \"a refused segment read copied no bytes\"]\n    pub fn read_at(&self, offset: usize, into: &mut [u8]) -> bool {",
        breaks: "the owned-segment byte observer reaches shipped wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 1344,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    pub const fn was_user(&self) -> bool {",
        replace: "    pub const fn was_user(&self) -> bool {",
        breaks: "the unmapped-range user-bit observer reaches shipped wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 1345,
        file: "wost/src/mm/heap.rs",
        find: "        #[cfg(feature = \"selftest\")]\n        if !self.record_slab_frame() {",
        replace: "        if !self.record_slab_frame() {",
        breaks: "the slab-evidence helper executes on the shipped refill path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1348,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            #[cfg(feature = \"selftest\")]\n            let result = if mapping_refusal(&MAPPING_WRITES_BEFORE_REFUSAL) {\n                Err(MapError::OutOfMemory)\n            } else {\n                self.set(virtual_address, physical, flags, level)\n            };\n            #[cfg(not(feature = \"selftest\"))]\n            let result = self.set(virtual_address, physical, flags, level);",
        replace: "            #[cfg(feature = \"selftest\")]\n            let injected = mapping_refusal(&MAPPING_WRITES_BEFORE_REFUSAL);\n            #[cfg(not(feature = \"selftest\"))]\n            let injected = false;\n            let result = if injected {\n                Err(MapError::OutOfMemory)\n            } else {\n                self.set(virtual_address, physical, flags, level)\n            };",
        breaks: "the shipped mapping loop carries a no-op test-refusal branch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1371,
        file: "wost/src/panic.rs",
        find: "    crate::arch::current::pci::force_unlock(panic);\n",
        replace: "",
        breaks: "the panic path forgets to reclaim the PCI configuration lock",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_static_lock_in_wost_is_reclaimed_by_the_panic_path",
        },
    },
    Mutation {
        id: 1372,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    PERMANENT_STACKS.force_unlock(panic);\n    TASK_STACK_PIN.force_unlock(panic);\n",
        replace: "    PERMANENT_STACKS.force_unlock(panic);\n",
        breaks: "the paging panic door omits the task stack pin ledger",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_static_lock_in_wost_is_reclaimed_by_the_panic_path",
        },
    },
    Mutation {
        id: 1410,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "pub struct FunctionClaim {\n    authority: FunctionAuthority,\n    identity: u32,\n}",
        replace: "pub struct FunctionClaim {\n    authority: FunctionAuthority,\n    pub identity: u32,\n}",
        breaks: "hardware PCI function authority exposes its private claim identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1411,
        file: "wost/src/mmio.rs",
        find: "        self.write_values(offset, values.iter().copied())",
        replace: "        // SAFETY: this weakened path assumes every supplied word is writable.\n        unsafe {\n            core::ptr::copy_nonoverlapping(\n                values.as_ptr(),\n                self.base.wrapping_add(offset).cast(),\n                values.len(),\n            )\n        };\n        Ok(())",
        breaks: "framebuffer slice publication bypasses checked volatile MMIO with a raw copy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::framebuffer_rows_publish_one_checked_volatile_span_before_shadow_commit",
        },
    },
    Mutation {
        id: 1412,
        file: "wost/src/boot/mod.rs",
        find: "                let Some(values) = span.source(&self.back) else {\n                    panic!(\"framebuffer shadow is shorter than its validated mapping\");\n                };\n                framebuffer_access(self.region.write_u32_slice(destination, values));\n                if span.copy_within(&mut self.back).is_none() {\n                    panic!(\"framebuffer shadow is shorter than its validated mapping\");\n                }",
        replace: "                if span.copy_within(&mut self.back).is_none() {\n                    panic!(\"framebuffer shadow is shorter than its validated mapping\");\n                }\n                let Some(values) = span.source(&self.back) else {\n                    panic!(\"framebuffer shadow is shorter than its validated mapping\");\n                };\n                framebuffer_access(self.region.write_u32_slice(destination, values));",
        breaks: "framebuffer shadow commits a row before device publication succeeds",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::framebuffer_rows_publish_one_checked_volatile_span_before_shadow_commit",
        },
    },
    Mutation {
        id: 1451,
        file: "wost/src/mmio.rs",
        find: "        let range = self.check_words(offset, values.len())?;",
        replace: "        let range = self.check_words(offset, values.len().min(1))?;",
        breaks: "checked MMIO publishes more words than the span it proved",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::framebuffer_rows_publish_one_checked_volatile_span_before_shadow_commit",
        },
    },
    Mutation {
        id: 1452,
        file: "wost/src/mmio.rs",
        find: "            unsafe { word.write_volatile(value) };",
        replace: "            unsafe { word.write(value) };",
        breaks: "checked MMIO words are published with ordinary writes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::framebuffer_rows_publish_one_checked_volatile_span_before_shadow_commit",
        },
    },
    Mutation {
        id: 1476,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "impl FunctionClaim {\n",
        replace: "impl FunctionClaim {\n    /// Split this claim between two holders.\n    pub fn split(self) -> (Self, Self) {\n        // SAFETY: the source is one valid aligned claim; this defect deliberately leaves both copies live.\n        let twin = unsafe { core::ptr::read(&self) };\n        (self, twin)\n    }\n\n",
        breaks: "hardware PCI function authority splits one claim into two",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1478,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "static CONFIGURATION: SpinLock<Configuration> = SpinLock::new(Configuration::new());\n",
        replace: "static CONFIGURATION: SpinLock<Configuration> = SpinLock::new(Configuration::new());\n\n/// A claim retained outside every driver.\nstatic SPARE: SpinLock<Option<FunctionClaim>> = SpinLock::new(None);\n",
        breaks: "a static can hold hardware PCI function authority outside every claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1479,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "/// A refused function-authority release with the unchanged claim.\n",
        replace: "/// Produce a second holder from one borrowed value.\npub trait Twin: Sized {\n    /// Return a second live copy.\n    fn twin(&self) -> Self {\n        // SAFETY: the source is one valid aligned value; this defect deliberately leaves both copies live.\n        unsafe { core::ptr::read(self) }\n    }\n}\n\nimpl Twin for FunctionClaim {}\n\n/// A refused function-authority release with the unchanged claim.\n",
        breaks: "hardware PCI function authority inherits a duplicating trait default",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1480,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "use crate::sync::SpinLock;\n",
        replace: "use crate::sync::SpinLock;\nuse pci_config::FunctionAuthority as Lease;\n\nimpl FunctionClaim {\n    /// Lend the embedded pure authority.\n    pub fn lease(&self) -> Lease {\n        // SAFETY: the source is one valid aligned authority; this defect deliberately leaves both copies live.\n        unsafe { core::ptr::read(&self.authority) }\n    }\n}\n",
        breaks: "a renamed import hides a hardware PCI authority duplication door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1481,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "/// Functions that may be held by drivers at once.\n",
        replace: "macro_rules! claim_twin {\n    ($claim:expr) => {\n        // SAFETY: the claim is one valid aligned value; this defect deliberately leaves both copies live.\n        unsafe { core::ptr::read($claim) }\n    };\n}\n\n/// Functions that may be held by drivers at once.\n",
        breaks: "a macro can generate PCI authority doors the inventory cannot see",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_function_authority_has_audited_mints_and_no_duplication_door",
        },
    },
    Mutation {
        id: 1525,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if !user_mapping_fits(at, len) {",
        replace: "        if at.checked_add(len).is_none_or(|end| end > USER_END) {",
        breaks: "a user space may map its last canonical page",
        reddens: Caught::Check("the last canonical page is never user memory"),
    },
    Mutation {
        id: 1528,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if flags.intersection(CACHE_TYPE) != Flags::NONE {\n        return Err(MapError::CacheAlias(virtual_base));\n    }\n    Ok(())",
        replace: "    let _ = (virtual_base, flags);\n    Ok(())",
        breaks: "RAM may be mapped with a cache type its direct map does not share",
        reddens: Caught::Check("ram mappings keep the direct map's cache type"),
    },
    Mutation {
        id: 1529,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        ram_cache_policy(virtual_base, flags)?;\n        self.may_reach(virtual_base, len)?;",
        replace: "        self.may_reach(virtual_base, len)?;",
        breaks: "owned and user RAM may be mapped uncached or write-through",
        reddens: Caught::Check("ram mappings keep the direct map's cache type"),
    },
    Mutation {
        id: 1530,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if flags.intersection(CACHE_TYPE) != region.cache_type() {\n            return Err(MapError::CacheAlias(virtual_base));\n        }\n",
        replace: "",
        breaks: "a boot range may be mapped with a cache type other than its own",
        reddens: Caught::Check("ram mappings keep the direct map's cache type"),
    },
    // The unit caches the stale translation, so the device reaches the frame
    // it was told to forget.
    Mutation {
        id: 1544,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if let Err(reason) = self.drain_dma() {\n            failure.get_or_insert(reason);\n        }\n        if let Some(reason) = failure {",
        replace: "        if let Some(reason) = failure {",
        breaks: "an unmapped segment stays reachable through the unit's cache",
        reddens: Caught::Check("a device write into an unmapped frame faults and changes nothing"),
    },
    Mutation {
        id: 1545,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "    fn map_range<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(\n        &mut self,\n        paddr: u64,\n        len: u64,\n    ) -> Result<(), SegmentChangeError> {\n        let Some(last) = len.checked_sub(1).and_then(|last| paddr.checked_add(last)) else {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        };\n        if !self.width.covers_on(self.capability, last) {\n            return Err(SegmentChangeError::Refused(VtdError::OutsideAddressWidth));\n        }\n        let start = paddr & !(PAGE - 1);\n        let mut page = start;\n        while page <= last {",
        replace: "    fn map_range<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(\n        &mut self,\n        paddr: u64,\n        len: u64,\n    ) -> Result<(), SegmentChangeError> {\n        let Some(last) = len.checked_sub(1).and_then(|last| paddr.checked_add(last)) else {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        };\n        if !self.width.covers_on(self.capability, last) {\n            return Err(SegmentChangeError::Refused(VtdError::OutsideAddressWidth));\n        }\n        let start = paddr & !(PAGE - 1);\n        let mut page = start;\n        while page <= last + PAGE {",
        breaks: "every mapped segment reaches one frame past its end",
        reddens: Caught::Check("the dma domain reaches exactly the live queue frames"),
    },
    Mutation {
        id: 1546,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !translation_enabled(status) {\n            return None;\n        }\n        let root = RootEntry {",
        replace: "        let _translation = translation_enabled(status);\n        let root = RootEntry {",
        breaks: "a domain proof is minted while translation is off",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1547,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.switch_interrupt_remapping(true)?;\n        self.switch_translation(true)?;\n        self.drain_dma()?;",
        replace: "        self.switch_interrupt_remapping(true)?;\n        self.drain_dma()?;",
        breaks: "the unit is programmed but translation is never switched on",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1548,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        (function.device() << 3) | function.function()\n",
        replace: "        (function.device() << 3) | (function.function() ^ 1)\n",
        breaks: "the context entry is written for a sibling function, so the storage function has no translation",
        reddens: Caught::Check("translation is active for the storage function"),
    },
    Mutation {
        id: 1554,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "pub struct DmaDomain<'unit> {\n    function: Address,\n    _unit: PhantomData<&'unit Vtd>,\n}",
        replace: "pub struct DmaDomain<'unit> {\n    pub function: Address,\n    pub _unit: PhantomData<&'unit Vtd>,\n}",
        breaks: "a DMA domain proof can be forged without a unit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1555,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        write_u64(&mut self.registers, vtd::RTADDR, self.root.address())?;\n        let command = set_root_table_pointer(self.gcmd);\n        write_u32(&mut self.registers, vtd::GCMD, command)?;\n        self.poll_status(root_table_pointer_set, VtdError::RootTableRefused)?;\n        let pointer = read_u64(&self.registers, vtd::RTADDR)?;\n        if root_table_address(pointer) == self.root.address() {\n",
        replace: "        write_u64(&mut self.registers, vtd::RTADDR, self.context.address())?;\n        let command = set_root_table_pointer(self.gcmd);\n        write_u32(&mut self.registers, vtd::GCMD, command)?;\n        self.poll_status(root_table_pointer_set, VtdError::RootTableRefused)?;\n        let pointer = read_u64(&self.registers, vtd::RTADDR)?;\n        if root_table_address(pointer) == self.context.address() {\n",
        breaks: "the root pointer is programmed with the context table's address and read back as expected, so the unit walks a table the kernel never meant as its root",
        reddens: Caught::Panic {
            marker: "the bound-space witness did not retire",
            why: "the wrong root pointer quarantines the early VT-d owner, so interrupt admission \
                  keeps timer preemption disabled and the published bound-space task cannot retire",
        },
    },
    Mutation {
        id: 1579,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "#[derive(Debug)]\n#[must_use = \"a DMA domain proof is required to admit bus mastering\"]",
        replace: "#[derive(Debug, Clone, Copy)]\n#[must_use = \"a DMA domain proof is required to admit bus mastering\"]",
        breaks: "a DMA domain proof can be copied and outlive the release that invalidated it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1580,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "#[derive(Debug)]\n#[must_use = \"a DMA domain proof is required to admit bus mastering\"]",
        replace: "#[derive(Debug, Clone)]\n#[must_use = \"a DMA domain proof is required to admit bus mastering\"]",
        breaks: "a DMA domain proof can be cloned and kept past the admission it permits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::pci_configuration_writes_require_function_authority",
        },
    },
    Mutation {
        id: 1591,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        let mut address = self.span.start;\n",
        replace: "        let mut address = self.span.end;\n",
        breaks: "the painted-stack scan starts at its own boundary and reads none of the span the candidate wrote",
        reddens: Caught::Check("nvme candidates preserve two pages of boot-stack headroom"),
    },
    Mutation {
        id: 1592,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "#[cfg(feature = \"selftest\")]\n#[derive(Debug)]\npub struct PaintedStack {",
        replace: "#[derive(Debug)]\npub struct PaintedStack {",
        breaks: "the painted-stack type reaches the shipped wost API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 1623,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "    fn map_range<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(\n        &mut self,\n        paddr: u64,\n        len: u64,\n    ) -> Result<(), SegmentChangeError> {\n        let Some(last) = len.checked_sub(1).and_then(|last| paddr.checked_add(last)) else {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        };\n        if !self.width.covers_on(self.capability, last) {",
        replace: "    fn map_range<const DEVICE_READS: bool, const DEVICE_WRITES: bool>(\n        &mut self,\n        paddr: u64,\n        len: u64,\n    ) -> Result<(), SegmentChangeError> {\n        let Some(last) = len.checked_sub(1).and_then(|last| paddr.checked_add(last)) else {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        };\n        if !self.width.covers(last) {",
        breaks: "the production DMA mapping seam ignores the unit's MGAW",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_enforces_effective_coverage_before_the_walk",
        },
    },
    Mutation {
        id: 1625,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            Err(error @ SegmentChangeError::Retained(_)) => {\n                segment.retain();\n                Err(error)\n            }",
        replace: "            Err(error @ SegmentChangeError::Retained(_)) => Err(error),",
        breaks: "an uncertain VT-d map can return a device-reachable segment to its caller",
        reddens: Caught::Check("uncertain vtd mapping never recycles its frames"),
    },
    Mutation {
        id: 1640,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if let Some(reason) = failure {\n            segment.retain();\n            Err(SegmentChangeError::Retained(reason))",
        replace: "        if let Some(reason) = failure {\n            Err(SegmentChangeError::Retained(reason))",
        breaks: "an uncertain VT-d unmap returns device-reachable frames to safe ownership",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_enforces_effective_coverage_before_the_walk",
        },
    },
    Mutation {
        id: 1642,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            Err(SegmentChangeError::Retained(reason))\n        } else {",
        replace: "            Err(SegmentChangeError::Refused(reason))\n        } else {",
        breaks: "an uncertain VT-d unmap disguises retained device reachability as a clean refusal",
        reddens: Caught::Check("uncertain vtd unmapping never recycles its frames"),
    },
    Mutation {
        id: 1655,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        while drained < usize::from(self.fault_registers) && self.take_fault().is_some() {",
        replace: "        while drained <= usize::from(self.fault_registers) && self.take_fault().is_some() {",
        breaks: "the bounded VT-d drain can inspect more live records than the hardware inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::device_fault_reporting_is_bounded_and_follows_retirement",
        },
    },
    Mutation {
        id: 1663,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        while drained < usize::from(self.fault_registers) && self.take_fault().is_some() {",
        replace: "        while drained < usize::MAX && self.take_fault().is_some() {",
        breaks: "the VT-d fault drain uses an unbounded constant instead of the hardware inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::device_fault_reporting_is_bounded_and_follows_retirement",
        },
    },
    Mutation {
        id: 1664,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            drained += 1;",
        replace: "            let _record_did_not_spend_budget = drained;",
        breaks: "the VT-d fault drain never spends its advertised record budget",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::device_fault_reporting_is_bounded_and_follows_retirement",
        },
    },
    Mutation {
        id: 1675,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.switch_translation(true)?;\n        self.drain_dma()?;\n        self.boundary_active()",
        replace: "        self.switch_translation(true)?;\n        self.boundary_active()",
        breaks: "pre-boundary DMA is not drained after the empty root becomes active",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1707,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !platform_interrupt_remapping {\n            return Err(VtdError::NoPlatformInterruptRemapping);\n        }",
        replace: "        if false && !platform_interrupt_remapping {\n            return Err(VtdError::NoPlatformInterruptRemapping);\n        }",
        breaks: "VT-d takeover accepts firmware that reports no interrupt-remapping platform support",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1708,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !extended.interrupt_remapping() || !extended.queued_invalidation() {",
        replace: "        if !extended.interrupt_remapping() {",
        breaks: "VT-d takeover accepts interrupt remapping without queued IEC invalidation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1709,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !extended.extended_interrupt_mode() {\n            return Err(VtdError::NoExtendedInterruptMode);\n        }",
        replace: "        if false && !extended.extended_interrupt_mode() {\n            return Err(VtdError::NoExtendedInterruptMode);\n        }",
        breaks: "VT-d takeover accepts a unit that cannot isolate x2APIC interrupt destinations",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1713,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if observed == pointer\n            && interrupt_table_base(observed) == self.interrupts.address()",
        replace: "        if interrupt_table_base(observed) == self.interrupts.address()",
        breaks: "interrupt-table publication accepts a readback with unexpected mode or size bits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1714,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.switch_queued_invalidation(true)?;\n        let observed_queue = read_u64(&self.registers, vtd::IQA)?;",
        replace: "        let observed_queue = read_u64(&self.registers, vtd::IQA)?;\n        self.switch_queued_invalidation(true)?;",
        breaks: "the invalidation queue is read back before the hardware enables queued invalidation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1715,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            || !interrupt_remapping_enabled(status)\n            || compatibility_interrupts_enabled(status)",
        replace: "            || false && !interrupt_remapping_enabled(status)\n            || compatibility_interrupts_enabled(status)",
        breaks: "an interrupt-isolation proof is minted while interrupt remapping is off",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1716,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            || compatibility_interrupts_enabled(status)\n            || status & gsts::QIES != 0",
        replace: "            || false && compatibility_interrupts_enabled(status)\n            || status & gsts::QIES != 0",
        breaks: "an interrupt-isolation proof is minted while compatibility-format interrupts bypass remapping",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1717,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            || status & gsts::QIES != 0\n            || !fault_event_control::masked(fault_control)",
        replace: "            || false && status & gsts::QIES != 0\n            || !fault_event_control::masked(fault_control)",
        breaks: "an interrupt-isolation proof is minted while the private invalidation queue is still live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1718,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            if self.interrupts.read_u64(index) != Some(0) {",
        replace: "            if self.interrupts.read_u64(index) == Some(u64::MAX) {",
        breaks: "an interrupt-isolation proof accepts present or otherwise nonzero remapping entries",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1722,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "    if interrupts.function() != domain.function() {\n        return Err(EnableError::Stale);\n    }",
        replace: "    if false && interrupts.function() != domain.function() {\n        return Err(EnableError::Stale);\n    }",
        breaks: "bus mastering accepts interrupt isolation minted for a different function",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1728,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "pub struct InterruptIsolation<'unit> {\n    function: Address,\n    _unit: PhantomData<&'unit Vtd>,\n}",
        replace: "pub struct InterruptIsolation<'unit> {\n    pub function: Address,\n    pub _unit: PhantomData<&'unit Vtd>,\n}",
        breaks: "an interrupt-isolation proof can be forged without a live VT-d owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1729,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        for index in 0..4 {\n            if self.interrupts.read_u64(index) != Some(0) {\n                return false;\n            }\n        }",
        replace: "        let _unchecked_interrupt_entries = &self.interrupts;",
        breaks: "interrupt isolation is minted without reading back the deny-all table entries",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1730,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        let disabled = self.switch_queued_invalidation(false);",
        replace: "        let disabled = Ok(());",
        breaks: "queued invalidation remains enabled after the private IEC flush completes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1732,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.set_interrupt_table()?;\n        self.invalidate_interrupt_cache()?;",
        replace: "        self.set_interrupt_table()?;",
        breaks: "interrupt remapping is enabled without globally invalidating the interrupt-entry cache",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1734,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    for (bits, register) in requests.iter_mut().zip(register::INTERRUPT_REQUEST) {",
        replace: "    for (bits, register) in requests\n        .iter_mut()\n        .zip(register::INTERRUPT_REQUEST[..7].iter().copied())\n    {",
        breaks: "the pre-sti audit ignores pending vectors in the highest local APIC IRR bank",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1735,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    if let Some(vector) = pending_vector(&access) {\n        return Err(PendingInterruptError::Pending(vector));\n    }",
        replace: "    let _unchecked_pending = pending_vector(&access);",
        breaks: "sti proceeds while an interrupt accepted before remapping remains pending",
        reddens: Caught::Check("a pending local interrupt refuses the first sti"),
    },
    Mutation {
        id: 1739,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.mask_fault_interrupt()?;\n        self.set_root_table()?;",
        replace: "        self.set_root_table()?;",
        breaks: "VT-d takeover leaves its non-remapped fault-event MSI armed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1740,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            || status & gsts::QIES != 0\n            || !fault_event_control::masked(fault_control)",
        replace: "            || status & gsts::QIES != 0",
        breaks: "interrupt isolation is minted while VT-d can inject its own fault-event vector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1741,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            prior | fault_event_control::MASK,",
        replace: "            fault_event_control::MASK,",
        breaks: "fault-event masking clears the register's preserved bits",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1742,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        let function = self.function.ok_or(VtdError::NoActiveDomain)?;\n        write_entry(&mut self.root, function.bus(), 0, 0)?;",
        replace: "        let function = self.function.ok_or(VtdError::NoActiveDomain)?;\n        self.switch_translation(false)?;\n        write_entry(&mut self.root, function.bus(), 0, 0)?;",
        breaks: "candidate retirement opens an unrestricted-DMA interval before fallback",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1762,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    register::LOCAL_VECTOR_TABLE.get(..=maximum)",
        replace: "    register::LOCAL_VECTOR_TABLE.get(..=usize::from(LAST_REQUIRED_LVT_ENTRY))",
        breaks: "an advertised CMCI source can retain firmware ownership during APIC takeover",
        reddens: Caught::Check("apic takeover masks exactly every advertised local source"),
    },
    Mutation {
        id: 1763,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "        if !Entry::from_bits(low_u32(InitializationRegisters::read(&registers, entry))).is_masked()\n        {\n            return Err(ApicError::NotResponding);\n        }",
        replace: "        let _ = InitializationRegisters::read(&registers, entry);",
        breaks: "APIC takeover can publish after a local source refuses its mask write",
        reddens: Caught::Check("unmaskable or unknown apic source inventories poison takeover"),
    },
    Mutation {
        id: 1764,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    if maximum < usize::from(LAST_REQUIRED_LVT_ENTRY) {",
        replace: "    if maximum < usize::from(LAST_REQUIRED_LVT_ENTRY) - 1 {",
        breaks: "APIC takeover accepts an inventory missing the kernel-owned error source",
        reddens: Caught::Check("unmaskable or unknown apic source inventories poison takeover"),
    },
    Mutation {
        id: 1765,
        file: "wost/src/arch/x86_64/apic.rs",
        find: "    register::LOCAL_VECTOR_TABLE.get(..=maximum)",
        replace: "    register::LOCAL_VECTOR_TABLE.get(..=maximum.min(register::LOCAL_VECTOR_TABLE.len() - 1))",
        breaks: "APIC takeover silently ignores a future source advertised beyond the known inventory",
        reddens: Caught::Check("unmaskable or unknown apic source inventories poison takeover"),
    },
    Mutation {
        id: 1766,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if flags.is_write_execute() {",
        replace: "        if flags.difference(Flags::WRITABLE).is_write_execute() {",
        breaks: "mapping admission erases write permission before testing W^X",
        reddens: Caught::Check("writable executable mappings leave no trace"),
    },
    Mutation {
        id: 1776,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if let Err(reason) = self.drain_dma() {\n            failure.get_or_insert(reason);\n        }\n        if let Some(reason) = failure {",
        replace: "        if let Err(reason) = self.invalidate(false) {\n            failure.get_or_insert(reason);\n        }\n        if let Some(reason) = failure {",
        breaks: "live VT-d unmap recycles frames after invalidation without draining older DMA",
        reddens: Caught::Check("a device write into an unmapped frame faults and changes nothing"),
    },
    Mutation {
        id: 1777,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if let Err(reason) = self.drain_dma() {\n            failure.get_or_insert(reason);\n        }\n        failure.map_or(Ok(()), Err)",
        replace: "        if let Err(reason) = self.invalidate(false) {\n            failure.get_or_insert(reason);\n        }\n        failure.map_or(Ok(()), Err)",
        breaks: "partial VT-d mapping rollback recycles frames without draining older DMA",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_revocation_drains_older_dma_before_recycling_frames",
        },
    },
    Mutation {
        id: 1778,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.flush_write_buffer()\n    }\n\n    fn flush_write_buffer",
        replace: "        Ok(())\n    }\n\n    fn flush_write_buffer",
        breaks: "drained VT-d invalidation skips the unit's required write-buffer flush",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_revocation_drains_older_dma_before_recycling_frames",
        },
    },
    Mutation {
        id: 1779,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            iotlb_command::global_with_drain(),",
        replace: "            iotlb_command::global(),",
        breaks: "the VT-d DMA drain command requests neither read nor write draining",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_revocation_drains_older_dma_before_recycling_frames",
        },
    },
    Mutation {
        id: 1781,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !version.register_invalidation_supported() {",
        replace: "        if version.bits() == u32::MAX {",
        breaks: "VT-d preflight accepts units whose register invalidations are architecturally ignored",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_revocation_drains_older_dma_before_recycling_frames",
        },
    },
    Mutation {
        id: 1782,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.poll_u64(command_register, iotlb_command::global_completed)?;",
        replace: "        self.poll_u64(command_register, iotlb_command::completed)?;",
        breaks: "a drained VT-d invalidation treats an ignored global request as complete",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_revocation_drains_older_dma_before_recycling_frames",
        },
    },
    Mutation {
        id: 1783,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            self.poll_u64(vtd::CCMD, context_command::global_completed)?;",
        replace: "            self.poll_u64(vtd::CCMD, context_command::completed)?;",
        breaks: "VT-d context invalidation accepts an ignored global request",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_revocation_drains_older_dma_before_recycling_frames",
        },
    },
    Mutation {
        id: 1784,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.poll_u64(command_register, iotlb_command::global_completed)\n    }",
        replace: "        self.poll_u64(command_register, iotlb_command::completed)\n    }",
        breaks: "ordinary VT-d IOTLB invalidation accepts an ignored global request",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_revocation_drains_older_dma_before_recycling_frames",
        },
    },
    Mutation {
        id: 1791,
        file: "wost/src/mm/mod.rs",
        find: "        if !span.write_u64_volatile_at(offset, value) {",
        replace: "        if !span.write_volatile_at(offset, &value.to_le_bytes()) {",
        breaks: "a live VT-d entry is published through eight byte stores",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1794,
        file: "wost/src/mm/mod.rs",
        find: "        unsafe { destination.cast::<u64>().write_volatile(value.to_le()) };",
        replace: "        let _ = self.write_byte_volatile(offset, value.to_le_bytes()[0]);\n        unsafe { destination.cast::<u64>().write_volatile(value.to_le()) };",
        breaks: "a permission byte becomes visible before the atomic VT-d entry publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1795,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.map_segment_with::<true, false>(segment)",
        replace: "        self.map_segment_with::<true, true>(segment)",
        breaks: "device-read-only DMA authority also grants device writes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1796,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            if let Err(reason) = self.map_page::<DEVICE_READS, DEVICE_WRITES>(page) {",
        replace: "            if let Err(reason) = self.map_page::<true, true>(page) {",
        breaks: "the range walk silently widens every leaf mapping",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1797,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        self.map_segment_with::<false, true>(segment)",
        replace: "        self.map_segment_with::<true, true>(segment)",
        breaks: "device-write-only DMA authority also grants device reads",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1801,
        file: "wost/src/mm/mod.rs",
        find: "        if !span.write_u64_volatile_at(offset, value) {",
        replace: "        let _ = span.write_byte_volatile(offset, value.to_le_bytes()[0]);\n        if !span.write_u64_volatile_at(offset, value) {",
        breaks: "the table writer exposes a permission byte before its atomic qword publication",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1802,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if self\n            .tables\n            .write_u64(slot, second_level::pte(page, DEVICE_READS, DEVICE_WRITES))",
        replace: "        let _ = self.tables.write_u64(slot, second_level::pte(page, true, true));\n        if self\n            .tables\n            .write_u64(slot, second_level::pte(page, DEVICE_READS, DEVICE_WRITES))",
        breaks: "a leaf is transiently published with bidirectional DMA authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1811,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !takeover_ready(status) {\n            return Establishment::Refused {\n                reason: VtdError::FirmwareFacilitiesActive { status },\n                registers,\n            };\n        }",
        replace: "        if status == u32::MAX {\n            return Establishment::Refused {\n                reason: VtdError::FirmwareFacilitiesActive { status },\n                registers,\n            };\n        }",
        breaks: "the post-allocation firmware-state preflight is removed before the first unit write",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 2048,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.roots().is_paired() && is_user_address(virtual_base) && !flags.contains(Flags::USER)\n        {\n            return Err(MapError::NotYours(virtual_base));",
        replace: "        if self.roots().is_paired() && is_user_address(virtual_base) && flags.contains(Flags::USER)\n        {\n            return Err(MapError::NotYours(virtual_base));",
        breaks: "a paired stripped root can inherit supervisor-only lower-half leaves",
        reddens: Caught::Check("a paired root admits only user leaves below the split"),
    },
    Mutation {
        id: 2053,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        paired_flush_premise(self.roots())?;\n        Ok(())",
        replace: "        let text = crate::boot::sections().text.start;\n        let (physical, _, _) = walk(self.root(), text)\n            .ok_or(MapError::InvalidEntryEnclave(text))?;\n        self.set_in(\n            TableRoot::Stripped,\n            text,\n            physical,\n            Flags::PRESENT,\n            Level::Pt,\n        )?;\n        paired_flush_premise(self.roots())?;\n        Ok(())",
        breaks: "an ordinary kernel text leaf enters the stripped root outside the exact enclave",
        reddens: Caught::Check("the user root contains only the entry enclave"),
    },
    Mutation {
        id: 2054,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    super::kpti::record_rust_entry(saved.code_selector);",
        replace: "    super::kpti::record_rust_entry(0);",
        breaks: "a user-origin entry reaches Rust without recording its full-root observation",
        reddens: Caught::Check("every user entry reaches Rust on the full root"),
    },
    Mutation {
        id: 2055,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"lock inc qword ptr [rip + WOST_KPTI_ROUTE + {route_prepared_returns}]\",",
        replace: "    \"cmp qword ptr [rip + WOST_KPTI_ROUTE + {route_prepared_returns}], 0\",",
        breaks: "a prepared user return loses its stripped-root transition evidence",
        reddens: Caught::Check("every user return installs the stripped root"),
    },
    Mutation {
        id: 2056,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"mov rcx, {raw_frame_words}\",\n    \"rep movsq\",\n    \"xor eax, eax\",",
        replace: "    \"mov rcx, {raw_frame_words}\",\n    \"rep movsq\",\n    \"mov eax, 1\",",
        breaks: "user entry leaves secret-capable transition and IST stack bytes unsanitized",
        reddens: Caught::Check("the entry stack is guarded and scrubbed"),
    },
    Mutation {
        id: 2057,
        file: "wost/src/arch/x86_64/kpti.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    record_complete_pair_switch(roots);\n    true",
        replace: "    #[cfg(feature = \"selftest\")]\n    let _unrecorded_pair = roots;\n    true",
        breaks: "a complete paired-root context switch loses its atomic pair observation",
        reddens: Caught::Check("a context switch moves one complete root pair"),
    },
    Mutation {
        id: 2058,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if let Some(stripped) = roots.stripped_root() {\n            // SAFETY: paired construction allocated and forgot this distinct\n            // root exactly once, and the active-root refusal covers it too.\n            drop(unsafe { mm::Frame::from_raw(stripped) });\n        }",
        replace: "        if let Some(stripped) = roots.stripped_root() {\n            let _leaked_stripped_root = stripped;\n        }",
        breaks: "paired retirement strands the stripped root instead of returning every frame",
        reddens: Caught::Check("paired-space refusal and retirement conserve every frame"),
    },
    Mutation {
        id: 2059,
        file: "wost/src/boot/mod.rs",
        find: "        core::hint::black_box(symbol).addr() as u64",
        replace: "        symbol.addr() as u64",
        breaks: "release optimization can reject linker-aliased enclave boundaries as unequal",
        reddens: Caught::Check("the kernel image obeys write-xor-execute"),
    },
    Mutation {
        id: 2061,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"verw word ptr [rip + {verw_selector}]\",",
        replace: "    \"sub rsp, 8\",\n    \"mov word ptr [rsp], {kernel_data}\",\n    \"verw word ptr [rsp]\",\n    \"add rsp, 8\",",
        breaks: "post-CR3 VERW re-dirties the already-scrubbed transition stack",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "elf::tests::kpti_exit_loads_stripped_root_after_scrub_and_last_secret_access",
        },
    },
    Mutation {
        id: 2288,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "HandoffAccess::QuiesceIsaDma => isa_dma.as_mut().map(IsaDma::quiesce),",
        replace: "HandoffAccess::QuiesceIsaDma => Some(0),",
        breaks: "the platform reports a quiet 8237 pair without masking it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2307,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        self.first.write::<u8>(0x0D, 0);\n",
        replace: "",
        breaks: "the entry ISA fence leaves the first controller's inherited DMA command state intact",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2308,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        self.second.write::<u8>(0x1A, 0);\n",
        replace: "",
        breaks: "the entry ISA fence leaves the second controller's inherited DMA command state intact",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2309,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        self.first.write::<u8>(0x0F, 0x0F);\n",
        replace: "",
        breaks: "the ISA requester is accepted without masking the first controller's channels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2310,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        self.second.write::<u8>(0x1E, 0x0F);\n",
        replace: "",
        breaks: "the ISA requester is accepted without masking the second controller's channels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2311,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        let first = self.first.read::<u8>(0x08);",
        replace: "        let first = self.first.read::<u8>(0x00);",
        breaks: "the ISA fence reads an address register instead of the first controller's status",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2312,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        let second = self.second.read::<u8>(0x10);",
        replace: "        let second = self.second.read::<u8>(0x00);",
        breaks: "the ISA fence reads an address register instead of the second controller's status",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2313,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        u32::from(first) | u32::from(second) << 8",
        replace: "        u32::from(first) | u32::from(first) << 8",
        breaks: "the second controller's pending requests are replaced by the first controller's status",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2314,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        u32::from(first) | u32::from(second) << 8",
        replace: "        u32::from(first) | u32::from(second) << 4",
        breaks: "the second controller's request bits miss the combined pending-request mask",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2316,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        controllers.reassert() & pci_config::ISA_DMA_PENDING_REQUESTS == 0",
        replace: "        0u32 & pci_config::ISA_DMA_PENDING_REQUESTS == 0",
        breaks: "a later unit-wide ISA fence accepts a stale quiet result without remasking",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2317,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "    ISA_DMA.lock_leaf().as_mut().is_some_and(|controllers| {",
        replace: "    ISA_DMA.lock_leaf().as_mut().is_none_or(|controllers| {",
        breaks: "the ISA quiet predicate accepts missing permanent controller ownership",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2318,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        *ISA_DMA.lock_leaf() = Some(controllers);",
        replace: "        *ISA_DMA.lock_leaf() = None;",
        breaks: "the entry ISA fence discards the controller capabilities instead of retaining them",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2319,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "    if let Some(controllers) = isa_dma {\n        *ISA_DMA.lock_leaf() = Some(controllers);\n    }",
        replace: "    if fenced.is_ok() {\n        if let Some(controllers) = isa_dma {\n            *ISA_DMA.lock_leaf() = Some(controllers);\n        }\n    }",
        breaks: "an uncertain entry fence loses its permanent ISA controller owner on refusal",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2320,
        file: "wost/src/arch/x86_64/port.rs",
        find: "    claim_block_pair(0x00, 0xC0)",
        replace: "    claim_block_pair(0x10, 0xC0)",
        breaks: "the permanent paired ISA claim reserves the wrong first-controller port window",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2321,
        file: "wost/src/arch/x86_64/pci.rs",
        find: "        self.first.write::<u8>(0x0F, 0x0F);\n        self.second.write::<u8>(0x1E, 0x0F);\n        let first = self.first.read::<u8>(0x08);\n        let second = self.second.read::<u8>(0x10);",
        replace: "        let first = self.first.read::<u8>(0x08);\n        let second = self.second.read::<u8>(0x10);\n        self.first.write::<u8>(0x0F, 0x0F);\n        self.second.write::<u8>(0x1E, 0x0F);",
        breaks: "the ISA quiet predicate uses status sampled before both channel masks were written",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::isa_dma_adapter_clears_masks_and_rechecks_both_controllers",
        },
    },
    Mutation {
        id: 2349,
        file: "wost/src/mm/mod.rs",
        find: "        if self.mapping_owner.is_some() {\n            self.retain();\n        }",
        replace: "        if false && self.mapping_owner.is_some() {\n            self.retain();\n        }",
        breaks: "dropping a still-mapped DMA segment recycles device-reachable frames",
        reddens: Caught::Check("a mapped dma owner drop retains its allocation"),
    },
    Mutation {
        id: 2350,
        file: "wost/src/mm/mod.rs",
        find: "            core::mem::forget(table);",
        replace: "            drop(table);",
        breaks: "dropping a published IOMMU table recycles a possible hardware table",
        reddens: Caught::Check("published iommu table drops retain their allocations"),
    },
    Mutation {
        id: 2351,
        file: "wost/src/mm/mod.rs",
        find: "        if self.segment.is_none() || self.mapping_owner.is_some() {",
        replace: "        if self.segment.is_none() {",
        breaks: "a second mapping replaces a segment lease before its first revocation",
        reddens: Caught::Check("a proven vtd revocation releases only its mapping owner"),
    },
    Mutation {
        id: 2352,
        file: "wost/src/mm/mod.rs",
        find: "        self.mapping_owner == Some(owner)",
        replace: "        let _ = owner;\n        self.mapping_owner.is_some()",
        breaks: "a foreign root releases another unit's DMA mapping lease",
        reddens: Caught::Check("a mapped dma owner drop retains its allocation"),
    },
    Mutation {
        id: 2353,
        file: "wost/src/mm/mod.rs",
        find: "        self.mapping_owner = Some(owner);",
        replace: "        let _ = owner;",
        breaks: "mapping acquisition returns success without recording its frame-retention lease",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2354,
        file: "wost/src/mm/mod.rs",
        find: "        self.mapping_owner = None;",
        replace: "        let _unreleased_mapping = self.mapping_owner;",
        breaks: "proved revocation leaves the old segment mapping lease live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2355,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !segment.begin_mapping(owner) {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        }",
        replace: "        let _ = segment.begin_mapping(owner);",
        breaks: "VT-d walks and publishes after the segment rejected its mapping lease",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2356,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        let owner = DmaMappingOwner::new(self.root.address());\n        if !segment.begin_mapping(owner) {",
        replace: "        let owner = DmaMappingOwner::new(0);\n        if !segment.begin_mapping(owner) {",
        breaks: "different VT-d units collapse onto one synthetic mapping-owner identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2357,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            Ok(()) => Ok(()),\n        }\n    }\n\n    fn map_range",
        replace: "            Ok(()) => {\n                let _ = segment.release_mapping(owner);\n                Ok(())\n            }\n        }\n    }\n\n    fn map_range",
        breaks: "successful mapping relinquishes its frame-retention lease while leaves remain live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2358,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "                assert!(\n                    segment.release_mapping(owner),\n                    \"mapping owner changed during rollback\"\n                );",
        replace: "                let _unreleased_owner = owner;",
        breaks: "a proved mapping rollback keeps an otherwise reclaimable segment tagged",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2359,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !segment.is_mapped_by(owner) {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        }",
        replace: "        if false && !segment.is_mapped_by(owner) {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        }",
        breaks: "unmapping edits tables before proving that this unit owns the segment lease",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2360,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "        if !segment.is_mapped_by(owner) {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        }",
        replace: "        if !segment.is_mapped_by(owner) {\n            return Err(SegmentChangeError::Refused(VtdError::InvalidRange));\n        }\n        let _ = segment.release_mapping(owner);",
        breaks: "unmapping releases the segment lease before clearing leaves or draining DMA",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2361,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            assert!(\n                segment.release_mapping(owner),\n                \"mapping owner changed during revocation\"\n            );",
        replace: "            let _unreleased_owner = owner;",
        breaks: "successful leaf revocation never releases its exact mapping lease",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_mapping_and_table_owners_are_composed_without_reclamation_gaps",
        },
    },
    Mutation {
        id: 2362,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "    pub fn domain(&self) -> Option<DmaDomain<'_>> {",
        replace: "    pub fn domain(&self) -> Option<DmaDomain<'static>> {",
        breaks: "the DmaDomain admission proof escapes the live unit borrow",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::vtd_admission_proofs_borrow_the_exact_live_unit",
        },
    },
    Mutation {
        id: 2363,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "    pub fn interrupt_isolation(&self) -> Option<InterruptIsolation<'_>> {",
        replace: "    pub fn interrupt_isolation(&self) -> Option<InterruptIsolation<'static>> {",
        breaks: "the InterruptIsolation admission proof escapes the live unit borrow",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::vtd_admission_proofs_borrow_the_exact_live_unit",
        },
    },
    Mutation {
        id: 2364,
        file: "wost/src/arch/x86_64/vtd.rs",
        find: "            Err(reason) => match self.rollback_range(start, last) {\n                Ok(()) => Err(SegmentChangeError::Refused(reason)),",
        replace: "            Err(reason) => match self.rollback_range(start, last) {\n                Ok(()) => Err(SegmentChangeError::Retained(reason)),",
        breaks: "a completed mapping rollback is misclassified as permanently retained ownership",
        reddens: Caught::Check("a rolled back vtd mapping releases its mapping owner"),
    },
    Mutation {
        id: 2369,
        file: "wost/src/mm/mod.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn dma_mapping_owner_drop_retains_for_test() -> bool {\n    let compact = size_of::<Option<DmaMappingOwner>>() == size_of::<Paddr>();\n    let before = free_frames();\n    let (Some(mapped), Some(revoked)) = (allocate_contiguous(1, 1), allocate_contiguous(2, 1))\n    else {\n        return false;\n    };\n    let owner = DmaMappingOwner::new(mapped.address());\n    let foreign = DmaMappingOwner::new(revoked.address());\n    let mut mapped = DmaSegment::from(mapped);\n    let mut revoked = DmaSegment::from(revoked);\n    let mapped_ok = mapped.begin_mapping(owner);\n    let refused = !mapped.begin_mapping(owner)\n        && !mapped.begin_mapping(foreign)\n        && !mapped.release_mapping(foreign)\n        && mapped.is_mapped_by(owner);\n    let released = !revoked.release_mapping(owner)\n        && revoked.begin_mapping(owner)\n        && revoked.release_mapping(owner)\n        && !revoked.is_mapped_by(owner)\n        && !revoked.release_mapping(owner);\n    drop(revoked);\n    let reclaimed = free_frames() == before - 1;\n    drop(mapped);\n    compact && mapped_ok && refused && released && reclaimed && free_frames() == before - 1\n}",
        replace: "pub fn dma_mapping_owner_drop_retains_for_test() -> bool { true }",
        breaks: "the dma_mapping_owner_drop_retains_for_test machine-only driver becomes a shipped public API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 2370,
        file: "wost/src/mm/mod.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn published_iommu_table_drop_retains_for_test() -> bool {\n    let before = free_frames();\n    let Some(unpublished) = IommuTable::new(1, TableWalks::Coherent) else {\n        return false;\n    };\n    let allocated = free_frames() == before - 1;\n    drop(unpublished);\n    if !allocated || free_frames() != before {\n        return false;\n    }\n    let mut retained = 0;\n    for frames in [1, 2, 3, 4, 5] {\n        let Some(table) = IommuTable::new(frames, TableWalks::Coherent) else {\n            return false;\n        };\n        let address = table.address();\n        let mut published = PublishedIommuTable::new(table);\n        let exact = published.address() == address\n            && published.write_u64(0, address)\n            && published.read_u64(0) == Some(address);\n        retained += frames;\n        drop(published);\n        if !exact || free_frames() != before - retained {\n            return false;\n        }\n    }\n    true\n}",
        replace: "pub fn published_iommu_table_drop_retains_for_test() -> bool { true }",
        breaks: "the published_iommu_table_drop_retains_for_test machine-only driver becomes a shipped public API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
    },
    Mutation {
        id: 2429,
        file: "wost/src/mm/mod.rs",
        find: "#[repr(transparent)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub(crate) struct DmaMappingOwner(NonZeroU64);\n\nimpl DmaMappingOwner {\n    /// Name a root whose published owner permanently withholds its allocation.\n    ///\n    /// # Panics\n    ///\n    /// Panics if the root is not frame aligned.\n    pub(crate) const fn new(root: Paddr) -> Self {\n        let Some(identity) = frame_identity(root) else {\n            panic!(\"a DMA mapping owner root is not frame aligned\");\n        };\n        Self(identity)\n    }\n}",
        replace: "#[repr(transparent)]\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub(crate) struct DmaMappingOwner(Option<NonZeroU64>);\n\nimpl DmaMappingOwner {\n    /// Name a root whose published owner permanently withholds its allocation.\n    ///\n    /// # Panics\n    ///\n    /// Panics if the root is not frame aligned.\n    pub(crate) const fn new(root: Paddr) -> Self {\n        let Some(identity) = frame_identity(root) else {\n            panic!(\"a DMA mapping owner root is not frame aligned\");\n        };\n        Self(Some(identity))\n    }\n}",
        breaks: "optional DMA mapping ownership occupies a second word and inflates the candidate stack",
        reddens: Caught::Check("a mapped dma owner drop retains its allocation"),
    },
];
