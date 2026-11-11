# D-33 x2APIC only, because xAPIC needs page tables

Status: Accepted, 2026-08-14.

**Decision.** x2APIC. `apic::init` requires `CPUID.01H:ECX.x2APIC`, switches
the processor into extended mode, and drives everything through
`rdmsr`/`wrmsr`. A processor without it is reported as
`ApicError::NoExtendedMode`, and the kernel runs on with interrupts off.

**Rationale.** xAPIC is a memory-mapped page, and Phase 2 owns no page
tables: Limine's direct map (base revision 3) covers only listed memory-map
regions, not device holes, and mapping the page ourselves needs Phase 3's
allocator. x2APIC is strictly less machinery: no direct map, no volatile
memory access, and no aliasing of a device page between a configuration
write and a trap handler's end-of-interrupt. Every Intel processor since
Nehalem and AMD one since Bulldozer has it, SMP wants it anyway, and this
project does not target older hardware.

**Consequences.**

- Two writes to `IA32_APIC_BASE`, in order. There is no transition from
  disabled straight to extended mode, and setting both bits at once faults.
  `Base::x2apic` encodes that, and both writes are read back.
- Registers keep their xAPIC offset names, as in SDM table 11-1, and
  `Register::msr` derives `0x800 + offset / 16`. Both table columns are
  transcribed into the host tests.
- In this mode the APIC identifier is the whole register, not its top byte.
