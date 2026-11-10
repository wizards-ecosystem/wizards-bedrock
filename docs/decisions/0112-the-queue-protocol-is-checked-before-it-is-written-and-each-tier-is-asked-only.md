# D-112 The queue protocol is checked before it is written, and each tier is asked only what it can answer

Status: Accepted, 2026-08-24.

**Context.** D-84 adopted two verification tiers for `libs/ring-proto`: the
queue ABI is the interface this kernel lives with longest, where unchecked
design wastes the most work. The crate had only unit tests.

**Decision.** Three tiers, and the third is new: **the model comes before the
protocol**, and each tier is asked only the question it can answer.

*TLA+, for the whole protocol.* `models/Ring.tla` states D-79's admission rule
and four properties: no dropped completion, reservations equal the unreported
operations, never more completions than operations, nothing lost in between. TLC
enumerates all 1040 reachable states of four actors, two of them a userspace
with **no fairness at all**: it may stop reaping for ever, and every property
must still hold. `tla2tools.jar` is vendored and digest-pinned (D-25), and
`cargo xtask model-check` runs it.

*proptest, for the index arithmetic.* Four properties over sequences no
hand-written test would reach, including counters started across the `u32` wrap
with `adopt`. Both run under `cargo test`, so the gate covers them.

*shuttle, for the interleavings.* Two sides advance one shared counter, the
shape a submission tail and a completion head will have.

**Not in `ci`, for the model check only.** It needs a JVM, which this project
cannot vendor into a Rust build, and a gate step that fails without Java teaches
people to skip it. It is a named phase task, like `vmware`.

**Breaking the checks found three things, as D-28 intends.**

*TLC found an error in my own invariant.* `TypeOK` said the submission queue
bounded `inflight`. It does not: operations taken from the SQ stay in flight
while more are submitted, so the *reservation* each holds is the limit. TLC
produced the eleven-step trace reaching five in flight with a queue of four. The
model's first job was correcting its author.

*Shuttle does not model memory ordering.* The first must-fail test was a model
whose only fault was `Relaxed` where `Release`/`Acquire` belonged. It passed ten
thousand schedules, because shuttle explores thread *schedules* under sequential
consistency and never reorders a store. Memory ordering belongs to the model and
the reviewer; interleaving belongs to shuttle.

*Shuttle sees only its own primitives.* The corrected model, built on
`std::sync::atomic`, also passed. The scheduler has no preemption points around
an atomic it did not instrument, so every schedule was the same schedule.
`shuttle::sync::atomic` finds the split read-modify-write in milliseconds. That
made two passing wrong versions in one session, on the tier meant to catch them.

**Consequences.** The must-fail tests stay, and the module doc records what each
tier cannot see as well as what it can. Breaking the model's admission rule
yields a counterexample naming `NoDroppedCompletion`. The config orders the
invariants so a failure is reported by the name that says what went wrong.
`verify-vendor` now iterates every listing, not just the first, so no vendored
directory goes unchecked.

`getrandom` is pinned to `=0.3.1`. proptest's default features reach it, and
0.3.2+ needs a `windows-sys` this toolchain's `dlltool.exe` cannot link, the
wall `fatfs` avoids by dropping chrono. It is recorded as the toolchain tax
`010` §9 measured, so nobody rediscovers it.

Left owed: the kernel has no ring yet. What is settled is the protocol it will
implement and the arithmetic it will use, in that order.
