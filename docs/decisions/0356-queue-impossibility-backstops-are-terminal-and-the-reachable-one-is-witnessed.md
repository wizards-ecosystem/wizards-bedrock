# D-356 Queue impossibility backstops are terminal, and the reachable one is witnessed

Status: Accepted, 2026-09-29. Amends D-112; closes PW-566.

**Decision.** `kernel/src` holds no `debug_assert`: each queue backstop is a
terminal `panic!` or `unreachable!` that halts the kernel in both profiles. The
guards fall in two groups. The consistency invariant `WorkState::is_consistent`
and the refused, running and terminal ownership asserts carry driving
mutations, registry 683 through 686: a single predicate flip reaches each and
reddens its exact panic. Of the structural let-else backstops, one is
reachable by a single defect: a `begin_read` that leaves the entry queued
drives `RunningRead::request` into its `unreachable!`. Registry mutation 1,539
makes exactly that change and is a terminal `Caught::Panic` witness. The
others, `WorkState::pop`'s empty-head arm and the two lost-pending arms, are
reached only after the consistency or ownership guard has already halted the
kernel, so no single defect arrives at them. They are recorded as model-backed
defense in depth, terminal in release, and are not given a standalone
mutation. This closes the last open `PW-006` sub-disposition ahead of that
row's phase-close ADR.

**Rationale.** The backstops had been `debug_assert!(false, ...)` arms, which
the release build elides, so a state the `Ring` model (D-112) proves
unreachable would pass silently in the shipped kernel. Their rewrite as
terminal panics closed that half; what remained was liveness, whether a single
defect reaches a backstop so that breaking it is observable.

**Alternatives.** A source contract banning `debug_assert!(false)` in the kernel
was rejected. The backstops are already terminal, the codebase already treats an
`assert!`-to-`debug_assert!` downgrade as a tested defect that a release boot
catches, and a standing ban would be a defensive layer over a root cause that is
already fixed. Driving the two lost-pending arms by hand was rejected because it
needs a two-step corruption the consistency guard intercepts first, which would
witness that guard rather than the backstop.
