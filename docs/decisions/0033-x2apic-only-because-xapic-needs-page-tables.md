# D-33 x2APIC only, because xAPIC needs page tables

Status: Accepted, 2026-08-14.

**Context.** The local APIC has two encodings of the same registers. xAPIC is
a page of memory-mapped words at the physical address `IA32_APIC_BASE`
reports; x2APIC puts them in model-specific registers. Phase 2 needs one, and
Phase 2 owns no page tables.

**Decision.** x2APIC. `apic::init` requires `CPUID.01H:ECX.x2APIC`, switches
the processor into extended mode, and drives everything through
`rdmsr`/`wrmsr`. A processor without it is reported as
`ApicError::NoExtendedMode`, and the kernel runs on with interrupts off.

**Alternatives.**

- *xAPIC through Limine's direct map.* Implemented first, it page-faulted on
  the first register read. Under base revision 3 the direct map covers only
  listed memory-map regions, and a device hole is not one. (The fault printed
  `RIP` and the address instead of resetting, as slice 1 was built to do.)
- *Map the page ourselves.* The missing page-table levels need a frame, so an
  allocator. That is Phase 3; pulling it forward makes the CPU phase a memory
  phase.
- *Fall back to Limine base revision 2*, which maps the first 4 GiB. That
  downgrades the bootloader contract around a gap we are about to close.
- *Support both encodings.* The xAPIC half would be unreachable until
  Phase 3: speculative API with a hardware contract attached.

**Rationale.** x2APIC is strictly less machinery here: no direct map, no
volatile memory access, and no argument about aliasing a device page between
a configuration write and a trap handler's end-of-interrupt. Every Intel
processor since Nehalem and AMD one since Bulldozer has it, SMP wants it
anyway, and this project does not target older hardware.

**Consequences.**

- Two writes to `IA32_APIC_BASE`, in order. There is no transition from
  disabled straight to extended mode, and setting both bits at once faults.
  `Base::x2apic` encodes that, and both writes are read back.
- Registers keep their xAPIC offset names, as in SDM table 11-1, and
  `Register::msr` derives `0x800 + offset / 16`. Both table columns are
  transcribed into the host tests.
- In this mode the APIC identifier is the whole register, not its top byte.
- `BootInfo::direct_map` was written and then deleted. With no caller it is
  speculative API, and its 4 GiB bound was false besides. Phase 3 adds it
  back with the code that uses it.
