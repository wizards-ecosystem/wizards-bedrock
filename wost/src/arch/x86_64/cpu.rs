//! Direct CPU control: halting, feature discovery, and the protection bits.
//!
//! The interrupt flag lives in [`interrupt`] instead, next to
//! the table whose existence is the precondition for setting it.

use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, __get_cpuid_max};
use core::fmt;

use super::interrupt;
use super::msr;
use x86_cpuid::{SPEC_CTRL_MSR, Speculation};

/// Extended feature enable register.
const EFER: u32 = 0xC000_0080;

/// `EFER` bit: enable `syscall`/`sysret`.
///
/// This kernel does not use them — ring 3 enters through one interrupt gate at
/// DPL 3 (D-96) — and left set by the firmware it is a second door into ring 0
/// that the kernel never chose. `LSTAR` would be whatever was there.
const EFER_SYSCALL: u64 = 1 << 0;

/// `IA32_SYSENTER_CS`. Zero makes `sysenter` fault, which is the shut state.
const SYSENTER_CS: u32 = 0x174;

/// `EFER` bit: honour the no-execute page bit rather than treating it as
/// reserved.
const EFER_NO_EXECUTE: u64 = 1 << 11;

/// `CR4` bit: user mode instruction prevention.
const CR4_UMIP: u64 = 1 << 11;

/// `CR4` bit: supervisor mode execution prevention.
const CR4_SMEP: u64 = 1 << 20;

/// `CR4` bit: supervisor mode access prevention.
const CR4_SMAP: u64 = 1 << 21;

/// Halt this CPU until the next interrupt.
///
/// Returns immediately if interrupts are disabled and one is already pending;
/// callers that must not proceed should loop, as [`halt_forever`] does.
pub fn halt() {
    // SAFETY: `hlt` in ring 0 stops the CPU until an interrupt arrives. It has
    // no memory effects and cannot fail.
    unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
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
/// `cpuid` leaf 0x8000_0007, `EDX` bit 8. Without it the counter still counts,
/// but it counts *cycles* — slower when the core throttles, stopped in the
/// deeper idle states — which is a performance counter, not a clock. Present
/// on everything since Nehalem and Bulldozer, which is the same generation
/// the x2APIC mode this kernel already requires arrived in.
pub fn has_invariant_tsc() -> bool {
    /// `cpuid` leaf 0x8000_0007, `EDX` bit 8.
    const INVARIANT: u32 = 1 << 8;

    max_leaf(0x8000_0000) >= 0x8000_0007 && cpuid(0x8000_0007, 0).edx & INVARIANT != 0
}

/// The timestamp counter: cycles since the processor started.
///
/// Fenced, because `rdtsc` is not a serialising instruction and the processor
/// is free to execute it out of order with the work it is meant to bracket —
/// which for a calibration window means measuring something other than the
/// window. `lfence` in front is the sequence both vendors document for
/// reading the counter as a clock.
///
/// `nomem` is deliberately absent for the same reason: it would let the
/// compiler move the read across memory operations, undoing at compile time
/// what the fence does at run time.
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
            (self.smep, "smep"),
            (self.smap, "smap"),
            (self.umip, "umip"),
            (self.nx, "nx"),
        ] {
            if enabled {
                f.write_str(separator)?;
                f.write_str(name)?;
                separator = " ";
            }
        }
        if separator.is_empty() {
            f.write_str("none")?;
        }
        Ok(())
    }
}

/// Whether `CR4.OSXSAVE` is set, which is what makes `XGETBV`, `XSETBV` and
/// the save instructions legal.
///
/// `CR4.OSXSAVE`: the bit that makes `XSAVE` and `XGETBV` legal at all.
///
/// One definition for the two functions that name it — the one that sets it
/// and the one that reads it back to say whether the write took. They were a
/// local each, with the same value and the same doc line (D-160).
const CR4_OSXSAVE: u64 = 1 << 18;

/// Read from the register rather than from a remembered flag: the one caller
/// is a check asking whether the write took, and a remembered flag would
/// answer with the memory of the write (D-115).
pub fn extended_state_is_enabled() -> bool {
    read_cr4() & CR4_OSXSAVE != 0
}

/// Turn on `CR4.OSXSAVE`, `OSFXSR` and `OSXMMEXCPT`, and clear `CR0.EM`.
///
/// Safe for the same reason [`enable_protections`] is: none of these bits can
/// produce undefined behaviour, and a processor that does not support one
/// faults, which is a trap the kernel reports.
///
/// After this, SSE instructions are legal — **including at CPL 3** — so it is
/// called only once every refusal in
/// [`xstate::init`](super::xstate::init) has been made. A path that enabled
/// and then refused would leave the register file shared between processes
/// with nothing saving it (D-115).
///
/// `pub(super)` for that reason: the ordering argument is about one call site,
/// and a `pub` front door let any safe caller reach the state D-115 fixed
/// inside `init` — which is the same defect one level out (D-123).
pub(super) fn enable_extended_state() {
    /// `CR4.OSFXSR`: without it every SSE instruction is a `#UD`, whatever
    /// `XCR0` says. Setting `OSXSAVE` alone bought a fault, not a register.
    const CR4_OSFXSR: u64 = 1 << 9;
    /// `CR4.OSXMMEXCPT`: SSE numeric errors arrive as `#XM` rather than as the
    /// `#UD` a processor raises when the kernel has not said it can handle
    /// them.
    const CR4_OSXMMEXCPT: u64 = 1 << 10;
    /// `CR0.MP`: monitor coprocessor, which the manual pairs with `EM` clear.
    const CR0_MONITOR: u64 = 1 << 1;
    /// `CR0.EM`: emulation. Set, every SSE instruction faults.
    const CR0_EMULATE: u64 = 1 << 2;

    write_cr0((read_cr0() | CR0_MONITOR) & !CR0_EMULATE);
    write_cr4(read_cr4() | CR4_OSFXSR | CR4_OSXMMEXCPT | CR4_OSXSAVE);
}

/// Enable every protection in [`Protections`] this CPU supports, and report
/// what is now on.
///
/// Safe because none of these bits can produce undefined behaviour: the worst
/// an unexpected one can do is fault, which is a trap the kernel reports. That
/// is why this is called after the interrupt table is installed and not before.
pub fn enable_protections() -> Protections {
    let supported = Protections::supported();

    let mut cr4 = read_cr4();
    if supported.smep {
        cr4 |= CR4_SMEP;
    }
    if supported.smap {
        cr4 |= CR4_SMAP;
    }
    if supported.umip {
        cr4 |= CR4_UMIP;
    }
    write_cr4(cr4);

    if supported.nx {
        // SAFETY: the only register named is `EFER`, and the only bit added to
        // what it already held is the one `cpuid` just reported as supported.
        unsafe { msr::write(EFER, msr::read(EFER) | EFER_NO_EXECUTE) };
    }

    shut_the_fast_syscall_doors();
    Protections::enabled()
}

/// Close `syscall` and `sysenter`, which this kernel does not use.
///
/// **The IDT is not the only door into ring 0.** `vector::SYSCALL`'s doc says
/// `0x80` is "the only gate in the table ring 3 may reach", and that is true of
/// the *table*: `syscall` and `sysenter` are instructions, not vectors. Neither
/// `EFER.SCE` nor `IA32_SYSENTER_CS` was ever read, cleared or asserted — the
/// boot preserved whatever the firmware left, because the `EFER` write ORs a
/// bit into what it read.
///
/// With either open, a ring-3 `syscall` transfers to ring 0 at `LSTAR` with no
/// stack switch, at an address this kernel never chose. Limine leaves `SCE`
/// clear and `SYSENTER_CS` is zero at reset, so nothing was reachable — which
/// is the "a claim nothing enforces" shape D-124 and D-126 are about, and the
/// reason this is a write rather than a comment (D-139).
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

/// Open the fast-syscall doors, so a check can watch them be shut again.
///
/// Nothing in this kernel wants them open. It exists because the check for
/// D-139 could not otherwise fail: Limine leaves `SCE` clear and
/// `SYSENTER_CS` is zero at reset, so `fast_syscall_doors_are_shut` answers
/// true whether or not the kernel ever wrote anything. Observing a state the
/// firmware already arranged is not a check of the code that arranges it.
///
/// Paired with [`shut_fast_syscall_doors`] and used in one place, immediately.
pub fn open_fast_syscall_doors_for_test() {
    // SAFETY: sets the bit that enables an instruction this kernel never
    // executes — no `syscall` or `sysret` appears in the tree — and the caller
    // shuts it again in the next statement. Ring 3 cannot run in between:
    // this is called from a kernel thread, and no program is entered until
    // `enter_user`, which refuses while the door is open.
    unsafe { msr::write(EFER, msr::read(EFER) | EFER_SYSCALL) };
}

/// Shut them again, for the same check.
pub fn shut_fast_syscall_doors() {
    shut_the_fast_syscall_doors();
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
///
/// Every x86_64 processor does. Checking is what makes the register access that
/// follows a reasoned step rather than an assumption, and costs one `cpuid`.
pub(super) fn has_local_apic() -> bool {
    /// `cpuid` leaf 1, `EDX` bit 9.
    const APIC: u32 = 1 << 9;

    cpuid(1, 0).edx & APIC != 0
}

/// Whether that controller has an x2APIC mode, where its registers are
/// model-specific registers rather than a page of memory.
///
/// Present on everything since Nehalem and Bulldozer. It is what lets this
/// kernel drive the controller before it can map anything.
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
pub fn access_check_is_off() -> bool {
    /// `EFLAGS.AC`.
    const AC: u64 = 1 << 18;

    let flags: u64;
    // SAFETY: `pushfq` and `pop` read the flags register into a general
    // register; neither touches memory the compiler owns beyond the stack slot
    // it is told about.
    unsafe {
        core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(nomem, preserves_flags))
    };
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
/// One MSR write at boot, and D-81 records why it happens here rather than
/// later: it is trivial now and awkward to retrofit into a mature entry path.
///
/// The decisions are `x86_cpuid`'s, host-tested against synthetic capability
/// words, and this keeps the two instructions. That split matters more here
/// than usual: QEMU's TCG does not emulate `IA32_ARCH_CAPABILITIES`, so on the
/// development loop this function always takes its earliest return and the
/// write below is never reached (D-91).
pub fn enable_speculation_controls() -> Speculation {
    // Leaf 7 is only meaningful where the processor reports it. Without this,
    // a machine whose highest basic leaf is below 7 answers with some other
    // leaf's registers, and a stray bit 29 sends the read below to an MSR that
    // does not exist — which is a `#GP`, not a wrong answer (D-94).
    let leaf_7_edx = if max_leaf(0) >= 7 { cpuid(7, 0).edx } else { 0 };
    let posture = Speculation::read(leaf_7_edx, msr::read);
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

/// One `cpuid` answer.
#[derive(Debug, Clone, Copy, Default)]
struct CpuidResult {
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
    // SAFETY: the only bits this crate changes are `MP` and `EM`, which govern
    // whether SSE instructions fault and nothing about memory or privilege.
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

/// A reversible change to the page attribute table.
#[derive(Debug)]
#[must_use = "dropping the transition restores the previous PAT entry"]
pub struct WriteCombining {
    previous: u8,
    restore: bool,
}

impl WriteCombining {
    /// Keep the new entry after the page tables selecting it are active.
    pub fn commit(mut self) -> bool {
        let enabled = write_combining_is_enabled();
        if enabled {
            self.restore = false;
        }
        enabled
    }
}

/// The page attribute table, `IA32_PAT`.
///
/// One definition. It was transcribed three times in this file — once in each
/// of the two public functions and once in `Drop` — so a typo in any of them
/// would have named a different MSR entirely, and nothing compared them
/// (D-124).
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
/// The page attribute table holds eight memory types, and a mapping picks one
/// with three of its bits. Only the second is changed here, from write-through
/// — which this kernel never asks for — to write-combining, so a caller that
/// sets `WRITE_THROUGH` and nothing else gets writes that coalesce in a fill
/// buffer instead of crossing the bus one at a time. Every other entry keeps
/// its architectural default, so no existing mapping changes meaning.
///
/// Reports whether the processor has a PAT at all. Safe for the same reason
/// [`enable_protections`] is: the worst an unexpected value can do here is
/// make a mapping slower or uncached, never unsound.
///
/// Call before the tables whose mappings select the entry are activated: the
/// manual asks for a TLB flush after repurposing a live PAT entry, and the
/// `CR3` load in `activate` is exactly that.
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
    // so reloading it preserves every live translation and flushes the TLB.
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
/// Drop any cached translation for `address` from this CPU's TLB.
///
/// Safe, and it is worth saying why, because most of this module is not: the
/// page tables are the truth and the TLB is a cache of them, so removing an
/// entry can only make the CPU re-read what is already there. The unsound
/// direction is the other one — *not* invalidating after a mapping is taken
/// away — which is why [`super::paging::Unmapped`] owns the frames until this
/// has run (D-88).
pub fn invalidate_page(address: u64) {
    // SAFETY: `invlpg` reads no memory through `address` and writes none; it
    // only discards a cached translation for it, which is sound for any value.
    unsafe { core::arch::asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags)) };
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
pub unsafe fn set_page_table_base(address: u64) {
    // SAFETY: the caller guarantees the address names a page table that maps
    // everything this CPU is standing on, which is the entire precondition for
    // the write. Writing `CR3` also flushes the non-global TLB entries, so no
    // stale translation survives the switch.
    unsafe {
        asm!(
            "mov cr3, {address}",
            address = in(reg) address,
            options(nostack, preserves_flags),
        );
    }
}

/// Write `CR4`.
///
/// Private, and it stays private: the register holds bits that switch paging
/// modes underneath the running kernel. The only value this module ever writes
/// is one it just read with protection bits added.
fn write_cr4(value: u64) {
    // SAFETY: `value` is the register's current contents with SMEP, SMAP or
    // UMIP set, each only if `cpuid` reported it. No bit that governs paging or
    // addressing is touched.
    unsafe {
        asm!(
            "mov cr4, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}
