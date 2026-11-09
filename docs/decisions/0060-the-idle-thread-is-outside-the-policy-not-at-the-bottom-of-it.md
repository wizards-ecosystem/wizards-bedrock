# D-60 The idle thread is outside the policy, not at the bottom of it

Status: Accepted, 2026-08-23.

**Context.** Slice 3's sleep had a branch for "nobody else is runnable": the
sleeping thread kept the CPU and halted on its own stack. Correct, but it
exists only because there is sometimes nothing to run, and everything after
this phase is easier if something always is.

**Options.** A thread of the lowest weight the policy will take; a second
scheduling class; or no idle thread at all, keeping the halt-in-place branch
everywhere it is needed.

**Decision.** A thread of its own with `Class::Idle`, which the policy never
sees. It is not in the fair-share average, it is not counted when deciding
whether the timer needs arming, and it is picked only when no normal thread
is runnable. `start_idle` is idempotent and runs after interrupts are on,
because a thread that halts needs them to be woken from.

**Rationale.** A weight small enough to ignore is still a weight: EEVDF would
hand an idle-weighted thread the CPU every so often on eligibility grounds,
and every one of those is a wakeup spent on nothing. The vision doc counts
wakeups. Keeping the class out of the policy is four lines and makes the
tickless claim exact rather than approximate — an idle machine still arms
nothing.

**Consequences.**

- The halt-in-place branch stays, because the idle thread does not exist
  during early boot or on a machine where the spawn failed. The self-checks
  start the idle thread half way through their own run, so both sides of it
  are exercised: the sleep checks above it have no idle thread, the checks
  below it do.
- Two checks, both broken once. The idle thread runs when the boot thread
  sleeps, and does not run at all while a spinner is runnable. Either alone is
  satisfied by a bug — an idle thread that never runs, or one the policy
  schedules like any other — and the second break, making it a normal thread,
  reddened four checks including the tickless ones.
- SMP owes it one idle thread per CPU, which is one more entry on D-56's list.
