# D-60 The idle thread is outside the policy, not at the bottom of it

Status: Accepted, 2026-08-23.

**Context.** Slice 3's sleep had a branch for "nobody else is runnable": the
sleeping thread kept the CPU and halted on its own stack. Correct, but it
exists only because sometimes nothing can run, and everything after this
phase is easier if something always can.

**Options.** A thread of the lowest weight the policy will take; a second
scheduling class; or no idle thread, keeping the halt-in-place branch
everywhere it is needed.

**Decision.** A thread of its own with `Class::Idle`, which the policy never
sees. It is outside the fair-share average, not counted when deciding
whether the timer needs arming, and picked only when no normal thread is
runnable. `start_idle` is idempotent and runs after interrupts are on,
because a halting thread needs them to wake.

**Rationale.** A weight small enough to ignore is still a weight. EEVDF would
give an idle-weighted thread the CPU now and then on eligibility, each a
wakeup spent on nothing, and the vision doc counts wakeups. Keeping the class
out of the policy takes four lines and makes the tickless claim exact: an idle
machine still arms nothing.

**Consequences.**

- The halt-in-place branch stays, because the idle thread does not exist
  during early boot or where the spawn failed. The self-checks start it half
  way through their run, so both sides are exercised: the sleep checks
  above it have no idle thread, the checks below it do.
- Two checks, both broken once. The idle thread runs when the boot thread
  sleeps, and never runs while a spinner is runnable. A bug satisfies either
  alone (an idle thread that never runs, or one the policy schedules like
  any other). The second break, making it a normal thread, reddened four
  checks including the tickless ones.
- SMP owes it one idle thread per CPU, one more entry on D-56's list.
