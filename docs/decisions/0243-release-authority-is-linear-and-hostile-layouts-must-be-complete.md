# D-243 Release authority is linear, and hostile layouts must be complete

Status: Accepted, 2026-08-30. Amends D-87, D-106, D-117, D-123, D-177 and D-241; closes PW-110, PW-111, PW-112, PW-211 and PW-308.

**Decision — privilege and lifetime.** Boot-evidence mappings reject `USER`,
as allocator-owned mappings already did, because a `BootRegion` proves only
that firmware described physical bytes, not that their contents may reach ring
3. Refusal is chosen over silently stripping the bit: attempting to publish
kernel bytes is an authority error, not a request for a nearby weaker mapping.

Release no longer accepts an address. Installing `Binding` returns a linear
`Bound` token which the process owner stores beside the `AddressSpace`.
Releasing a live task returns that same token unchanged; after retirement it
becomes `Released`. Construction that stops before installation consumes its
still-uninstalled `Binding` directly. Thus no safe caller can hold both release
evidence and something that can later install the same root.

The process transaction binds only after its other fallible objects exist and
returns the appropriate token through every thread-construction refusal. Its
region ledger is a tuple array plus an explicit used count, rather than an
array of tagged optional tuples, which keeps the debug construction record
small enough: the first token-carrying draft exceeded the guarded kernel stack
and double-faulted, so stack size is part of this boundary's correctness.

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

**Rationale.** The post-handoff audit treated every earlier `DONE` claim as an
unproved hypothesis and found five safe-boundary failures: `map_boot` accepted
`Flags::USER`; `task::release_address_space` took the root as a plain integer,
so a caller could mint `Released`, reclaim the tables and then install the
stale `Binding`; `covered_by` jumped over a reserved claim nested in a long
usable region; FAT32 accepted more data clusters than its FAT could index and
always read FAT 0; and XSAVE accepted an enabled component with no CPUID
sub-leaf and AVX without SSE.

**Alternatives.** A global registry of pending roots would duplicate ownership
already represented by Rust values and introduce synchronization into task
construction. Checking only the task table was the defect because a pending
binding is deliberately not there. Sorting the boot memory map would not
resolve contradictory overlapping claims; safe arbitration is independent of
input order. Deferring FAT and CPUID contradictions to later reads or hardware
faults would move hostile input past the boundary that has enough information
to reject it.

**Evidence and consequences.** Two debug QEMU mutations independently fail
only `a boot mapping cannot publish kernel memory` and `a bound space is not
reclaimed`. The memory-map byte oracle exhausts every query over two regions in
the bounded address space 0 through 6 and caught both the original nested hole
and its retained mutation. FAT32's host tests trace the exact selected table
sectors and kill four capacity/selector/offset mutations. XSAVE's presence and
dependency checks each fail under their retained mutation. The paging and task
source fingerprints were re-reviewed because the safe wrappers around those
blocks changed.
