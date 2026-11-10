# D-31 A fault is terminal and reported; a breakpoint resumes

Status: Accepted, 2026-08-13. Amended by D-77; narrowed to kernel faults by D-102.

**Context.** With an IDT installed, something has to decide what a fault
means.

**Decision.** Vector 3 records its frame and returns. Every other vector
panics with its name, the error code and `RIP`/`CS`/`RFLAGS`/`RSP`, plus
`CR2` for a page fault, where the address is the whole report. Nothing is
recovered or retried.

**Rationale.** D-14: containment is supervised restart, and there is no
supervisor. A ring 0 fault is a kernel bug, and the useful response is the
report. User threads will add one branch ("was this ring 3?") in the one
place faults arrive.

**Consequences.** The handler logs nothing. A console lock taken in trap
context deadlocks against interrupted code holding it, the hazard
`001-boot.md` records against `SpinLock`. The terminal panic path tries
reporters without blocking instead of invalidating a live guard. Breakpoints
are recorded into atomics for the same reason. Recording is unconditional,
since a trap handler is no place for conditional compilation; its accessor
is behind the `selftest` feature.
