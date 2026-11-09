# D-202 The idle-start latch reserves an attempt and commits only a live thread

Status: Accepted, 2026-08-28. Amends D-61 and D-117; closes PW-306.

**Context.** `start_idle` claimed a one-shot latch before asking the bounded
thread table for a slot. If the table was transiently full, `spawn` returned
`Full` but the latch remained claimed. Every later call then returned before
retrying, leaving a kernel which could become idle without its idle thread or
the deferred cleanup and refusal-reporting work that thread owns. Wizzy
process churn is the concrete caller which will make transient slot pressure
ordinary rather than synthetic.

**Decision.** Idle admission has three states: unstarted, starting and
started. A caller atomically reserves `unstarted -> starting`; competing or
later callers do nothing while that reservation exists. Any spawn refusal
rolls the state back to unstarted, while a successful spawn commits started.
The new slot is marked `Idle` through `spawn_prepared`'s existing arrangement
window, while it is still reserved and cannot be scheduled, rather than being
published as an ordinary runnable thread and reclassified afterwards.

**Rationale.** The state describes the work rather than merely whether one
caller has passed a line. Rollback makes retry explicit, and the intermediate
reservation preserves exactly-one creation if multiple CPUs eventually call
the idempotent entry. Reusing the spawn transaction's arrangement window also
keeps scheduler policy true at the first instant the slot becomes runnable.

**Alternatives.** Claiming `Once` after `spawn` was rejected because two
callers could each publish a thread before either claimed it. Clearing or
extending the general `Once` abstraction was rejected because hardware
initialization uses its irreversible semantics correctly; this retry belongs
to idle-thread policy. Reserving a raw table slot in `start_idle` was rejected
because it would duplicate `spawn_prepared`'s thread/task rollback protocol.

**Consequences.** Release QEMU fills every spawnable slot, observes an
idle-start refusal with no idle thread, releases the fillers, retries more than
once, and finds exactly one live idle slot consuming exactly one resource. A
self-test-only recovery lets the deliberately burned state continue through
the rest of the suite; removing the rollback reddened only `a failed idle
start can be retried exactly once` with 208/209. Its persistent mutation brings
the register to 140 entries and the complete gate carries 209 machine checks.
