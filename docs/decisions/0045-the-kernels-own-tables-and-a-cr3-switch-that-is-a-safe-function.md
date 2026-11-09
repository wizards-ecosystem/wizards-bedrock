# D-45 The kernel's own tables, and a `CR3` switch that is a safe function

Status: Accepted (2026-08-22) (amended by D-76; its one-way mapping retired by D-86)

**Context.** `007-memory.md` names the switch to the kernel's own page tables
as the phase's one irreversible moment: *"the switch is the one moment a
mistake reports as a triple fault rather than a page fault"*. At that
instruction the kernel is executing from, logging through, and standing on
mappings the bootloader provided. A table missing any of them faults on an
instruction fetch with no handler mapped, and the machine resets with nothing
on the wire.

That is also the sharpest possible test of what `005-code-conventions.md`
demands of `wost`: every abstraction must make the unsafe operation underneath
it *checkable*, not merely wrapped.

**Options for the switch.** (a) `unsafe fn activate` with a `# Safety` section
listing what the caller must have mapped; (b) the same, plus a `debug_assert`;
(c) a **safe** `activate` that walks the new tables for the addresses whose
absence would be fatal, and refuses rather than loading.

**Decision.** (c). Before `CR3` is written, `activate` translates — *in the new
tables* — the address of its own code and the current stack pointer, plus a
page of headroom below the stack for the first push after the switch. Code
must come back executable, stack writable. A table failing either check is
refused with the old one still loaded, so the failure is a `Result` on the
serial log instead of a reset.

`activate` takes the space **by value**. That is what makes
`AddressSpace::map` safe: nothing can add a mapping to a table the CPU is
already walking, because after the switch the caller has nothing left to map
into. A second call is refused with `AlreadyActive`, asked before the walks so
the error names the real problem rather than whichever mapping the second
caller's empty table happened to lack.

**The direct map's contract.** `008-research-notes-2.md` §3.3 proposed *usable
RAM only, device holes excluded*. Half of that survived contact.

Excluding device holes is right and is kept: `Reserved` is where MMIO lives,
and mapping it here would give a device the direct map's cache type instead of
the one it needs. Devices are mapped deliberately, through `MmioRegion`, and
are never reachable by accident.

*Usable RAM only* was wrong, and would have triple-faulted on the first switch.
The kernel is executing on a stack the bootloader placed in
`BootloaderReclaimable` and logging through a `Framebuffer` region. The direct
map therefore covers everything that **is memory** — `Usable`,
`BootloaderReclaimable`, `KernelAndModules`, `Framebuffer`, `AcpiReclaimable`,
`AcpiNvs` — and nothing that is not. It is built at the offset the bootloader
already used, which is what keeps every pointer the kernel is holding valid
across the switch rather than requiring a sweep to rewrite them.

2 MiB pages wherever the addresses and remaining length permit: 511 MiB of
direct map costs a few hundred kilobytes of tables instead of a megabyte, and
one TLB entry covers 2 MiB instead of 4 KiB.

**W^X, the D-12 debt owed since boot.** The linker script gained six symbols
bounding the three protection classes, so the image is mapped by *what each
part may do* rather than as one range: text read-only and executable, rodata
read-only and not, data writable and not. There is no fourth combination,
which is the whole of W^X. Intermediate entries are deliberately permissive
(`PRESENT | WRITABLE`, never `NO_EXECUTE`) because permissions intersect down a
walk — a restrictive parent would silently cap every leaf beneath it, so the
enforcement is at the leaves, which is where the check reads it back.

**Rationale.** The check is cheap — two walks of four levels — and it converts
the single most dangerous operation in the kernel from a documented obligation
into a verified one. `005` says a bound stated in prose is not a bound; for
`CR3` the prose would have been three paragraphs of `# Safety` that no caller
could confirm it had met.

**Consequences.** Four on-machine checks. `the loaded page table is ours`
requires both that the switch was attempted and that `CR3` holds the root we
recorded — a kernel that noted success and kept the old tables satisfies only
the first. `the kernel image obeys write-xor-execute` walks the live tables at
three points per section, because a run of large pages can be right at the
edges and wrong between them; broken once by mapping `.data` executable, and
the harness went red. `the direct map is writable and never executable` covers
the largest writable mapping in the kernel. `a second address space is refused`
is the property that makes `map` safe.

Two bugs the checks caught, both in the checks themselves. The W^X probe
computed its midpoint as `(start + last) / 2`, which overflows on a pair of
higher-half addresses; it now works from offsets. And `activate` validated
before testing the latch, so the second-switch check saw `CodeUnmapped` rather
than `AlreadyActive` — correct behaviour, useless diagnosis.

Recorded for SMP: a direct map means unmapping anything from it needs a TLB
shootdown — an IPI to every CPU. Free while single-core, and cheaper to write
down now than to discover in Phase 12.
