# D-184 The closed compiler gate has one finite, evidence-bound foundation register

Status: Accepted (2026-08-27), refines D-18, D-77, D-135, D-175 and D-183

**Context.** D-183 permits kernel correctness, concrete language-neutral
prerequisites and artifact-handoff specification while its compiler gate is
closed; gate work in `wizards-ink` is externally owned. Consumer scaffolding
waits for PW-601's pinned-draft-artifact and runtime-seam trigger. D-183
deliberately does not enumerate every legitimate kernel debt. Calling the
resulting space "polish" would make it unlimited; freezing the OS repository
completely would leave safe-abstraction, DMA, privilege and verification
defects in the foundation Wizzy must trust.

The Phase 6 slices had landed and QEMU reported 189 checks, but the current
189-check VMware round and current 77-mutation full sweep were not recorded.
A fresh source audit also found five P0 boundaries which a product-language
decision cannot waive: shared-mapping lifetime, mapping rollback ownership,
DMA lifetime after timeout, transactional PCI BAR/claim handling, and CR0.WP.

**Options.** Leave D-183 categorical; admit anything described as kernel
polish; freeze all OS work; or keep one checked register whose entries require
a concrete defect, invariant, proof gap, handoff consumer or explicit trigger.

**Decision.** The last option. `016-pre-wizzy-foundation.md` is the sole live
closed-gate OS backlog. It records state, priority, why each item is admissible,
the finite deliverable and the evidence that closes it. `REQUIRED`, `READY`,
`VERIFY` and `DECIDE` rows must become `DONE` or `SUPERSEDED` before the kernel
is called the polished Wizzy foundation. `TRIGGER` means stopped, not overdue.
Post-gate product APIs and later roadmap phases are classified explicitly and
do not enter the register by being renamed foundation work.

A new row needs falsifiable evidence: a live defect, violated invariant,
missing qualification proof, concrete artifact handoff consumer, or named
trigger. "State of the art", convenience and eventual usefulness are not
evidence. A decision-changing item gets its own ADR. Unknown future defects
may therefore be added without making the list an open-ended feature backlog.

The compiler gate still opens only when all four D-183 conditions hold together
at one reachable pinned `wizards-ink` revision. Ordinary OS CI remains
self-contained; a sibling checkout never becomes an implicit dependency.

**Rationale.** A finite register makes "ready" a testable claim while keeping
the language boundary intact. Risk is retired before Wizzy depends on it, but
an imagined runtime cannot freeze APIs for the real compiler. Separating
confirmed, verify-first, decision and trigger states also prevents a static
audit observation from being reported as an established exploit.

**Consequences.** Phase 6 is better described as planned-slices-complete and
not closed, rather than code-complete. Its close now includes the register's
P0/P1 work and the exact exceptional evidence on one commit. The old
`014-storage.md` mutation sentence is historical source-count evidence, not a
claim that the current 77-entry sweep ran. The missing Phase 3 VMware result and
old non-self-test VMware boots remain historical limits and are not recreated.

An `xtask` prose test checks that the register is finite in shape: unique IDs,
known states/priorities, substantive close evidence, the boundary markers and
current-plan references to D-184. Like the D-183 check, it is a host repository
claim and does not add a machine mutation.
