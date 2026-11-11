# D-224 The boot memory map is complete or refused

Status: Accepted, 2026-08-29. Closes PW-108.

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

**Proof.** Host maps at 127 and 128 entries round-tripped every region in order.
At the exact bound, useful, firmware and reserved tail entries changed the
allocatable total, first direct-mapped run, complete coverage and memory-only
coverage independently. A 129-entry input returned `TooMany` with both exact
counts and no `MemoryMap` value. Mutation 236 retains the former `continue`,
which accepts the first 128 as an `Ok` map and reddens only
`tests::the_region_limit_is_all_or_nothing`.

**Rationale.** The handoff had copied at most 128 regions and counted the rest
as dropped, while the allocator, direct map and firmware/device checks acted on
the accepted prefix, so the type claimed to be the bootloader's memory map
while proving less.

**Alternatives.** Heap allocation was rejected because the map is what creates
the heap. A larger fixed array was rejected because every finite silent cap has
the same defect. Checked coalescing was rejected as the sole answer because it
cannot reduce alternating kinds and risks erasing boundaries whose kind is the
policy input; it could be added before the same refusal only if measured boot
maps ever require it.

**Consequences.** Memory-map policy becomes ordinary host-tested Rust and
`wost::boot` re-exports it, preserving callers while shrinking the unsafe-file
review surface. PW-108 is closed.
