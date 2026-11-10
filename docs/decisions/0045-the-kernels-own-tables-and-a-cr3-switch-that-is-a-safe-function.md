# D-45 The kernel's own tables, and a `CR3` switch that is a safe function

Status: Accepted, 2026-08-22. Amended by D-76; its one-way mapping retired by D-86.

**Context.** `007-memory.md` names the switch to the kernel's own page tables
as the phase's one irreversible moment: *"the switch is the one moment a
mistake reports as a triple fault rather than a page fault"*. At that
instruction the kernel executes from, logs through and stands on mappings
the bootloader provided. A table missing any of them faults on an
instruction fetch with no handler mapped, and the machine resets with
nothing on the wire.

It is also the sharpest test of what `005-code-conventions.md` demands of
`wost`: every abstraction must make the unsafe operation underneath it
*checkable*, not merely wrapped.

**Options for the switch.** (a) `unsafe fn activate` with a `# Safety`
section listing what the caller must have mapped; (b) the same, plus a
`debug_assert`; (c) a **safe** `activate` that walks the new tables for the
addresses whose absence would be fatal, and refuses rather than loading.

**Decision.** (c). Before `CR3` is written, `activate` translates *in the new
tables* its own code address, the current stack pointer, and a page of
headroom below the stack for the first push after the switch. Code must come
back executable, stack writable. A table failing either check is refused
with the old one still loaded: a `Result` on the serial log, not a reset.

`activate` takes the space **by value**, which makes `AddressSpace::map`
safe: after the switch the caller has nothing left to map into, so nothing
can add a mapping to a table the CPU is walking. A second call is refused
with `AlreadyActive`, asked before the walks, so the error names the actual
problem and not whatever mapping the second caller's empty table lacked.

**The direct map's contract.** `008-research-notes-2.md` §3.3 proposed
*usable RAM only, device holes excluded*. Half of that survived contact.

Excluding device holes is right and is kept. `Reserved` is where MMIO lives,
and mapping it here would give a device the direct map's cache type, not the
one it needs. Devices are mapped on purpose, through `MmioRegion`, and are
never reachable by accident.

*Usable RAM only* was wrong, and would have triple-faulted on the first
switch: the kernel runs on a stack the bootloader placed in
`BootloaderReclaimable` and logs through a `Framebuffer` region. The direct
map therefore covers everything that **is memory** (`Usable`,
`BootloaderReclaimable`, `KernelAndModules`, `Framebuffer`, `AcpiReclaimable`,
`AcpiNvs`) and nothing else. It sits at the bootloader's offset, so every
pointer the kernel holds stays valid across the switch without a sweep.

2 MiB pages wherever the addresses and remaining length permit: 511 MiB of
direct map costs a few hundred kilobytes of tables, not a megabyte, and one
TLB entry covers 2 MiB, not 4 KiB.

**W^X, the D-12 debt owed since boot.** The linker script gained six symbols
bounding the three protection classes, so the image is mapped by *what each
part may do*, not as one range. Text is read-only and executable, rodata
read-only and not, data writable and not; with no fourth combination, that
is W^X. Intermediate entries are permissive on purpose (`PRESENT | WRITABLE`,
never `NO_EXECUTE`), because permissions intersect down a walk. A
restrictive parent would silently cap every leaf below it, so enforcement is
at the leaves, where the check reads it back.

**Rationale.** The check is cheap (two walks of four levels) and makes the
most dangerous operation in the kernel a verified obligation, not a
documented one. `005` says a bound stated in prose is not a bound. For `CR3`
the prose would have been three paragraphs of `# Safety` no caller could
confirm it had met.

**Consequences.** Four on-machine checks. `the loaded page table is ours`
requires that the switch was attempted and that `CR3` holds the root we
recorded; a kernel that noted success and kept the old tables passes only
the first half. `the kernel image obeys write-xor-execute` walks the live
tables at three points per section, since a run of large pages can be right
at the edges and wrong between them. Mapping `.data` executable once turned
the harness red. `the direct map is writable and never executable` covers
the largest writable mapping in the kernel.
`a second address space is refused` is the property that makes `map` safe.

The checks caught two bugs, both in the checks. The W^X probe computed its
midpoint as `(start + last) / 2`, which overflows on two higher-half
addresses; it now works from offsets. And `activate` validated before
testing the latch, so the second-switch check saw `CodeUnmapped`, not
`AlreadyActive`: correct behaviour, useless diagnosis.

Recorded for SMP: unmapping anything from a direct map needs a TLB
shootdown, an IPI to every CPU. That is free while single-core, and cheaper
to write down now than to discover in Phase 12.
