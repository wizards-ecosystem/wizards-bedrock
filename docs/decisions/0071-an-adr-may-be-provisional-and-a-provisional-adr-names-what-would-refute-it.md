# D-71 An ADR may be Provisional, and a Provisional ADR names what would refute it

Status: Accepted (2026-08-23)

**Context.** This log has two statuses: **Accepted**, and **Verified** for
decisions supported by `003`'s adversarially-verified tier. The 2026-08-23
research pass found that the strongest label was doing work the evidence could
not support. D-13 — the framekernel, the most structural decision in the
project — is marked **Verified** on the strength of a single paper for which
*no independent replication exists*, and whose headline TCB figure does not
survive scrutiny. `008` §7 already recorded that honestly; the ADR did not.

**Decision.** A third status, **Provisional**: accepted and built upon, but
resting on evidence that has not been independently confirmed. A Provisional
ADR must state a **falsification condition** — what would have to be observed
for it to be revisited.

Applied now to three:

- **D-13** (framekernel) — refuted if independent measurement shows the
  discipline's cost materially exceeds the paper's, or if the containment
  property fails to hold at scale in a project not authored by its proponents.
- **D-09** (WASI) — see D-72, which restates it; the watch condition is the
  Component Model reaching Phase 4, or an interpreter shipping support.
- **D-08** (WizFS) — see D-73; refuted if the crash harness proves
  unbuildable before the format is frozen.

**Rationale.** "Verified" should mean verified. Marking a decision Provisional
costs nothing, changes no code, and prevents the specific failure where a
project defends a choice because its own log says the question is settled. The
falsification condition is the load-bearing half: no foundational ADR here
previously stated what would change its mind.

**Consequences.**

- D-13's status changes from Verified to Provisional. This is not a retreat
  from the framekernel — §1 of `010` finds the containment property *better*
  evidenced than before — it is an accurate label on the evidence behind it.
- `003`'s tiers still govern what "Verified" means. Provisional sits below it.
