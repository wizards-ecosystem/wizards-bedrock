# D-152 Time is read from a page, and the page is global

Status: Accepted, 2026-08-25. Amended by D-181.

**Decision.** Monotonic time is ambient and is read from one read-only page,
mapped into every program, **global** rather than per program. It carries the
counter's zero (D-57's counter is the clock), its frequency, and enough for a
program to convert a `rdtsc` into nanoseconds without trapping.

**Rationale.** `005` forbids the speculative API, and a per-program page is one:
it exists only to virtualise time, which is a container's problem, and this
project has no containers and no plan for them. Ambient time is deliberate:
WASI makes clocks capabilities and Zircon treats monotonic time as ambient.
Monotonic time names no object and grants no access, and a program that cannot
read it can still measure duration by spinning. What a clock capability would
buy is determinism, a replay and sandboxing property this OS has not claimed; a
future that wants deterministic replay gives up a page, not a principle.

**Consequences.**

- Under D-114's former taxonomy, `sys_clock_get` stopped being OWED and became
  a load. D-181 records the checked reader and unfinished fallback/realtime
  work as BODY=PARTIAL, REMAINDER=PARTIAL.
- The page is one more thing mapped into every program and one more entry in
  D-46's KASLR ledger.
- A wall clock is still absent. `CLOCK_REALTIME` needs an epoch, and this
  kernel has never been told what time it is.
- The page layout is a shared structure a program reads, so it needs a version
  field for the reason the ring header has one (D-79). The copy-out-then-validate
  discipline does not apply, because the kernel is the only writer. Both go in
  the charter.
