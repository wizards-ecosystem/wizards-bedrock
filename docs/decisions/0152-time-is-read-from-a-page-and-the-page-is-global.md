# D-152 Time is read from a page, and the page is global

Status: Accepted, 2026-08-25. Amended by D-181.

**Context.** Phase 4 made the counter the clock (D-57). Reading it from ring 3
currently means a trap. Every OS that cares eventually publishes a read-only
page instead — Linux's vDSO `clock_gettime`, Zircon's vDSO — and this is one
step behind the work already done.

Two questions. Whether time is ambient at all, and if so, whether the page is
per-program or shared.

**Options for the second.** One shared read-only page, mapped into every
program. Or a page per program, which is what a future that wants to
*virtualise* time would need.

**Decision.** A read-only page, mapped into every program, **global**. It
carries the counter's zero, its frequency, and enough for a program to convert
a `rdtsc` into nanoseconds without trapping.

**Rationale.** `005` forbids the speculative API, and a per-program page is one:
nothing this OS has decided to want needs it. Virtualising time is a
container's problem, and this project has no containers and no plan for them;
adding the machinery now means a page per program, an accounting question, and
a second thing to keep in step, for a capability nobody has asked for.

**Time being ambient at all is the more interesting half, and it is deliberate
rather than incidental.** WASI makes clocks capabilities: a program that was
not given a clock cannot read one. Zircon treats monotonic time as ambient.
Both are defensible and this project should not arrive at either because a page
was easier than a door. The argument for ambient here: monotonic time carries
no authority — it names no object, grants no access, and a program that cannot
read it can still measure duration by spinning. What it would buy is
*determinism*, which is a replay and sandboxing property, and this OS has not
claimed it. Recorded so that a future that wants deterministic replay knows it
is giving up a page and not a principle.

**Consequences.**

- Under D-114's former taxonomy, `sys_clock_get` stopped being OWED and became
  a load. D-181 records the checked reader and unfinished fallback/realtime
  work as BODY=PARTIAL, REMAINDER=PARTIAL.
- The page is one more thing mapped into every program, and one more thing in
  the address-space layout — an entry in D-46's KASLR ledger.
- A wall clock is still absent. This publishes the monotonic counter;
  `CLOCK_REALTIME` needs an epoch and this kernel has never been told what time
  it is.

**Left owed.** The page's layout is a shared structure a program reads, so it
needs a version field for the reason the ring header has one (D-79), and the
same copy-out-then-validate discipline does *not* apply — the kernel is the
only writer. Both go in the charter.
