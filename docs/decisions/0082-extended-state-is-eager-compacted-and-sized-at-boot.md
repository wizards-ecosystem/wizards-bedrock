# D-82 Extended state is eager, compacted, and sized at boot

Status: Accepted, 2026-08-24.

**Context.** D-29 deferred XSAVE until there was a scheduler. Ring 3 arrives in
Phase 5, so the decision is due. It is not only a context-switch decision: the
saved size leaks into per-thread stack arithmetic, so it has to be settled
before the stack allocator is.

**Decision.** Eager, compacted, boot-sized, out-of-line, CPUID-derived, with
the arithmetic in a `libs/` crate. (**`libs/xstate` was never written.** The
compacted-offset table it was to hold turned out to be four instructions and a
CPUID read, which live in `wost/src/arch/x86_64/xstate.rs` — the arithmetic
that would have justified a crate was not there. Recorded rather than quietly
dropped, because three lines below still name it.)

- **Eager only. Never lazy.**
- **`XSAVES` → `XSAVEC` → `XSAVE`, and never `XSAVEOPT`.**
- **`XCR0` = x87 + SSE + AVX.** No AVX-512, no AMX.
- **`XstateBuf` out-of-line and growable**, 64-byte aligned, sized at boot from
  the mask — never a fixed array inside the thread struct.
- **Offsets are computed from `CPUID.0DH` into a table** in `libs/xstate` — a
  crate that was never written; see the note above,
  host-tested against synthetic CPUID snapshots. `wost` keeps the four
  instructions and the MSR writes.
- **`#NM` (vector 7) is reserved** for a future XFD-based allocator; nothing is
  built.
- **`+soft-float` stays, and the gate checks that it stays.**

**Rationale.** Lazy FPU restore is CVE-2018-3665 and Linux went eager in 2016;
recording it here is what stops someone re-optimising it back in. `XSAVEOPT`'s
modified-optimisation is a correctness hazard while the init-optimisation that
`XSAVEC`/`XSAVES` provide is the one that pays, and `XSAVES` is preferred even
with no supervisor state today because it shares the compacted format with
`XSAVEC` — and CET's shadow-stack pointers live in supervisor xstate, so
wanting CET later means wanting `XSAVES` now.

Every enabled `XCR0` bit is per-thread memory forever, and AMX's `XTILEDATA`
alone is 8 KiB today and up to 64 KiB architecturally. A fixed array inside the
thread struct is precisely what would make AMX or a future component impossible
to add.

The offset table is not defensive generality. **APX is state component 19 but
sits at offset 960** in the non-compacted format, in deprecated MPX's hole — a
higher index at a lower offset. Linux had to add a dynamic ordering table and
delete its monotonicity assertions in 6.16. A host test against a synthetic
CPUID snapshot with the APX case is the cheapest possible way to not repeat
that.

`+soft-float` is what makes trap entry never touch extended state; only a task
switch does. It is a gate check because it is a build flag nobody would notice
losing.

**Alternatives.** Lazy restore (rejected: CVE-2018-3665); `XSAVEOPT` (rejected:
modified-optimisation correctness hazard); enabling AVX-512 or AMX (rejected:
per-thread memory forever, for no caller); a fixed-size buffer in the thread
struct (rejected: forecloses growth, and the size is not known at compile
time); assuming component offsets increase with index (rejected: APX).

**Consequences.**

- `libs/xstate` was to be a new host-tested crate; the APX snapshot is one of its
  tests, and the break-test is widening `XCR0` by a bit and confirming the size
  arithmetic notices.
- The stack allocator in slice 3 must know the boot-computed size, and must
  also leave a second region slot per thread so CET's shadow stack is not
  foreclosed (`010` §10). Slices 3 and 5 are coupled by this number.
- D-29's deferral is discharged.
