# D-224 The boot memory map is complete or refused

Status: Accepted (2026-08-29); closes PW-108

**Context.** The boot handoff copied at most 128 physical-memory regions into
a fixed array and counted every later entry as dropped. Logging that loss did
not restore the evidence: the allocator selected usable frames, the direct map
selected memory, and firmware/device checks excluded ranges using only the
accepted prefix. Even where omission merely lost RAM on today's ordered maps,
the type still claimed to be the bootloader's memory map while proving less.

This copy runs before the heap exists. Dynamic storage would introduce a
dependency cycle, raising the constant would move rather than resolve the
boundary, and adjacent-region coalescing cannot bound an alternating valid map.

**Decision.** `libs/memory-map` owns the safe, allocation-free region types
and policy. Its constructor consumes the complete input stream, accepts one
through 128 regions, and returns a typed `Empty` or `TooMany { reported,
capacity }` error without exposing a copied prefix. The Limine adapter only
translates protocol kinds and maps that error into `HandoffError`; it no
longer pushes entries one by one. `wost::mm::Report` consequently has no
`dropped_regions` state because no initialized allocator can have one.

The fixed capacity remains 128. Refusal is deliberate degradation: the kernel
can report the handoff error while staying on the bootloader's mappings, but it
does not initialize its allocator or replace the page tables from incomplete
ownership evidence.

**Proof.** Host maps at 127 and 128 entries round-tripped every region in
order. At the exact bound, useful, firmware and reserved tail entries changed
the allocatable total, first direct-mapped run, complete coverage and
memory-only coverage independently. A 129-entry input returned `TooMany` with
both exact counts and no `MemoryMap` value. Replacing the overflow mark with
the former `continue` accepted the first 128 as an `Ok` map and reddened only
`tests::the_region_limit_is_all_or_nothing`; mutation 236 retains that defect.

**Alternatives.** Heap allocation was rejected because the map is what creates
the heap. A larger fixed array was rejected because every finite silent cap has
the same defect. Checked coalescing was rejected as the sole answer because it
cannot reduce alternating kinds and risks erasing boundaries whose kind is the
policy input; it could be added before the same refusal only if measured boot
maps ever require it.

**Consequences.** Memory-map policy becomes ordinary host-tested Rust and
`wost::boot` re-exports it, preserving callers while shrinking the unsafe-file
review surface. The machine-check count remains 212; the mutation registry is
236. PW-108 is closed.
