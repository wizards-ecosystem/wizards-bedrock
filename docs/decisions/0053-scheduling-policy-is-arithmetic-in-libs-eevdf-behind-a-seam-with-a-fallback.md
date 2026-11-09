# D-53 Scheduling policy is arithmetic in `libs/eevdf`, behind a seam with a fallback

Status: Accepted (2026-08-22)

**Context.** D-15 chose EEVDF and wanted pluggable policy as "a cheap future
door, not a v1 feature"; `008-research-notes-2.md` §4.4 took sched_ext's
lesson as a constraint on the interface — policy *separable and observable,
with a fallback path* — and flagged the lag rule as the part that is easy to
omit.

**Decision.** The policy is `libs/eevdf`, pure functions over plain data:
virtual time that wraps and compares by signed difference, charging scaled
by weight, a weighted average as the fair-share point, eligibility as
non-negative lag, the pick as earliest deadline among the eligible, and
placement that clamps lag to one slice either way. Thirteen host tests
including a simulation that drives the policy the way the kernel does and
asserts a 1:2 weight ratio lands at 1.9–2.1, and a sleeper test that asserts
the clamp is what stops a returning thread running unopposed.

The kernel's `thread.rs` is the mechanism side of the seam: it keeps an
`Entity` per thread, feeds the runnable ones to the policy, and acts on the
answer. Candidates are considered in rotation from just after the running
thread, so equal deadlines take turns rather than the lowest index winning
every tie. If the policy ever answers "nobody" when everyone was considered —
which its own tests say cannot happen — the kernel takes the next runnable
slot round-robin and counts it, and a self-check requires the count to be
zero. That is the fallback in its smallest honest form: not a second policy,
but a way to keep the machine up and make the inconsistency visible.

**Rationale.** Arithmetic that decides who runs is exactly what D-30's
pattern exists for; on the host it can be driven a thousand steps in a
millisecond. Separability costs nothing — the seam is a function signature —
and observability is one counter.

**Consequences.**

- Virtual time carries a `RESOLUTION` of 1024 units per unit of real time,
  because the fairness simulation found the heavy thread's clock standing
  still: one millisecond at twice the base weight truncated to nothing. The
  test that found it is the test that guards it.
- Real time is charged in milliseconds from `uptime_ms`, and the start
  stamp always advances to `now`, so time is conserved across stints even
  when single stints are shorter than a millisecond — it is attributed
  coarsely, never lost. Slice 3's finer clock refines the attribution.
- The yield path excludes the running thread from the candidates but not
  from the average, so "nobody else is eligible" is a real answer there and
  the thread keeps the CPU; the fallback counter is reserved for the
  preemption path, where everyone is considered. The first boot counted a
  fallback for exactly this reason, and the counter earned its keep on day
  one by saying so.
- Lag is clamped, not decayed: preserved across a sleep so a brief sleep
  sheds no debt, capped at one slice so a long one banks no credit. That is
  mainline's rule; the decay `008` described is a proposal there, and it can
  be revisited when something here can sleep (slice 4).
