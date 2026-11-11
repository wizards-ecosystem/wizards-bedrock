# D-177 A check that names an address names a machine

Status: Accepted, 2026-08-25.

**Decision.** Four paging checks (a boot mapping cannot claim ownership, a user
space cannot reach the kernel half, a user mapping cannot widen a supervisor
table, a differing remap is refused) called `BootRegion::describe(PAGE_SIZE,
PAGE_SIZE)`. VMware's firmware does not describe physical `0x1000` as memory,
so on VMware each fell out of its setup's `else` arm and returned `false`,
reported identically to the property being violated.

**A caller that needs a described range asks for one.**
`MemoryMap::first_memory_run(len)` returns the first page-aligned run of that
many bytes the map covers with memory, never physical zero, and the four probes
cut their pages from it. On QEMU the answer is the page they already named, so
the existing boots are unchanged by construction.

**A precondition that can fail gets its own line.** *the map offers a run to
probe* is checked before the four that depend on it, so a machine that cannot
supply one says so once rather than reddening four unrelated properties.

**The rule:**

> **A check that hardcodes an input hardcodes the machine that supplied it.**
> D-165 says a check must reach the code it is about; this is how it stops —
> not by being deleted, but by having its setup quietly answer `None` somewhere
> the author never booted. Where the setup can fail, the failure is a separate
> check, because "the property is false" and "the check did not run" are
> different findings and the harness renders them the same colour.

**Rationale.** This is D-155's rule (choose inputs where the answers differ)
arriving from the other side: `0x1000` was chosen because it was there, and
being there is a fact about QEMU. The four properties stand between safe code
and a second owner of a frame the kernel is running on, and on the machine D-18
makes this project answer to none had been evaluated. **A second machine does
not re-run the checks, it discovers which of them were about the first
machine.**

**Consequences.** `wost::boot::MemoryMap` gains `first_memory_run`, and
`wost::mm` re-exports it through the map's lock, with no unsafe and no new lock.
The precondition check is broken by mutation 66.
