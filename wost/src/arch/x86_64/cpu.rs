use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, __get_cpuid_max};
use core::fmt;

use x86_cpuid::physical::{ADDRESS_SIZE_LEAF, PhysicalAddressWidth};
use x86_cpuid::{
    CpuidLeaf, Cr4Additions, ExecutionEnvironment, FrequencyHints, SPEC_CTRL_MSR,
    SharedKernelMappingReason, Speculation,
};
pub use x86_cpuid::{DmaHandoffPlatform, SeedSource};
use x86_paging::{CanonicalAddress, Entry};

use super::interrupt;
use super::msr;
use crate::mm::PhysicalAddress;

#[cfg(feature = "selftest")]
mod selftest;
#[cfg(feature = "selftest")]
pub use selftest::access_flag_is_set;

const EFER: u32 = 0xC000_0080;

const EFER_SYSCALL: u64 = 1 << 0;

const SYSENTER_CS: u32 = 0x174;

const PRED_CMD: u32 = 0x49;

const PRED_CMD_IBPB: u64 = 1 << 0;

const EFER_NO_EXECUTE: u64 = 1 << 11;

const CR0_EM: u64 = 1 << 2;

const CR0_NE: u64 = 1 << 5;

const CR0_WRITE_PROTECT: u64 = 1 << 16;

const CR4_TSD: u64 = 1 << 2;

const CR4_DE: u64 = 1 << 3;

const CR4_MCE: u64 = 1 << 6;

const CR4_PCE: u64 = 1 << 8;

const CR4_FSGSBASE: u64 = 1 << 16;

// A bootloader may leave these set; each would hand ring 3 a counter, a register or a breakpoint.
const CR4_CLEARED: u64 = CR4_TSD | CR4_DE | CR4_PCE | CR4_FSGSBASE;

pub(super) const FLAGS_RESERVED: u64 = 1 << 1;

const CR4_UMIP: u64 = 1 << 11;

const CR4_PCIDE: u64 = 1 << 17;

const CR4_SMEP: u64 = 1 << 20;

const CR4_SMAP: u64 = 1 << 21;

// A maskable interrupt cannot wake `hlt` while IF is clear.
pub fn halt() {
    // SAFETY: `hlt` in ring 0 waits for an interrupt with no precondition and cannot fail; it stays
    // a compiler memory barrier so state publication cannot move past the sleep.
    unsafe { asm!("hlt", options(nostack, preserves_flags)) };
}

pub fn halt_forever() -> ! {
    interrupt::disable_for_good();
    loop {
        halt();
    }
}

pub fn has_invariant_tsc() -> bool {
    const INVARIANT: u32 = 1 << 8;

    max_leaf(0x8000_0000) >= 0x8000_0007 && cpuid(0x8000_0007, 0).edx & INVARIANT != 0
}

pub fn frequency_hints() -> FrequencyHints {
    let maximum = max_leaf(0);
    let ratio = (maximum >= 0x15).then(|| {
        let leaf = cpuid(0x15, 0);
        (leaf.eax, leaf.ebx, leaf.ecx)
    });
    let base_mhz = (maximum >= 0x16).then(|| cpuid(0x16, 0).eax);
    FrequencyHints::from_leaves(ratio, base_mhz)
}

pub(crate) fn physical_address_width() -> Option<PhysicalAddressWidth> {
    let maximum = max_leaf(0x8000_0000);
    let eax = if maximum >= ADDRESS_SIZE_LEAF {
        cpuid(ADDRESS_SIZE_LEAF, 0).eax
    } else {
        0
    };
    PhysicalAddressWidth::from_leaf(maximum, eax)
}

// `lfence` keeps the non-serialising `rdtsc` inside the interval it measures.
pub fn timestamp() -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: `rdtsc` has no ring 0 precondition, since `CR4.TSD` gates ring 3 only; the `lfence`
    // before it orders execution and touches no state.
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

// Meaningful only inside a page-fault handler; other vectors leave `CR2` stale.
pub fn faulting_address() -> u64 {
    let value: u64;
    // SAFETY: reading a control register in ring 0 has no preconditions and no effects.
    unsafe {
        asm!(
            "mov {value}, cr2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Protections {
    pub write_protect: bool,
    pub smep: bool,
    pub smap: bool,
    pub umip: bool,
    pub nx: bool,
}

const SMEP_SUPPORTED: u32 = 1 << 7;

const SMAP_SUPPORTED: u32 = 1 << 20;

const UMIP_SUPPORTED: u32 = 1 << 2;

const NX_SUPPORTED: u32 = 1 << 20;

impl Protections {
    fn supported() -> Self {
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
            smep: structured.ebx & SMEP_SUPPORTED != 0,
            smap: structured.ebx & SMAP_SUPPORTED != 0,
            umip: structured.ecx & UMIP_SUPPORTED != 0,
            nx: extended.edx & NX_SUPPORTED != 0,
        }
    }

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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut separator = "";
        for (enabled, name) in [
            (self.write_protect, "wp"),
            (self.smep, "smep"),
            (self.smap, "smap"),
            (self.umip, "umip"),
            (self.nx, "nx"),
        ] {
            if enabled {
                formatter.write_str(separator)?;
                formatter.write_str(name)?;
                separator = " ";
            }
        }
        if separator.is_empty() {
            formatter.write_str("none")?;
        }
        Ok(())
    }
}

const CR4_OSXSAVE: u64 = 1 << 18;

pub(crate) fn extended_state_is_enabled() -> bool {
    read_cr4() & CR4_OSXSAVE != 0
}

// Only `xstate::init` may call this: SSE enabled before switches save it is shared with ring 3.
pub(super) fn enable_extended_state() {
    const CR0_MP: u64 = 1 << 1;

    write_cr0((read_cr0() | CR0_MP) & !CR0_EM);
    add_cr4(Cr4Additions::extended_state());
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum ControlStateError {
    Cr0,
    Cr4,
    Flags,
}

impl fmt::Display for ControlStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cr0 => "cr0 did not keep ne set and em clear",
            Self::Cr4 => "cr4 did not keep mce set and tsd, de, pce and fsgsbase clear",
            Self::Flags => "rflags did not read back as only its reserved bit",
        })
    }
}

impl core::error::Error for ControlStateError {}

// Every bit not named here stays as the bootloader set it, since paging-class bits are its choice.
pub fn establish_control_state() -> Result<(), ControlStateError> {
    assert!(
        !interrupt::are_enabled(),
        "control state is established before interrupts are accepted"
    );
    write_cr0((read_cr0() | CR0_NE) & !CR0_EM);
    let machine_check = if has_machine_check() { CR4_MCE } else { 0 };
    write_cr4((read_cr4() | machine_check) & !CR4_CLEARED);
    let flags = normalise_flags();

    let cr0 = read_cr0();
    if cr0 & CR0_NE == 0 || cr0 & CR0_EM != 0 {
        return Err(ControlStateError::Cr0);
    }
    let cr4 = read_cr4();
    if cr4 & machine_check != machine_check || cr4 & CR4_CLEARED != 0 {
        return Err(ControlStateError::Cr4);
    }
    if flags != FLAGS_RESERVED {
        return Err(ControlStateError::Flags);
    }
    Ok(())
}

fn has_machine_check() -> bool {
    const MCE: u32 = 1 << 7;

    cpuid(1, 0).edx & MCE != 0
}

fn normalise_flags() -> u64 {
    let flags: u64;
    // SAFETY: `popfq` loads only the reserved bit, clearing `AC`, `NT`, `IOPL`, `DF` and `TF` with
    // `IF` already clear; `pushfq` and `pop` then reuse the slot, so `rsp` ends as it started.
    unsafe {
        asm!(
            "push {reserved}",
            "popfq",
            "pushfq",
            "pop {flags}",
            reserved = const FLAGS_RESERVED,
            flags = out(reg) flags,
        );
    }
    flags
}

// These bits cause no undefined behaviour, only faults, so they wait for the interrupt table.
pub fn enable_protections() -> Protections {
    let supported = Protections::supported();

    write_cr0(read_cr0() | CR0_WRITE_PROTECT);

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
    // `AC` survives interrupt delivery and ring 3 may set it, so every SMAP entry must clear it.
    if Protections::enabled().smap {
        interrupt::arm_access_clear();
    }

    if supported.nx {
        // SAFETY: the only register named is `EFER`, and the only bit added to what it held is the
        // one `cpuid` just reported as supported.
        unsafe { msr::write(EFER, msr::read(EFER) | EFER_NO_EXECUTE) };
    }

    shut_the_fast_syscall_doors();
    Protections::enabled()
}

fn write_protection_is_enabled() -> bool {
    read_cr0() & CR0_WRITE_PROTECT != 0
}

fn shut_the_fast_syscall_doors() {
    // SAFETY: only `EFER` is named, and the only bit removed enables an instruction this kernel
    // never executes: no `syscall` or `sysret` appears in the tree.
    unsafe { msr::write(EFER, msr::read(EFER) & !EFER_SYSCALL) };
    // SAFETY: `IA32_SYSENTER_CS` names the ring-0 segment `sysenter` would enter through, and zero
    // is the architectural shut state: `sysenter` with a null selector raises `#GP` instead.
    unsafe { msr::write(SYSENTER_CS, 0) };
}

pub(crate) fn fast_syscall_doors_are_shut() -> bool {
    msr::read(EFER) & EFER_SYSCALL == 0 && msr::read(SYSENTER_CS) == 0
}

pub(super) fn has_local_apic() -> bool {
    const APIC: u32 = 1 << 9;

    cpuid(1, 0).edx & APIC != 0
}

pub(super) fn has_x2apic() -> bool {
    const X2APIC: u32 = 1 << 21;

    cpuid(1, 0).ecx & X2APIC != 0
}

pub fn has_buffer_clear() -> bool {
    const MD_CLEAR: u32 = 1 << 10;

    max_leaf(0) >= 7 && cpuid(7, 0).edx & MD_CLEAR != 0
}

// Holding one proves CPUID enumerated its instruction, so `read` cannot raise #UD.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeedReader(SeedSource);

const SEED_ATTEMPTS: u32 = 128;

impl SeedReader {
    pub fn detect() -> Option<Self> {
        let structured_ebx = if max_leaf(0) >= 7 { cpuid(7, 0).ebx } else { 0 };
        SeedSource::read(cpuid(1, 0).ecx, structured_ebx).map(Self)
    }

    pub const fn source(self) -> SeedSource {
        self.0
    }

    // A clear carry means the hardware had no value ready; it is retried a bounded number of times.
    pub fn read(self) -> Option<u64> {
        (0..SEED_ATTEMPTS).find_map(|_| {
            let word = match self.0 {
                SeedSource::Rdseed => rdseed(),
                SeedSource::Rdrand => rdrand(),
            };
            if word.is_none() {
                core::hint::spin_loop();
            }
            word
        })
    }
}

fn rdseed() -> Option<u64> {
    let value: u64;
    let ready: u8;
    // SAFETY: only `SeedReader::read` calls this, and a reader exists only where CPUID enumerated
    // `RDSEED`; the instruction writes one register and the flags and touches no memory.
    unsafe {
        asm!(
            "rdseed {value}",
            "setc {ready}",
            value = out(reg) value,
            ready = out(reg_byte) ready,
            options(nomem, nostack),
        );
    }
    (ready != 0).then_some(value)
}

fn rdrand() -> Option<u64> {
    let value: u64;
    let ready: u8;
    // SAFETY: only `SeedReader::read` calls this, and a reader exists only where CPUID enumerated
    // `RDRAND`; the instruction writes one register and the flags and touches no memory.
    unsafe {
        asm!(
            "rdrand {value}",
            "setc {ready}",
            value = out(reg) value,
            ready = out(reg_byte) ready,
            options(nomem, nostack),
        );
    }
    (ready != 0).then_some(value)
}

pub fn enable_speculation_controls() -> Speculation {
    let posture = speculation_posture();
    if !posture.wants_boot_ibrs() {
        return posture;
    }

    let wanted = posture.spec_ctrl_after(msr::read(SPEC_CTRL_MSR));
    // SAFETY: `IA32_SPEC_CTRL` selects only a speculation mode; `IBRS` under `IBRS_ALL` changes
    // branch prediction, never what any instruction means, and every firmware bit is preserved.
    unsafe { msr::write(SPEC_CTRL_MSR, wanted) };
    posture
}

fn speculation_posture() -> Speculation {
    // A nonexistent leaf may alias another answer and must not authorize the capabilities MSR read.
    let leaf = if max_leaf(0) >= 7 {
        cpuid(7, 0)
    } else {
        CpuidResult::default()
    };
    let sizes = if max_leaf(0x8000_0000) >= ADDRESS_SIZE_LEAF {
        cpuid(ADDRESS_SIZE_LEAF, 0)
    } else {
        CpuidResult::default()
    };
    Speculation::read(
        CpuidLeaf::new(leaf.eax, leaf.ebx, leaf.ecx, leaf.edx),
        CpuidLeaf::new(sizes.eax, sizes.ebx, sizes.ecx, sizes.edx),
        msr::read,
    )
}

pub(super) fn has_predictor_barrier() -> bool {
    speculation_posture().has_predictor_barrier()
}

// Executes only where `has_predictor_barrier` holds; elsewhere the write raises #GP.
pub(super) fn barrier_predictions() {
    // SAFETY: `IA32_PRED_CMD` is a write-only command register; `IBPB` discards indirect branch
    // predictions and changes nothing any instruction means, and the caller saw it enumerated.
    unsafe { msr::write(PRED_CMD, PRED_CMD_IBPB) };
}

// QEMU TCG's answer is trusted host configuration; elsewhere only `RDCL_NO` admits sharing.
pub fn shared_kernel_mapping_reason(
    speculation: &Speculation,
) -> Option<SharedKernelMappingReason> {
    x86_cpuid::shared_kernel_mapping_reason(speculation, execution_environment())
}

// A PCI requester can rewrite RAM but not the hypervisor leaf this identity is read from.
pub fn dma_handoff_platform() -> Option<DmaHandoffPlatform> {
    execution_environment().dma_handoff_platform()
}

fn execution_environment() -> ExecutionEnvironment {
    let leaf_1 = cpuid(1, 0);
    ExecutionEnvironment::read(leaf_1.ecx, || {
        let leaf = cpuid(0x4000_0000, 0);
        CpuidLeaf::new(leaf.eax, leaf.ebx, leaf.ecx, leaf.edx)
    })
}

pub(super) fn shared_kernel_mappings_are_safe() -> bool {
    shared_kernel_mapping_reason(&speculation_posture()).is_some()
}

#[derive(Clone, Copy, Default, Debug)]
struct CpuidResult {
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
}

// An unimplemented leaf answers as the highest implemented one, so callers check the range.
fn max_leaf(leaf: u32) -> u32 {
    __get_cpuid_max(leaf).0
}

fn cpuid(leaf: u32, sub_leaf: u32) -> CpuidResult {
    let answer = __cpuid_count(leaf, sub_leaf);
    CpuidResult {
        eax: answer.eax,
        ebx: answer.ebx,
        ecx: answer.ecx,
        edx: answer.edx,
    }
}

fn read_cr0() -> u64 {
    let value: u64;
    // SAFETY: reading a control register in ring 0 has no preconditions and no effects.
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
    // SAFETY: private to this module, whose callers pass the value just read with only `MP`, `NE`
    // or `WP` set and `EM` cleared; `PE`, `PG` and the cache bits are written back unchanged, so
    // paging and memory stay as compiled code assumes, and the changed bits only decide faults.
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
    // SAFETY: reading a control register in ring 0 has no preconditions and no effects.
    unsafe {
        asm!(
            "mov {value}, cr4",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

pub(super) fn page_table_context_ids_are_disabled() -> bool {
    read_cr4() & CR4_PCIDE == 0
}

#[derive(Debug)]
#[must_use = "dropping the transition restores the previous PAT entry"]
pub struct WriteCombining {
    previous: u8,
    restore: bool,
}

impl WriteCombining {
    #[must_use = "a refused PAT commit restores the previous entry on drop"]
    pub fn commit(mut self) -> bool {
        let enabled = write_combining_is_enabled();
        if enabled {
            self.restore = false;
        }
        enabled
    }
}

const PAT: u32 = 0x277;

const PAT_ENTRY_1_SHIFT: u32 = 8;

const PAT_ENTRY_1: u64 = 0xff << PAT_ENTRY_1_SHIFT;

const WRITE_COMBINING: u64 = 0x01;

impl Drop for WriteCombining {
    fn drop(&mut self) {
        if !self.restore {
            return;
        }
        let current = msr::read(PAT);
        let restored = (current & !PAT_ENTRY_1) | (u64::from(self.previous) << PAT_ENTRY_1_SHIFT);
        // SAFETY: only PAT entry 1 changes, back to the byte read from it before the transition;
        // the root reload that follows drops translations cached under the write-combining type.
        unsafe { msr::write(PAT, restored) };
        reload_page_table();
    }
}

// Call before activating the tables that select it; activation's `CR3` load is the flush.
pub fn enable_write_combining() -> Option<WriteCombining> {
    const SUPPORTED: u32 = 1 << 16;

    if cpuid(1, 0).edx & SUPPORTED == 0 {
        return None;
    }

    let current = msr::read(PAT);
    let previous = ((current & PAT_ENTRY_1) >> PAT_ENTRY_1_SHIFT) as u8;
    let updated = (current & !PAT_ENTRY_1) | (WRITE_COMBINING << PAT_ENTRY_1_SHIFT);
    // SAFETY: only PAT entry 1 changes, from the write-through type no live mapping selects to
    // write-combining, and the reload below drops translations cached under the old type.
    unsafe { msr::write(PAT, updated) };
    reload_page_table();
    Some(WriteCombining {
        previous,
        restore: true,
    })
}

fn reload_page_table() {
    let root = page_table_base();
    // SAFETY: `root` is the table the processor is executing through now, so reloading it keeps
    // the live mappings and flushes their non-global translations.
    unsafe { set_page_table_base(root) };
}

fn write_combining_is_enabled() -> bool {
    (msr::read(PAT) & PAT_ENTRY_1) >> PAT_ENTRY_1_SHIFT == WRITE_COMBINING
}

pub fn page_table_base() -> PhysicalAddress {
    let value: u64;
    // SAFETY: reading a control register in ring 0 has no preconditions and no effects.
    unsafe {
        asm!(
            "mov {value}, cr3",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    PhysicalAddress::from_entry(Entry::from_bits(value))
}

pub(crate) fn invalidate_page(address: CanonicalAddress) {
    // SAFETY: constructing `CanonicalAddress` proved the operand canonical, and `invlpg`
    // dereferences no memory and only evicts this CPU's cached translation.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) address.get(), options(nostack, preserves_flags));
    }
}

/// # Safety
///
/// `address` must be mapped: `clflush` references its operand and faults on an unmapped one.
pub(crate) unsafe fn flush_cache_line(address: *const u8) {
    // SAFETY: the caller promises a mapped operand, and the instruction writes no register; it only
    // moves one line out of this CPU's caches.
    unsafe { asm!("clflush [{}]", in(reg) address, options(nostack, preserves_flags)) };
}

/// # Safety
///
/// `address` must be the physical address of a correctly formed, 4 KiB-aligned PML4 that stays
/// live while loaded and maps, at the addresses they already occupy, every page this CPU is about
/// to touch: the code executing this call, its stack, and anything either reads or writes before
/// the next switch. A table missing any faults with no handler mapped, a triple fault and reset.
pub(super) unsafe fn set_page_table_base(address: PhysicalAddress) {
    assert!(
        page_table_context_ids_are_disabled(),
        "page-table switching requires CR4.PCIDE to remain clear"
    );
    // SAFETY: the caller guarantees the table maps everything this CPU stands on, the whole
    // precondition; with PCID disabled, the write flushes both roots' non-global translations.
    unsafe {
        asm!(
            "mov cr3, {address}",
            address = in(reg) address.get(),
            options(nostack, preserves_flags),
        );
    }
}

fn add_cr4(additions: Cr4Additions) {
    write_cr4(additions.apply(read_cr4()));
}

fn write_cr4(value: u64) {
    // SAFETY: private to this module, whose callers pass the value just read with only
    // `Cr4Additions` bits or a supported `MCE` added, or `TSD`, `DE`, `PCE` and `FSGSBASE`
    // cleared; paging-mode bits are written back unchanged and no memory changes, and the asm
    // stays a memory barrier because these bits change which accesses the processor permits.
    unsafe {
        asm!(
            "mov cr4, {value}",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}
