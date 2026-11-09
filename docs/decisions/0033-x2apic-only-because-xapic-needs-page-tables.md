# D-33 x2APIC only, because xAPIC needs page tables

Status: Accepted, 2026-08-14.

**Context.** The local APIC has two encodings of the same registers. xAPIC is a
page of memory-mapped words at the physical address `IA32_APIC_BASE` reports;
x2APIC puts the same registers in model-specific registers. Phase 2 needs one
of them, and Phase 2 does not own any page tables.

**Decision.** x2APIC. `apic::init` requires `CPUID.01H:ECX.x2APIC`, switches the
processor into extended mode, and drives everything through `rdmsr`/`wrmsr`. A
processor without it is reported as `ApicError::NoExtendedMode` and the kernel
runs on with interrupts off.

**Alternatives.**

- *xAPIC through Limine's direct map.* Implemented first, and it page-faulted
  on the first register read. Under base revision 3 the direct map covers the
  regions the memory map lists, and a device hole is not one of them. (The
  fault printed `RIP` and a faulting address instead of resetting the machine,
  which is slice 1 doing exactly what it was built for.)
- *Map the page ourselves.* Needs a frame for the missing page-table levels,
  which needs an allocator. That is Phase 3, and pulling it forward would make
  the CPU phase a memory phase.
- *Fall back to Limine base revision 2*, which maps the first 4 GiB. Downgrading
  the bootloader contract to route around a gap we are about to close anyway.
- *Support both encodings.* The xAPIC half would be unreachable code until
  Phase 3 — speculative API with a hardware contract attached.

**Rationale.** x2APIC is strictly less machinery here, not more: no direct map,
no volatile memory access, and no argument needed about aliasing a device page
between a configuration write and an end-of-interrupt from a trap handler. It
has been in every Intel processor since Nehalem and every AMD one since
Bulldozer, it is what SMP will want regardless, and this project does not target
older hardware.

**Consequences.**

- Two writes to `IA32_APIC_BASE`, in order. The architecture has no transition
  from disabled straight to extended mode; setting both bits at once faults.
  `Base::x2apic` encodes that, and both writes are read back.
- Registers are still named by their xAPIC offset — that is how SDM table 11-1
  names them — and `Register::msr` derives `0x800 + offset / 16`. Both columns
  of the table are transcribed into the host tests.
- The APIC identifier is the whole register in this mode, not its top byte.
- `BootInfo::direct_map` was written and then deleted: with no caller left it is
  speculative API, and the 4 GiB bound it enforced was false besides. Phase 3
  adds it back with the code that can use it.
