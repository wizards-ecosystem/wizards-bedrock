# D-71 An ADR may be Provisional, and a Provisional ADR names what would refute it

Status: Accepted, 2026-08-23.

**Context.** This log has two statuses: **Accepted**, and **Verified** for
decisions supported by `003`'s adversarially-verified tier. The 2026-08-23
research pass found the strongest label claiming more than the evidence
supports. D-13, the framekernel, is the project's most structural decision. It
is marked **Verified** on one paper with *no independent replication*, whose
headline TCB figure does not survive scrutiny. `008` §7 already recorded that;
the ADR did not.

**Decision.** A third status, **Provisional**: accepted and built upon, but
resting on evidence not yet independently confirmed. A Provisional ADR must
state a **falsification condition**: what observation would revisit it.

Applied now to three:

- **D-13** (framekernel): refuted if independent measurement shows the
  discipline's cost materially exceeds the paper's, or if the containment
  property fails at scale in a project not authored by its proponents.
- **D-09** (WASI): see D-72, which restates it; the watch condition is the
  Component Model reaching Phase 4, or an interpreter shipping support.
- **D-08** (WizFS): see D-73; refuted if the crash harness proves unbuildable
  before the format is frozen.

**Rationale.** "Verified" should mean verified. Marking a decision Provisional
costs nothing, touches no code, and stops a project defending a choice because
its own log calls the question settled. The falsification condition matters
most: no foundational ADR here previously stated what would change its mind.

**Consequences.**

- D-13's status changes from Verified to Provisional. The framekernel stays:
  §1 of `010` finds the containment property *better* evidenced than before,
  and the new status labels that evidence accurately.
- `003`'s tiers still govern what "Verified" means. Provisional sits below it.
