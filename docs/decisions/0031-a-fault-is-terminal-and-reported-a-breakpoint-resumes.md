# D-31 A fault is terminal and reported; a breakpoint resumes

Status: Accepted (2026-08-13) (amended by D-77; narrowed to kernel faults by D-102)

**Context.** With an IDT installed, something has to decide what a fault means.

**Decision.** Vector 3 records its frame and returns. Every other vector panics
with the vector's name, the error code, and `RIP`/`CS`/`RFLAGS`/`RSP` — plus
`CR2` for a page fault, where the address is the whole of the report. Nothing is
recovered and nothing is retried.

**Rationale.** D-14: containment is supervised restart, and there is no
supervisor. A fault in ring 0 is a kernel bug, and the useful response is the
report. When user threads exist, this grows one branch — "was this ring 3?" —
in the one place faults arrive.

**Consequences.** The handler logs nothing. Taking a console lock in trap
context deadlocks against interrupted code that already holds it, which is the
hazard `001-boot.md` records against `SpinLock`; the terminal panic path tries
reporters without blocking instead of invalidating a live guard. Breakpoints
are recorded into atomics for the same reason. The recording is unconditional
— a trap handler is no place for conditional compilation — while the accessor
that reads it is behind the `selftest` feature.
