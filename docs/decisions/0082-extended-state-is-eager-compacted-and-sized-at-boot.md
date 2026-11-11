# D-82 Extended state is eager, compacted, and sized at boot

Status: Accepted, 2026-08-24.

**Decision.** Eager, compacted, boot-sized, out-of-line, CPUID-derived, with the
arithmetic in a `libs/` crate. This discharges D-29's deferral of XSAVE until
there was a scheduler. (**`libs/xstate` was never written.** Its
compacted-offset table turned out to be four instructions and a CPUID read in
`wost/src/arch/x86_64/xstate.rs`, too little to justify a crate. Two lines
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

**Rationale.** Lazy FPU restore is CVE-2018-3665; recording it stops anyone
optimising it back in. `XSAVEOPT`'s modified-optimisation is a correctness
hazard. `XSAVES` is preferred even with no supervisor state today because
CET's shadow-stack pointers live in supervisor xstate. Every enabled `XCR0`
bit is per-thread memory forever. **APX is state component 19 but lies at
offset 960** in the non-compacted format, so offsets do not increase with
index. `+soft-float` keeps trap entry off extended state, and it is a gate
check because nobody would notice losing a build flag.

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
