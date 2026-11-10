# D-53 Scheduling policy is arithmetic in `libs/eevdf`, behind a seam with a fallback

Status: Accepted, 2026-08-22.

**Context.** D-15 chose EEVDF and wanted pluggable policy as "a cheap future
door, not a v1 feature". Then `008-research-notes-2.md` §4.4 took sched_ext's
lesson as an interface constraint (policy *separable and observable, with a
fallback path*) and flagged the lag rule as easy to omit.

**Decision.** The policy is `libs/eevdf`, pure functions over plain data.
Virtual time wraps and compares by signed difference; charging is scaled by
weight; a weighted average is the fair-share point. Eligibility is
non-negative lag, the pick is the earliest eligible deadline, and placement
clamps lag to one slice either way. Thirteen host tests include a
simulation that drives the policy as the kernel does and asserts a 1:2
weight ratio lands at 1.9 to 2.1. A sleeper test asserts that the clamp
stops a returning thread running unopposed.

The kernel's `thread.rs` is the mechanism side: it keeps an `Entity` per
thread, feeds the runnable ones to the policy, and acts on the answer.
Candidates rotate from just after the running thread, so equal deadlines take
turns instead of the lowest index winning every tie. The policy's tests say it
cannot answer "nobody" when everyone was considered. If it ever does, the
kernel takes the next runnable slot round-robin and counts it, and a
self-check requires the count to be zero. That is the fallback in its smallest
form: not a second policy, but a way to keep the machine up and the
inconsistency visible.

**Rationale.** D-30's pattern exists for arithmetic that decides who runs;
on the host it runs a thousand steps in a millisecond. Separability costs
nothing, since the seam is one function type, and observability is one
counter.

**Consequences.**

- Virtual time carries a `RESOLUTION` of 1024 units per unit of real time. The
  fairness simulation found the heavy thread's clock still: one millisecond at
  twice the base weight truncated to nothing. That test now guards it.
- Real time is charged in milliseconds from `uptime_ms`, and the start stamp
  always advances to `now`. Time is conserved across stints even when one stint
  is shorter than a millisecond: attributed coarsely, never lost. Slice 3's
  finer clock refines the attribution.
- The yield path excludes the running thread from the candidates but not from
  the average, so "nobody else is eligible" is a valid answer there and the
  thread keeps the CPU. The fallback counter is for the preemption path, where
  everyone is considered. The first boot counted a fallback for this reason,
  which the counter reported on day one.
- Lag is clamped, not decayed: preserved across a sleep so a brief sleep sheds
  no debt, capped at one slice so a long one banks no credit. That is mainline's
  rule. The decay `008` described is a proposal there, and it can be revisited
  when something here can sleep (slice 4).
