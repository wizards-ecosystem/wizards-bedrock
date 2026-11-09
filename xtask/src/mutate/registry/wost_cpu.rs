use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
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
            name: "contracts::paging::task_switch_loads_cr3_under_its_guard_before_context_state",
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
            name: "contracts::recovery::each_recovery_adapter_binds_the_live_architectural_values",
        },
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
            name: "contracts::machine::buffer_clear_assembly_writes_are_guarded",
        },
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
            name: "contracts::apic::guard_types_do_not_cross_cpus",
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
        id: 572,
        file: "wost/src/arch/x86_64/segment.rs",
        find: "fn write_stack(at: usize, top: u64) {\n    assert!(",
        replace: "fn write_stack(at: usize, top: u64) {\n    debug_assert!(",
        breaks: "release TSS stack writes lose their alignment precondition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::production_invariants_are_not_debug_only",
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
            name: "contracts::paging::task_init_validates_the_arena_before_publication_and_pinning",
        },
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
            name: "contracts::machine::machine_probes_are_absent_from_the_shipped_wost_graph",
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
            name: "contracts::paging::invlpg_requires_canonical_address_authority",
        },
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
        id: 860,
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "#[cfg(feature = \"selftest\")]\n#[must_use = \"a refused resume changed no trap frame\"]\npub(super) fn resume_write_protection(",
        replace: "#[cfg(feature = \"selftest\")]\n#[must_use = \"a refused write-protection resume changed no trap frame\"]\npub(super) fn resume_write_protection(",
        breaks: "the write-protection recovery door loses its pinned must-use contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::fallible_operation_bools_stay_must_use",
        },
    },
    Mutation {
        id: 869,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "        frame_error_code: frame.error_code(),",
        replace: "        frame_error_code: u64::from(frame.vector()),",
        breaks: "the user-copy adapter substitutes the trap vector for the saved error word",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::recovery::each_recovery_adapter_binds_the_live_architectural_values",
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
            name: "contracts::recovery::each_recovery_adapter_binds_the_live_architectural_values",
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
            name: "contracts::recovery::each_recovery_adapter_binds_the_live_architectural_values",
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
            name: "contracts::recovery::each_recovery_adapter_binds_the_live_architectural_values",
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
            name: "contracts::recovery::each_recovery_adapter_binds_the_live_architectural_values",
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
            name: "contracts::recovery::every_instruction_pointer_writer_requires_its_opaque_authority",
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
            name: "contracts::recovery::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
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
            name: "contracts::recovery::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
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
            name: "contracts::recovery::every_instruction_pointer_writer_refuses_a_non_live_frame_first",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
        vector: u64::from(frame.vector()),
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
        vector: u64::from(frame.vector()),
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
            name: "contracts::recovery::recovery_authority_cannot_be_built_from_a_raw_address",
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
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
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
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
            name: "contracts::doors::unsafe_instruction_doors_match_their_compiled_caller_scope",
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
        id: 1265,
        file: "wost/src/arch/x86_64/usercopy.rs",
        find: "#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        replace: "/// A mutated general unchecked copy door.\npub fn copy_from_user_unwalked(_from: u64, _into: &mut [u8]) -> Result<(), UserCopyError> {\n    Ok(())\n}\n\n#[cfg(feature = \"selftest\")]\npub fn drive_fault_recovery_for_test() -> bool {",
        breaks: "the removed general unwalked user-copy door returns to the shipped API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine::machine_probes_are_absent_from_the_shipped_wost_graph",
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
            name: "contracts::machine::machine_probes_are_absent_from_the_shipped_wost_graph",
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
            name: "contracts::machine::machine_probes_are_absent_from_the_shipped_wost_graph",
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
            name: "contracts::machine::buffer_clear_state_is_selftest_only",
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
            name: "contracts::machine::buffer_clear_assembly_writes_are_guarded",
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
            name: "contracts::machine::buffer_clear_assembly_writes_are_guarded",
        },
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
];
