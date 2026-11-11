# D-45 The kernel's own tables, and a `CR3` switch that is a safe function

Status: Accepted, 2026-08-22. Amended by D-76; its one-way mapping retired by D-86.

**Decision.** `activate` is a **safe** function. Before `CR3` is written, it
translates *in the new tables* its own code address, the current stack
pointer, and a page of headroom below the stack for the first push after the
switch. Code must come back executable, stack writable. A table failing
either check is refused with the old one still loaded: a `Result` on the
serial log, not a reset.

`activate` takes the space **by value**, which makes `AddressSpace::map`
safe: after the switch the caller has nothing left to map into, so nothing
can add a mapping to a table the CPU is walking. A second call is refused
with `AlreadyActive`, asked before the walks, so the error names the actual
problem.

**The direct map's contract.** Device holes are excluded: `Reserved` is
where MMIO lives, and devices are mapped on purpose, through `MmioRegion`,
with the cache type they need, never reachable by accident. The direct map
covers everything that **is memory** (`Usable`, `BootloaderReclaimable`,
`KernelAndModules`, `Framebuffer`, `AcpiReclaimable`, `AcpiNvs`) and nothing
else, because the kernel runs on a stack in `BootloaderReclaimable` and logs
through a `Framebuffer` region. It sits at the bootloader's offset, so every
pointer the kernel holds stays valid across the switch without a sweep. It
uses 2 MiB pages wherever the addresses and remaining length permit.

**W^X, the D-12 debt owed since boot.** Six linker symbols bound the three
protection classes, so the image is mapped by *what each part may do*. Text
is read-only and executable, rodata read-only and not, data writable and
not; with no fourth combination, that is W^X. Intermediate entries are
permissive on purpose (`PRESENT | WRITABLE`, never `NO_EXECUTE`), because
permissions intersect down a walk; enforcement is at the leaves, where the
check reads it back.

**Rationale.** A table missing a needed mapping resets the machine with
nothing on the wire. The check is cheap (two walks of four levels) and makes
the most dangerous operation in the kernel a verified obligation, not a
`# Safety` section no caller could confirm it had met.

**Consequences.** Recorded for SMP: unmapping anything from a direct map
needs a TLB shootdown, an IPI to every CPU.
