# D-243 Release authority is linear, and hostile layouts must be complete

Status: Accepted, 2026-08-30. Amends D-87, D-106, D-117, D-123, D-177 and D-241; closes PW-110, PW-111, PW-112, PW-211 and PW-308.

**Context.** The post-handoff audit treated every earlier `DONE` claim as an
unproved hypothesis. It found five safe-boundary failures.

`AddressSpace::map_boot` stripped ownership but accepted `Flags::USER`. A
`BootRegion` proves only that firmware described physical bytes; it does not
prove that kernel or firmware contents may be published to ring 3.

Address-space release had a deeper authority split. `AddressSpace::bind`
minted a one-shot `Binding`, but `task::release_address_space` accepted the
root as a plain integer. A caller could mint `Released` before installing the
binding, reclaim the tables, and then install the stale binding through safe
APIs.

Three hostile descriptions also passed their preflights. A long usable memory
region let `covered_by` jump over a nested reserved claim. FAT32 accepted more
data clusters than its FAT could index and always read FAT 0. XSAVE accepted an
enabled component with no CPUID sub-leaf and AVX without SSE.

**Decision — privilege and lifetime.** Boot-evidence mappings reject `USER`,
as allocator-owned mappings already did. Refusal is chosen over silently
stripping the bit: attempting to publish kernel bytes is an authority error,
not a request for a nearby weaker mapping.

Release no longer accepts an address. Installing `Binding` returns a linear
`Bound` token which the process owner stores beside the `AddressSpace`.
Releasing a live task returns that same token unchanged; after retirement it
becomes `Released`. Construction that stops before installation consumes its
still-uninstalled `Binding` directly. Thus no safe caller can hold both release
evidence and something that can later install the same root.

The process transaction binds only after its other fallible objects exist and
returns the appropriate token through every thread-construction refusal. Its
region ledger is now a tuple array plus an explicit used count, rather than an
array of tagged optional tuples. Besides being the data's actual shape, this
makes the debug construction record smaller; the first token-carrying draft
exceeded the guarded kernel stack and double-faulted, so stack size is part of
this boundary's correctness.

**Decision — complete descriptions.** Memory coverage first arbitrates every
nonempty region intersecting the requested interval, then walks the union of
admitted regions. A forbidden nested claim therefore cannot be skipped by a
larger span's end.

FAT32 preflight requires room for the two reserved entries and every advertised
data cluster. `BPB_ExtFlags` selects FAT 0 when mirroring is enabled and the
named in-range copy otherwise; table addressing includes that copy's offset.

Compacted XSAVE preflight requires every enabled extended component to have a
present sub-leaf and enforces AVX's SSE dependency before any hardware write.
Contradictory virtual hardware is refused rather than delegated to `XSETBV` or
an undersized state area.

**Alternatives.** A global registry of pending roots would duplicate ownership
already represented by Rust values and introduce synchronization into task
construction. Checking only the task table was the defect because a pending
binding is deliberately not there. Sorting the boot memory map would not
resolve contradictory overlapping claims; safe arbitration is independent of
input order. Deferring FAT and CPUID contradictions to later reads or hardware
faults would move hostile input past the boundary that has enough information
to reject it.

**Evidence and consequences.** Two debug QEMU mutations independently failed
only `a boot mapping cannot publish kernel memory` and `a bound space is not
reclaimed`. The memory-map byte oracle exhausts every query over two regions in
the bounded address space 0 through 6 and caught both the original nested hole
and its retained mutation. FAT32's fifteen host tests trace the exact selected
table sectors and kill four capacity/selector/offset mutations. XSAVE's exact
presence and dependency checks each failed under their retained mutation. The
unsafe block and declaration counts did not change; the paging and task source
fingerprints were re-reviewed because the safe wrappers around those blocks
did.
