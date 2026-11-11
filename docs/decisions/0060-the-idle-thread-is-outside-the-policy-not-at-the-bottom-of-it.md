# D-60 The idle thread is outside the policy, not at the bottom of it

Status: Accepted, 2026-08-23.

**Decision.** A thread of its own with `Class::Idle`, which the policy never
sees, rather than a lowest-weight thread or a second scheduling class. It is
outside the fair-share average, not counted when deciding whether the timer
needs arming, and picked only when no normal thread is runnable.
`start_idle` is idempotent and runs after interrupts are on, because a
halting thread needs them to wake.

**Rationale.** A weight small enough to ignore is still a weight. EEVDF
would give an idle-weighted thread the CPU now and then on eligibility, each
a wakeup spent on nothing. Keeping the class out of the policy makes the
tickless claim exact: an idle machine still arms nothing.

**Consequences.**

- The halt-in-place branch stays, because the idle thread does not exist
  during early boot or where the spawn failed.
- SMP owes it one idle thread per CPU, one more entry on D-56's list.
