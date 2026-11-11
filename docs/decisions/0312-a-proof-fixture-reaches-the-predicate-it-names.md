# D-312 A proof fixture reaches the predicate it names

Status: Accepted, 2026-09-12. Amends D-244 and D-301; advances PW-001.

**Decision.** Bound the NVMe order proof to `finish_preparation` and require no
register write before its `Controller` cleanup owner. Remove the redundant
qualified-type clause. Parse the exact kernel-entry grammar once, retain the
parsed handler in `EntrySpec`, and resolve that retained name; the alternate
fixture changes both declaration and invocation so only the `kernel_main`
identity guard can refuse it. Exercise item-macro shadow policy and active
build-script validation directly before their integrated callers.

**Rationale.** Five survivors sat in host checks whose broad fixtures stopped
before, or passed around, the changed predicate: the NVMe witness bounded the
whole preparation path, a second qualified-type predicate implied the mutated
one, three copies of the kernel-entry grammar could disagree, and shadowing and
build-script refusals were credited through callers with other refusal doors.

**Alternatives.** Keeping broader fixtures as the only witnesses was rejected
because their verdicts do not identify which contract refused the input.
Retaining duplicate type and entry predicates was rejected because no mutation
of one redundant layer can prove it is load-bearing.
