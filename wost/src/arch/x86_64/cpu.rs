//! Direct CPU control: halting, feature discovery, and the protection bits.
//!
//! The interrupt flag lives in [`interrupt`] instead, next to
//! the table whose existence is the precondition for setting it.

use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, __get_cpuid_max};
use core::fmt;

#[cfg(feature = "selftest")]
use core::sync::atomic::{AtomicUsize, Ordering};

use x86_cpuid::physical::{ADDRESS_SIZE_LEAF, PhysicalAddressWidth};
use x86_cpuid::{
    CpuidLeaf, Cr4Additions, ExecutionEnvironment, FrequencyHints, SPEC_CTRL_MSR,
    SharedKernelMappingReason, Speculation,
};
use x86_paging::CanonicalAddress;

use super::interrupt;
use super::msr;

/// Extended feature enable register.
const EFER: u32 = 0xC000_0080;

/// `EFER` bit: enable `syscall`/`sysret`.
///
/// This kernel enters through its DPL-3 interrupt gate instead (D-96).
const EFER_SYSCALL: u64 = 1 << 0;

/// `IA32_SYSENTER_CS`. Zero makes `sysenter` fault, which is the shut state.
const SYSENTER_CS: u32 = 0x174;

/// `EFER` bit: honour the no-execute page bit rather than treating it as
/// reserved.
const EFER_NO_EXECUTE: u64 = 1 << 11;

/// `CR0` bit: supervisor writes obey read-only page-table entries.
const CR0_WRITE_PROTECT: u64 = 1 << 16;

/// `CR4` bit: user mode instruction prevention.
const CR4_UMIP: u64 = 1 << 11;

const CR4_PCIDE: u64 = 1 << 17;

/// `CR4` bit: supervisor mode execution prevention.
const CR4_SMEP: u64 = 1 << 20;

/// `CR4` bit: supervisor mode access prevention.
const CR4_SMAP: u64 = 1 << 21;

#[cfg(feature = "selftest")]
::core::arch::global_asm!(
    ".pushsection .rodata.wost_write_protection_probe,\"a\",@progbits",
    ".globl wost_write_protection_target",
    "wost_write_protection_target:",
    "  .byte 0xa5",
    ".popsection",
    ".text",
    ".globl wost_write_protection_probe",
    "wost_write_protection_probe:",
    "  lea rdi, [rip + wost_write_protection_target]",
    "  mov al, [rdi]",
    ".globl wost_write_protection_fault",
    "wost_write_protection_fault:",
    "  mov [rdi], al",
    ".globl wost_write_protection_fault_end",
    "wost_write_protection_fault_end:",
    "  xor eax, eax",
    "  ret",
    ".globl wost_write_protection_fixup",
    "wost_write_protection_fixup:",
    "  mov eax, 1",
    "  ret",
);

#[cfg(feature = "selftest")]
unsafe extern "C" {
    /// Write the byte back to its own read-only location.
    fn wost_write_protection_probe() -> u64;
    /// The write instruction the probe expects to fault on.
    static wost_write_protection_fault: u8;
    /// One past the write instruction.
    static wost_write_protection_fault_end: u8;
    /// Where the recovered write resumes.
    static wost_write_protection_fixup: u8;
    /// The byte in the kernel's read-only image.
    static wost_write_protection_target: u8;
}

#[cfg(feature = "selftest")]
pub(super) struct WriteProtectionRecoveryTarget(u64);

#[cfg(feature = "selftest")]
impl WriteProtectionRecoveryTarget {
    fn fixup() -> Self {
        Self(core::ptr::addr_of!(wost_write_protection_fixup) as u64)
    }

    pub(super) const fn address(&self) -> u64 {
        self.0
    }
}

/// Halt this CPU until an enabled interrupt or another architectural wake event.
///
/// An ordinary maskable interrupt cannot wake `hlt` while `IF` is clear.
pub fn halt() {
    // SAFETY: `hlt` in ring 0 stops the CPU until an interrupt arrives. It has
    // no architectural precondition and cannot fail. It remains a compiler
    // memory barrier so state publication cannot move past the sleep.
    unsafe { asm!("hlt", options(nostack, preserves_flags)) };
}

/// Disable interrupts and halt permanently.
///
/// This is the kernel's terminal state: used by the panic handler and by the
/// idle path before a scheduler exists.
pub fn halt_forever() -> ! {
    let _disabled = interrupt::disable();
    loop {
        halt();
    }
}

/// Whether this processor's timestamp counter runs at a constant rate
/// whatever the core is doing.
///
/// Without this CPUID guarantee, throttling and deep idle may change or stop
/// the count, making it a performance counter rather than a clock.
pub fn has_invariant_tsc() -> bool {
    /// `cpuid` leaf 0x8000_0007, `EDX` bit 8.
    const INVARIANT: u32 = 1 << 8;

    max_leaf(0x8000_0000) >= 0x8000_0007 && cpuid(0x8000_0007, 0).edx & INVARIANT != 0
}

/// Frequency evidence from `cpuid` leaves 0x15 and 0x16.
///
/// The pure decoder keeps the architectural ratio distinct from leaf 0x16's
/// nominal processor frequency. Unsupported leaves are represented as absent
/// rather than queried and mistaken for answers from another leaf.
pub fn frequency_hints() -> FrequencyHints {
    let maximum = max_leaf(0);
    let ratio = (maximum >= 0x15).then(|| {
        let leaf = cpuid(0x15, 0);
        (leaf.eax, leaf.ebx, leaf.ecx)
    });
    let base_mhz = (maximum >= 0x16).then(|| cpuid(0x16, 0).eax);
    FrequencyHints::from_leaves(ratio, base_mhz)
}

/// The running processor's validated `MAXPHYADDR`.
pub fn physical_address_width() -> Option<PhysicalAddressWidth> {
    let maximum = max_leaf(0x8000_0000);
    let eax = if maximum >= ADDRESS_SIZE_LEAF {
        cpuid(ADDRESS_SIZE_LEAF, 0).eax
    } else {
        0
    };
    PhysicalAddressWidth::from_leaf(maximum, eax)
}

/// The timestamp counter: cycles since the processor started.
///
/// `lfence` and the compiler memory barrier keep this non-serialising
/// instruction inside the interval it is meant to measure.
pub fn timestamp() -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: `rdtsc` reads the counter into edx:eax and has no preconditions
    // in ring 0 — `CR4.TSD`, the bit that could make it fault, gates ring 3
    // only, and this kernel never sets it. The `lfence` before it orders
    // execution and touches no state.
    unsafe {
        asm!(
            "lfence",
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nostack, preserves_flags),
        );
    }
    u64::from(high) << 32 | u64::from(low)
}

/// The current stack pointer.
pub fn stack_pointer() -> u64 {
    let value: u64;
    // SAFETY: reading `rsp` has no preconditions and no effects.
    unsafe {
        asm!(
            "mov {value}, rsp",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

/// The address whose access raised the last page fault.
///
/// Meaningful only inside a page fault handler: `CR2` is written by the fault
/// and holds whatever the previous one left there otherwise.
pub fn faulting_address() -> u64 {
    let value: u64;
    // SAFETY: reading a control register in ring 0 has no preconditions and no
    // effects.
    unsafe {
        asm!(
            "mov {value}, cr2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

/// The CPU protections that separate ring 0 from ring 3.
///
/// Each is independent, and each may be absent on an older CPU or a virtual
/// machine that does not expose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protections {
    /// Supervisor writes honour read-only page-table entries.
    pub write_protect: bool,
    /// Supervisor mode execution prevention: ring 0 cannot execute from a page
    /// marked user-accessible.
    pub smep: bool,
    /// Supervisor mode access prevention: ring 0 cannot read or write one
    /// either, except inside a deliberate window around a checked copy.
    pub smap: bool,
    /// User mode instruction prevention: ring 3 cannot read the descriptor
    /// table registers, which would otherwise leak kernel addresses.
    pub umip: bool,
    /// The no-execute page bit is honoured rather than reserved. What makes
    /// W^X enforceable once the kernel owns its page tables.
    pub nx: bool,
}

impl Protections {
    /// What this CPU implements, whether or not it is switched on.
    pub fn supported() -> Self {
        let structured = if max_leaf(0) >= 7 {
            cpuid(7, 0)
        } else {
            CpuidResult::default()
        };
        let extended = if max_leaf(0x8000_0000) >= 0x8000_0001 {
            cpuid(0x8000_0001, 0)
        } else {
            CpuidResult::default()
        };

        Self {
            write_protect: true,
            smep: structured.ebx & 1 << 7 != 0,
            smap: structured.ebx & 1 << 20 != 0,
            umip: structured.ecx & 1 << 2 != 0,
            nx: extended.edx & 1 << 20 != 0,
        }
    }

    /// What is switched on, read back from the control registers rather than
    /// remembered from whatever was written.
    pub fn enabled() -> Self {
        let cr4 = read_cr4();
        Self {
            write_protect: write_protection_is_enabled(),
            smep: cr4 & CR4_SMEP != 0,
            smap: cr4 & CR4_SMAP != 0,
            umip: cr4 & CR4_UMIP != 0,
            nx: msr::read(EFER) & EFER_NO_EXECUTE != 0,
        }
    }
}

impl fmt::Display for Protections {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for (enabled, name) in [
            (self.write_protect, "wp"),
            (self.smep, "smep"),
            (self.smap, "smap"),
            (self.umip, "umip"),
            (self.nx, "nx"),
        ] {
            if enabled {
                core::fmt::Write::write_str(f, separator)?;
                core::fmt::Write::write_str(f, name)?;
                separator = " ";
            }
        }
        if separator.is_empty() {
            core::fmt::Write::write_str(f, "none")?;
        }
        Ok(())
    }
}

const CR4_OSXSAVE: u64 = 1 << 18;

/// Read from the register rather than from a remembered flag: the one caller
/// is a check asking whether the write took, and a remembered flag would
/// answer with the memory of the write (D-115).
pub fn extended_state_is_enabled() -> bool {
    read_cr4() & CR4_OSXSAVE != 0
}

/// Enable extended-state instructions after all layout preflight succeeds.
///
/// This is restricted to [`xstate::init`](super::xstate::init), because enabling
/// SSE before save/restore is ready would share register state with ring 3
/// (D-115, D-123).
pub(super) fn enable_extended_state() {
    /// `CR0.MP`: monitor coprocessor, which the manual pairs with `EM` clear.
    const CR0_MONITOR: u64 = 1 << 1;
    /// `CR0.EM`: emulation. Set, every SSE instruction faults.
    const CR0_EMULATE: u64 = 1 << 2;

    write_cr0((read_cr0() | CR0_MONITOR) & !CR0_EMULATE);
    add_cr4(Cr4Additions::extended_state());
}

/// Enable every protection in [`Protections`] this CPU supports, and report
/// what is now on.
///
/// Safe because none of these bits can produce undefined behaviour: the worst
/// an unexpected one can do is fault, which is a trap the kernel reports. That
/// is why this is called after the interrupt table is installed and not before.
pub fn enable_protections() -> Protections {
    let supported = Protections::supported();

    enable_write_protection();

    let mut additions = Cr4Additions::none();
    if supported.smep {
        additions = additions.with_smep();
    }
    if supported.smap {
        additions = additions.with_smap();
    }
    if supported.umip {
        additions = additions.with_umip();
    }
    add_cr4(additions);

    if supported.nx {
        // SAFETY: the only register named is `EFER`, and the only bit added to
        // what it already held is the one `cpuid` just reported as supported.
        unsafe { msr::write(EFER, msr::read(EFER) | EFER_NO_EXECUTE) };
    }

    shut_the_fast_syscall_doors();
    Protections::enabled()
}

/// Whether supervisor writes honour read-only page-table entries.
///
/// Read back from `CR0`, not remembered from the write that asked for it.
pub fn write_protection_is_enabled() -> bool {
    read_cr0() & CR0_WRITE_PROTECT != 0
}

/// Drive the write-protect bit off and through the kernel's enabling path.
///
/// Firmware commonly leaves `CR0.WP` set, so the check needs a controlled
/// off/on transition. Interrupts stay excluded and a failed enable is repaired.
#[cfg(feature = "selftest")]
pub fn drive_write_protection_for_test() -> bool {
    let disabled = interrupt::disable();
    write_cr0(read_cr0() & !CR0_WRITE_PROTECT);
    let was_cleared = !write_protection_is_enabled();
    enable_write_protection();
    let was_enabled = write_protection_is_enabled();
    if !was_enabled {
        write_cr0(read_cr0() | CR0_WRITE_PROTECT);
    }
    let result = was_cleared && was_enabled;
    disabled.restore();
    result
}

/// Attempt a supervisor write to a byte in the kernel's read-only image.
///
/// The assembly reads the byte and writes the same value back, so a broken
/// protection leaves the image unchanged. `true` means the write faulted and
/// the trap handler resumed at this probe's private fixup.
#[cfg(feature = "selftest")]
pub fn write_protection_probe() -> bool {
    // SAFETY: the assembly names one byte in this crate's own read-only image
    // and writes back exactly the value it read. The trap path recognises only
    // this instruction and address; without protection, no value changes.
    unsafe { wost_write_protection_probe() != 0 }
}

/// Resume the controlled write-protection probe, if it caused this fault.
///
/// `false` for every other instruction or address, having changed nothing.
#[cfg(feature = "selftest")]
pub fn recover_write_protection_probe(
    frame: &interrupt::TrapFrame,
    fault: trap_cause::PageFault,
) -> bool {
    let at = frame.instruction_pointer();
    let start = core::ptr::addr_of!(wost_write_protection_fault) as u64;
    let end = core::ptr::addr_of!(wost_write_protection_fault_end) as u64;
    let target = core::ptr::addr_of!(wost_write_protection_target) as u64;
    let Some(target_end) = target.checked_add(1) else {
        return false;
    };
    let attempt = trap_cause::RecoveryAttempt {
        vector: frame.vector(),
        frame_error_code: frame.error_code(),
        code_selector: frame.code_selector(),
        instruction_pointer: at,
        page_fault: fault,
        faulting_address: faulting_address(),
    };
    let rule = trap_cause::RecoveryRule {
        fault: trap_cause::RecoveryFault::SupervisorWriteProtection,
        fault_addresses: target..target_end,
        instruction_addresses: start..end,
    };
    if !attempt.is_admitted_by(&rule) {
        return false;
    }
    interrupt::resume_write_protection(frame, WriteProtectionRecoveryTarget::fixup())
}

fn enable_write_protection() {
    write_cr0(read_cr0() | CR0_WRITE_PROTECT);
}

/// Close `syscall` and `sysenter`, which this kernel does not use.
///
/// Both instructions bypass the IDT and would enter through state this kernel
/// never configured, so boot closes them explicitly instead of trusting
/// firmware defaults (D-139).
fn shut_the_fast_syscall_doors() {
    // SAFETY: the only register named is `EFER`, and the only bit *removed*
    // from what it already held is the one that enables an instruction this
    // kernel never executes — no `syscall` or `sysret` appears in the tree.
    unsafe { msr::write(EFER, msr::read(EFER) & !EFER_SYSCALL) };
    // SAFETY: `IA32_SYSENTER_CS` selects the ring-0 code segment `sysenter`
    // would enter through. Zero is the architectural shut state: `sysenter`
    // with a null selector raises `#GP` rather than transferring.
    unsafe { msr::write(SYSENTER_CS, 0) };
}

/// Drive a fast-syscall door open and through the kernel's closing path.
///
/// Firmware already boots the test machine shut, so only a controlled
/// closed/open/closed transition makes the production close causal (D-139).
/// Interrupts stay excluded while the door is open.
#[cfg(feature = "selftest")]
pub fn drive_fast_syscall_shutdown_for_test() -> bool {
    let disabled = interrupt::disable();
    let shut_at_start = fast_syscall_doors_are_shut();
    // SAFETY: sets the bit that enables an instruction this kernel never
    // executes — no `syscall` or `sysret` appears in the tree — and interrupts
    // are excluded until the private closing path runs below, so no existing
    // ring-3 context can resume through the open door.
    unsafe { msr::write(EFER, msr::read(EFER) | EFER_SYSCALL) };
    let reader_sees_open = !fast_syscall_doors_are_shut();
    shut_the_fast_syscall_doors();
    let shut_again = fast_syscall_doors_are_shut();
    disabled.restore();
    shut_at_start && reader_sees_open && shut_again
}

/// Whether both fast-syscall doors are shut.
///
/// Read back from the registers rather than from a flag this module set, for
/// the reason D-115 gives about `XCR0`: a Rust bool records that the write was
/// attempted, and the question is what the CPU holds.
pub fn fast_syscall_doors_are_shut() -> bool {
    msr::read(EFER) & EFER_SYSCALL == 0 && msr::read(SYSENTER_CS) == 0
}

/// Whether this processor has a local advanced programmable interrupt
/// controller of its own.
pub(super) fn has_local_apic() -> bool {
    /// `cpuid` leaf 1, `EDX` bit 9.
    const APIC: u32 = 1 << 9;

    cpuid(1, 0).edx & APIC != 0
}

/// Whether that controller has an x2APIC mode, where its registers are
/// model-specific registers rather than a page of memory.
pub(super) fn has_x2apic() -> bool {
    /// `cpuid` leaf 1, `ECX` bit 21.
    const X2APIC: u32 = 1 << 21;

    cpuid(1, 0).ecx & X2APIC != 0
}

/// Whether `EFLAGS.AC` is set right now — that is, whether SMAP is currently
/// letting ring 0 touch user pages.
///
/// Read rather than assumed, because the whole point of D-101 is that ring 3
/// can set this bit and interrupt delivery does not clear it.
#[cfg(feature = "selftest")]
pub fn access_check_is_off() -> bool {
    /// `EFLAGS.AC`.
    const AC: u64 = 1 << 18;

    let flags: u64;
    // SAFETY: `pushfq` and `pop` use one matching stack slot and restore `rsp`;
    // no Rust value aliases the temporary word.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    flags & AC != 0
}

/// Whether this processor implements `MD_CLEAR`, the `VERW` buffer flush.
///
/// `cpuid` leaf 7 subleaf 0, `EDX` bit 10. Separate from [`Speculation`]
/// because it is a *capability* rather than a claim about immunity: a machine
/// may have the instruction and also report `MDS_NO`, in which case using it
/// buys nothing (D-98).
pub fn has_buffer_clear() -> bool {
    /// `cpuid` leaf 7 subleaf 0, `EDX` bit 10.
    const MD_CLEAR: u32 = 1 << 10;

    max_leaf(0) >= 7 && cpuid(7, 0).edx & MD_CLEAR != 0
}

/// Read the machine's speculation posture and enable enhanced IBRS if it has
/// it.
///
/// `x86_cpuid` owns the host-tested policy. QEMU TCG lacks the architectural
/// capabilities MSR, so native/VMware qualification exercises the write
/// (D-81, D-91).
pub fn enable_speculation_controls() -> Speculation {
    let posture = speculation_posture();
    if !posture.wants_boot_ibrs() {
        return posture;
    }

    let wanted = posture.spec_ctrl_after(msr::read(SPEC_CTRL_MSR));
    // SAFETY: `IA32_SPEC_CTRL` selects a speculation mode and nothing else.
    // Setting `IBRS` on a processor reporting `IBRS_ALL` changes how indirect
    // branches are predicted, never what any instruction means, and the value
    // preserves every bit firmware set.
    unsafe { msr::write(SPEC_CTRL_MSR, wanted) };
    posture
}

fn speculation_posture() -> Speculation {
    // A nonexistent leaf may alias another answer and must not authorize an
    // architectural-capabilities MSR read (D-94).
    let leaf_7_edx = if max_leaf(0) >= 7 { cpuid(7, 0).edx } else { 0 };
    Speculation::read(leaf_7_edx, msr::read)
}

/// Return the CPU evidence that permits kernel mappings in user page tables.
///
/// The QEMU TCG answer is trusted development-host configuration, not a
/// hardware attestation. Native and other virtualized machines therefore need
/// the processor's affirmative `RDCL_NO` report.
pub fn shared_kernel_mapping_reason(
    speculation: &Speculation,
) -> Option<SharedKernelMappingReason> {
    x86_cpuid::shared_kernel_mapping_reason(speculation, execution_environment())
}

/// Whether CPU state binds this boot to a qualified DMA-handoff platform.
///
/// PCI requesters can rewrite guest RAM but cannot forge the architectural
/// hypervisor leaf. ACPI handoff bytes therefore have authority only on the
/// exact QEMU TCG and VMware release environments recognized by `x86-cpuid`.
pub fn dma_handoff_platform_is_authenticated() -> bool {
    execution_environment().authenticates_dma_handoff()
}

fn execution_environment() -> ExecutionEnvironment {
    let leaf_1 = cpuid(1, 0);
    ExecutionEnvironment::read(leaf_1.ecx, || {
        let leaf = cpuid(0x4000_0000, 0);
        CpuidLeaf::new(leaf.eax, leaf.ebx, leaf.ecx, leaf.edx)
    })
}

/// Reread unforgeable hardware evidence for the final ring-3 crossing.
pub(super) fn shared_kernel_mappings_are_safe() -> bool {
    shared_kernel_mapping_reason(&speculation_posture()).is_some()
}

/// One `cpuid` answer.
#[derive(Debug, Clone, Copy, Default)]
struct CpuidResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

/// The highest sub-leaf `cpuid` will answer for `leaf`'s family.
///
/// Both this and [`cpuid`] are safe calls: `cpuid` is architectural on x86_64,
/// so `core` exposes it without an obligation. A leaf the CPU does not
/// implement answers with the highest one it does, which is why the callers
/// check the range first rather than trusting the answer.
fn max_leaf(leaf: u32) -> u32 {
    __get_cpuid_max(leaf).0
}

/// Ask the CPU about itself.
fn cpuid(leaf: u32, sub_leaf: u32) -> CpuidResult {
    let result = __cpuid_count(leaf, sub_leaf);
    CpuidResult {
        eax: result.eax,
        ebx: result.ebx,
        ecx: result.ecx,
        edx: result.edx,
    }
}

fn read_cr0() -> u64 {
    let value: u64;
    // SAFETY: reading a control register in ring 0 has no preconditions and no
    // effects.
    unsafe {
        asm!(
            "mov {value}, cr0",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn write_cr0(value: u64) {
    // SAFETY: the only bits this crate changes are `MP`, `EM` and `WP`. The
    // first two govern whether SSE instructions fault; `WP` makes supervisor
    // writes obey permissions the page tables already state.
    unsafe {
        asm!(
            "mov cr0, {value}",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}

fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: reading a control register in ring 0 has no preconditions and no
    // effects.
    unsafe {
        asm!(
            "mov {value}, cr4",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn page_table_context_ids_are_enabled() -> bool {
    read_cr4() & CR4_PCIDE != 0
}

/// Whether page-table reloads flush every non-global translation.
#[cfg(feature = "selftest")]
pub fn page_table_context_ids_are_disabled_for_test() -> bool {
    !page_table_context_ids_are_enabled()
}

/// A reversible change to the page attribute table.
#[derive(Debug)]
#[must_use = "dropping the transition restores the previous PAT entry"]
pub struct WriteCombining {
    previous: u8,
    restore: bool,
}

impl WriteCombining {
    /// Keep the new entry after the page tables selecting it are active.
    #[must_use = "a refused PAT commit restores the previous entry on drop"]
    pub fn commit(mut self) -> bool {
        let enabled = write_combining_is_enabled();
        if enabled {
            self.restore = false;
        }
        enabled
    }
}

/// The page attribute table, `IA32_PAT`.
const PAT: u32 = 0x277;

impl Drop for WriteCombining {
    fn drop(&mut self) {
        if !self.restore {
            return;
        }
        let current = msr::read(PAT);
        let restored = (current & !(0xff << 8)) | (u64::from(self.previous) << 8);
        // SAFETY: only PAT entry 1 is restored to the byte read from it before
        // this transition. All other entries retain their current values.
        unsafe { msr::write(PAT, restored) };
        reload_page_table();
    }
}

/// Give page tables a write-combining memory type to select.
///
/// Only the unused write-through entry changes. Call before activating tables
/// that select it; activation's `CR3` load supplies the required flush.
pub fn enable_write_combining() -> Option<WriteCombining> {
    /// `cpuid` leaf 1, `EDX` bit 16.
    const SUPPORTED: u32 = 1 << 16;

    if cpuid(1, 0).edx & SUPPORTED == 0 {
        return None;
    }

    let current = msr::read(PAT);
    let previous = ((current >> 8) & 0xff) as u8;
    let updated = (current & !(0xff << 8)) | (1 << 8);
    // SAFETY: only entry 1 changes, to the architectural WC encoding. The
    // transition restores this exact byte unless the caller commits it.
    unsafe { msr::write(PAT, updated) };
    reload_page_table();
    Some(WriteCombining {
        previous,
        restore: true,
    })
}

/// Reload the current root so a PAT change cannot leave cached translations.
fn reload_page_table() {
    let root = page_table_base();
    // SAFETY: `root` is the page table the processor is executing through now,
    // so reloading it preserves the live mappings and flushes their non-global
    // translations.
    unsafe { set_page_table_base(root) };
}

/// Whether the page attribute table's second entry is write-combining.
///
/// Read back rather than remembered: a hypervisor may decline the write, and
/// the whole point of the entry is that a mapping selecting it behaves
/// differently.
pub fn write_combining_is_enabled() -> bool {
    /// Encoding of write-combining in a PAT entry.
    const WRITE_COMBINING: u64 = 0x01;

    (msr::read(PAT) >> 8) & 0xFF == WRITE_COMBINING
}

/// The physical address of the page table the CPU is walking.
///
/// The low twelve bits of `CR3` hold cache-control flags, not address, so they
/// are masked off — a caller comparing this against a table's address would
/// otherwise never see a match.
pub fn page_table_base() -> u64 {
    /// `CR3` bits 51:12. The rest are flags or reserved.
    const ADDRESS: u64 = 0x000f_ffff_ffff_f000;

    let value: u64;
    // SAFETY: reading a control register in ring 0 has no preconditions and no
    // effects.
    unsafe {
        asm!(
            "mov {value}, cr3",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value & ADDRESS
}

#[cfg(feature = "selftest")]
static PAGE_INVALIDATIONS: AtomicUsize = AtomicUsize::new(0);

/// Drop any cached translation for `address` from this CPU's TLB.
///
/// Removing a cached entry can only make the CPU reread its page tables;
/// [`CanonicalAddress`] excludes the operands that raise `#GP`.
pub fn invalidate_page(address: CanonicalAddress) {
    // SAFETY: construction of `CanonicalAddress` proves the instruction's
    // operand is canonical. `invlpg` dereferences no memory and only evicts
    // this CPU's cached translation.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) address.get(), options(nostack, preserves_flags))
    };
    #[cfg(feature = "selftest")]
    PAGE_INVALIDATIONS.fetch_add(1, Ordering::Relaxed);
}

/// Number of `invlpg` instructions issued since boot.
#[cfg(feature = "selftest")]
pub fn page_invalidations_for_test() -> usize {
    PAGE_INVALIDATIONS.load(Ordering::Relaxed)
}

/// Write the cache line holding `address` back to memory and drop it.
///
/// For a device that reads memory without snooping the processor caches, so a
/// table entry the CPU wrote reaches the bytes the device will read (D-358).
///
/// # Safety
///
/// `address` must be mapped: `clflush` references its operand and faults on an
/// unmapped one.
pub(crate) unsafe fn flush_cache_line(address: *const u8) {
    // SAFETY: the caller promises a mapped operand, and the instruction writes
    // no register; it only moves one line out of this CPU's caches.
    unsafe { asm!("clflush [{}]", in(reg) address, options(nostack, preserves_flags)) };
}

/// Point the CPU at a different top-level page table.
///
/// # Safety
///
/// `address` must be the physical address of a correctly formed, 4 KiB-aligned
/// PML4 that stays live for as long as it is loaded, and which maps — at the
/// addresses they already occupy — every page this CPU is about to touch:
/// the code executing this call, its stack, and anything either reads or
/// writes before the next switch. A table missing any of them faults on an
/// instruction that has no handler mapped, which the CPU reports as a triple
/// fault and the machine as a reset.
///
/// # Panics
///
/// If `CR4.PCIDE` is set. This kernel uses a `CR3` reload to flush inactive
/// roots and therefore cannot preserve tagged translations.
pub(super) unsafe fn set_page_table_base(address: u64) {
    assert!(
        !page_table_context_ids_are_enabled(),
        "page-table switching requires CR4.PCIDE to remain clear"
    );
    // SAFETY: the caller guarantees the address names a page table that maps
    // everything this CPU is standing on, which is the entire precondition for
    // the write. PCID is disabled, so writing `CR3` flushes every non-global
    // translation from the previous use of either root.
    unsafe {
        asm!(
            "mov cr3, {address}",
            address = in(reg) address,
            options(nostack, preserves_flags),
        );
    }
}

fn add_cr4(additions: Cr4Additions) {
    let value = additions.apply(read_cr4());
    // SAFETY: `Cr4Additions` can only add the reviewed extended-state and
    // protection bits. The asm is a memory barrier because those bits change
    // which memory accesses and instruction fetches the CPU permits.
    unsafe {
        asm!(
            "mov cr4, {value}",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}
