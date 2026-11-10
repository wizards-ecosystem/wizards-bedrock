# D-43 The frame allocator: mechanism in `wost`, policy in `libs/`

Status: Accepted, 2026-08-22.

**Context.** `007-memory.md` holds two open questions that are one. What
shape should the physical frame allocator take: bitmap, buddy, or staged?
And where does the D-13 boundary fall, given that table *mutation* is
`wost`'s but allocator *policy* is arithmetic?

`008-research-notes-2.md` §3.1 found the answer published. Asterinas calls
it **Safe Policy Injection**: the privileged framework keeps the *mechanism*
and the invariant, and the *policy* that chooses which frame lives outside
the trusted base in ordinary safe Rust. For frames, the invariant is the
conversion that makes "every frame handed out came from unused memory"
true. Asterinas has three such injection points; the frame allocator is one.

This project already has the pattern. D-39 moved the I/O port bitmap to
`libs/port-space` because it is arithmetic with no hardware behind it,
leaving `wost::port` the lock and the instruction. `005-code-conventions.md`
records the resulting rule: *a bound stated in prose is not a bound; where
the checkable form needs arithmetic, the arithmetic goes to a `libs/`
crate.*

**Options.** (a) Allocator wholly in `wost`, bitmap and all; (b) allocator
wholly in `kernel`, with `wost` exposing raw physical writes; (c) split it:
`wost` owns the frame handle and the physical access, a `libs/` crate owns
the search.

**Decision.** (c), on the `port-space` precedent.

`libs/frame-space` holds two bitmaps and knows only frame indices: `managed`
(usable RAM under our control) and `free` (which of those are not handed
out). Its capacity is a const parameter, so the kernel names its ceiling and
tests use spaces small enough for a stack. `wost::mm` wraps one in a
`SpinLock` and exports `Frame` and `Segment`, handles whose existence *is*
the claim and whose `Drop` releases it.

Shape: a bitmap with first-fit from a rotating hint, **order-aware from the
start**. `008` §3.1 argued for that, and this phase proves it: the direct map
wants 2 MiB pages, so Phase 3 needs `allocate_contiguous(512, 512)`, not
Phase 8. A buddy remains the upgrade, a change to one `libs/` file, because
the API speaks in frames and not in the structure that finds them.

**Rationale.** Two bitmaps is the decision inside the decision. One cannot
tell a double free from a frame that was never ours, and `free` is what a
`Drop` impl calls, the one place a bug is silent. With both, releasing a
foreign or already-free frame is a refusal the caller can see: what `005`
means by *checkable, not merely wrapped*.

Nothing computes the kernel image's extent to exclude it. Only
`MemoryKind::Usable` is inserted, so **the memory map itself** excludes the
image, the bootloader's structures and the ACPI tables, including the
megabyte of bitmap in the image's `.bss`. A range this code computed would
be a second source of truth for what the bootloader already knows.

The reclaimable kinds are *not* allocatable. Bootloader structures are still
being read when the allocator is built, and the ACPI tables are unparsed.
Reclaiming either is a later decision with a precondition attached.

**Consequences.** 20 host tests, including a run across two bitmap words and a
full drain-and-restore sweep that requires every frame conserved. Six
on-machine checks; one earned its place the hard way. The first "allocation
returns a zeroed frame" passed with the zeroing removed: it short-circuited on
an address comparison, and the hint means a released frame is rarely the next
one handed out. Breaking it once, which D-28 requires and which nearly did not
happen, exposed a check that could not fail. Its replacement, `dirty memory
can be zeroed, and seen to be`, writes `0xAA`, requires the kernel to *see*
it, zeroes, and requires the kernel to see that. Breaking `Frame::fill` turns
the harness red. The composition `allocate_zeroed = allocate + zero` rests on
that check plus three lines of code, and the selftest says so.

The ceiling is 16 GiB, at two bits of `.bss` per frame: one megabyte, which
the image does not carry because both bitmaps start zeroed. The release
kernel is 510 KiB on disk with a megabyte of `.bss` behind it, and D-40's ELF
verifier confirms `.bss` stayed `NOBITS`. Memory above the ceiling is
reported as ignored at boot rather than silently lost.

Absent on purpose: the untyped-memory split (`UFrame`/`USegment`) that `008`
§3.2 and D-44 adopt as a discipline. It arrives with the first caller that
can map a frame into a second address space, in Phase 5. Writing it now
would be a safety contract with nothing to check it against
(`005-code-conventions.md`).
