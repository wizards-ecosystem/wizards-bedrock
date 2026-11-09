# D-356 Queue impossibility backstops are terminal, and the reachable one is witnessed

Status: Accepted (2026-09-29), amends D-112; closes PW-566

**Context.** The 2026-08-29 handoff left a finding without a register row. The
queue's unreachable-branch backstops were `debug_assert!(false, ...)` arms, which
the release build elides, so a state the `Ring` model (D-112) proves unreachable
would pass silently in the shipped kernel, and nothing executable drove the
branches to show the guard was live. The four asserts have since been rewritten,
and `kernel/src` now holds no `debug_assert` at all: each backstop is a terminal
`panic!` or `unreachable!` that halts the kernel in both profiles, so the
release-elision half of the finding was already closed by construction. The open
half was liveness, whether any single defect reaches a backstop so that breaking
it is observable.

**Decision.** The guards fall in two groups. The consistency invariant
`WorkState::is_consistent` and the refused, running and terminal ownership asserts
already carry driving mutations, registry 683 through 686: a single predicate flip
reaches each and reddens its exact panic. The structural backstops that remain are
the let-else arms that fire only when a pending slot the code just held has
vanished, or when a read runs without the running state its own `begin_read` set.
One of these is reachable by a single defect: a `begin_read` that leaves the entry
queued drives `RunningRead::request` into its `unreachable!`. Registry mutation
1,539 makes exactly that change and is a terminal `Caught::Panic` witness. The
others, `WorkState::pop`'s empty-head arm and the two lost-pending arms, are
reached only after the consistency or ownership guard above has already halted the
kernel, so no single defect arrives at them. They are recorded as model-backed
defense in depth, terminal in release, and are not given a standalone mutation.
This closes the last open `PW-006` sub-disposition ahead of that row's phase-close
ADR.

**Evidence.** On 2026-09-29 the debug self-test passed 265/265. Mutation 1,539
turns `begin_read`'s `*state = ReadState::Running` into `ReadState::Queued`; the
first boot-time read then reaches `RunningRead::request`, and the boot halts with
"running file work was made without a running read", reddening only that self-test.
A search confirms `kernel/src` carries no `debug_assert`, so no backstop is
release-elided.

**Alternatives.** A source contract banning `debug_assert!(false)` in the kernel
was rejected. The backstops are already terminal, the codebase already treats an
`assert!`-to-`debug_assert!` downgrade as a tested defect that a release boot
catches, and a standing ban would be a defensive layer over a root cause that is
already fixed. Driving the two lost-pending arms by hand was rejected because it
needs a two-step corruption the consistency guard intercepts first, which would
witness that guard rather than the backstop.
