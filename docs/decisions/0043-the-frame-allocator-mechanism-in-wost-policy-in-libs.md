# D-43 The frame allocator: mechanism in `wost`, policy in `libs/`

Status: Accepted, 2026-08-22.

**Decision.** Split the frame allocator, on the `port-space` precedent:
`wost` owns the frame handle and the physical access, a `libs/` crate owns
the search.

`libs/frame-space` holds two bitmaps and knows only frame indices: `managed`
(usable RAM under our control) and `free` (which of those are not handed
out). Its capacity is a const parameter, so the kernel names its ceiling and
tests use spaces small enough for a stack. `wost::mm` wraps one in a
`SpinLock` and exports `Frame` and `Segment`, handles whose existence *is*
the claim and whose `Drop` releases it.

Shape: a bitmap with first-fit from a rotating hint, **order-aware from the
start**, because the direct map wants 2 MiB pages and Phase 3 needs
`allocate_contiguous(512, 512)`. A buddy remains the upgrade, a change to
one `libs/` file, because the API speaks in frames and not in the structure
that finds them.

Only `MemoryKind::Usable` is inserted, so **the memory map itself** excludes
the image, the bootloader's structures and the ACPI tables; nothing computes
the image's extent. The reclaimable kinds are *not* allocatable: bootloader
structures are still being read when the allocator is built, and the ACPI
tables are unparsed. Reclaiming either is a later decision with a
precondition attached.

**Rationale.** This is Asterinas's Safe Policy Injection (`008` §3.1): the
privileged framework keeps the mechanism and the invariant, and the policy
lives outside the trusted base in safe Rust. Two bitmaps let a release of a
foreign or already-free frame be a refusal the caller can see, in the `Drop`
that would otherwise fail silently. A range this code computed would be a
second source of truth for what the bootloader already knows.

**Consequences.** The ceiling is 16 GiB, at two bits of `.bss` per frame:
one megabyte, which the image does not carry because both bitmaps start
zeroed. Memory above the ceiling is reported as ignored at boot rather than
silently lost.

Absent on purpose: the untyped-memory split (`UFrame`/`USegment`) that `008`
§3.2 and D-44 adopt as a discipline. It arrives with the first caller that
can map a frame into a second address space, in Phase 5. Writing it now
would be a safety contract with nothing to check it against
(`005-code-conventions.md`).
