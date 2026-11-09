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
        id: 571,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    assert!(index < ENTRIES, \"a level's index is always below 512\");",
        replace: "    debug_assert!(index < ENTRIES, \"a level's index is always below 512\");",
        breaks: "release page-table slot formation loses its index bound",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::production_invariants_are_not_debug_only",
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
        id: 718,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    fn device_reclaim_error(&self) -> Option<ReclaimError> {\n        let mappings = self.devices.iter().flatten().count();\n        (mappings != 0).then_some(ReclaimError::Devices(mappings))\n    }",
        replace: "    fn device_reclaim_error(&self) -> Option<ReclaimError> {\n        let _mappings = self.devices.iter().flatten().count();\n        None\n    }",
        breaks: "a live device ledger no longer prevents its page tables from being reclaimed",
        reddens: Caught::Check("active device tables cannot be reclaimed"),
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
            name: "contracts::paging::paging_activation_loads_cr3_before_publishing_ready",
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
        id: 988,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    fn map_device_range(\n        &mut self,\n        admitted: &AdmittedDeviceRange,\n    ) -> Result<DeviceMapping, MapError> {",
        replace: "    fn map_device_range(\n        &mut self,\n        admitted: &AdmittedDeviceRange,\n    ) -> Result<DeviceMapping, MapError> {\n        let _fixed_projection = [None::<DeviceExtent>; MAX_DEVICE_MAPPINGS];",
        breaks: "device mapping rebuilds a maximum-sized ownership projection on the kernel stack",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::an_address_space_keeps_its_bounded_ledger_off_the_stack",
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
            name: "contracts::address_space::an_address_space_keeps_its_bounded_ledger_off_the_stack",
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
            name: "contracts::address_space::a_kernel_stack_top_cannot_outlive_its_pin",
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
            name: "contracts::address_space::a_kernel_stack_top_cannot_outlive_its_pin",
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
            name: "contracts::address_space::an_address_space_keeps_its_bounded_ledger_off_the_stack",
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
        id: 1344,
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    #[cfg(feature = \"selftest\")]\n    pub const fn was_user(&self) -> bool {",
        replace: "    pub const fn was_user(&self) -> bool {",
        breaks: "the unmapped-range user-bit observer reaches shipped wost",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine::machine_probes_are_absent_from_the_shipped_wost_graph",
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
            name: "contracts::machine::machine_evidence_writes_are_absent_from_shipped_paths",
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
            name: "contracts::doors::every_static_lock_in_wost_is_reclaimed_by_the_panic_path",
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
            name: "contracts::machine::machine_probes_are_absent_from_the_shipped_wost_graph",
        },
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
];
