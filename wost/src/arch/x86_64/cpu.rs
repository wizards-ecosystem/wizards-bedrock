//! Direct CPU control: halting, feature discovery, and the protection bits.
//!
//! The interrupt flag lives in [`interrupt`] instead, next to
//! the table whose existence is the precondition for setting it.

use core::arch::asm;
use core::arch::x86_64::{__cpuid_count, __get_cpuid_max};
use core::fmt;

use super::interrupt;
use super::msr;

/// Extended feature enable register.
const EFER: u32 = 0xC000_0080;

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
    interrupt::disable();
    loop {
        halt();
    }
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

    Protections::enabled()
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
