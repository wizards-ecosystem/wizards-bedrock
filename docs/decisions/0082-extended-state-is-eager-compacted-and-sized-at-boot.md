# D-82 Extended state is eager, compacted, and sized at boot

Status: Accepted, 2026-08-24.

**Context.** D-29 deferred XSAVE until there was a scheduler. Ring 3 arrives in
Phase 5, so the decision is due. The saved size also leaks into per-thread stack
arithmetic, so it must be settled before the stack allocator.

**Decision.** Eager, compacted, boot-sized, out-of-line, CPUID-derived, with the
arithmetic in a `libs/` crate. (**`libs/xstate` was never written.** Its
compacted-offset table turned out to be four instructions and a CPUID read in
`wost/src/arch/x86_64/xstate.rs`, too little to justify a crate. Three lines
below still name it.)

- **Eager only. Never lazy.**
- **`XSAVES` → `XSAVEC` → `XSAVE`, and never `XSAVEOPT`.**
- **`XCR0` = x87 + SSE + AVX.** No AVX-512, no AMX.
- **`XstateBuf` out-of-line and growable**, 64-byte aligned, sized at boot from
  the mask, never a fixed array inside the thread struct.
- **Offsets are computed from `CPUID.0DH` into a table** in `libs/xstate` (never
  written; see above), host-tested against synthetic CPUID snapshots. `wost`
  keeps the four instructions and the MSR writes.
- **`#NM` (vector 7) is reserved** for a future XFD-based allocator; nothing is
  built.
- **`+soft-float` stays, and the gate checks that it stays.**

**Rationale.** Lazy FPU restore is CVE-2018-3665, and Linux went eager in 2016.
Recording it stops anyone optimising it back in. `XSAVEOPT`'s
modified-optimisation is a correctness hazard; the init-optimisation of
`XSAVEC`/`XSAVES` is the one that pays. `XSAVES` is preferred even with no
supervisor state today: it shares the compacted format with `XSAVEC`, and CET's
shadow-stack pointers live in supervisor xstate. Wanting CET later means wanting
`XSAVES` now.

Every enabled `XCR0` bit is per-thread memory forever: AMX's `XTILEDATA` alone
is 8 KiB today and up to 64 KiB architecturally. A fixed array inside the thread
struct would make AMX or a future component impossible to add.

The offset table covers a case Linux hit. **APX is state component 19 but lies
at offset 960** in the non-compacted format, in deprecated MPX's hole: a higher
index at a lower offset. Linux had to add a dynamic ordering table and delete
its monotonicity assertions in 6.16. A host test against a synthetic CPUID
snapshot with the APX case is the cheapest way to avoid that.

`+soft-float` keeps trap entry off extended state; only a task switch touches
it. It is a gate check because nobody would notice losing a build flag.

**Alternatives.** Lazy restore (rejected: CVE-2018-3665). `XSAVEOPT` (rejected:
modified-optimisation correctness hazard). Enabling AVX-512 or AMX (rejected:
per-thread memory forever, for no caller). A fixed-size buffer in the thread
struct (rejected: forecloses growth, and the size is unknown at compile time).
Assuming component offsets increase with index (rejected: APX).

**Consequences.**

- `libs/xstate` was to be a new host-tested crate with the APX snapshot among
  its tests. The break-test widens `XCR0` by a bit and confirms the size
  arithmetic notices.
- The stack allocator in slice 3 must know the boot-computed size and leave a
  second region slot per thread so CET's shadow stack stays open (`010` §10).
  This number couples slices 3 and 5.
- D-29's deferral is discharged.
