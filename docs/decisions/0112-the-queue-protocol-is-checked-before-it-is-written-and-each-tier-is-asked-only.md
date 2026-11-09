# D-112 The queue protocol is checked before it is written, and each tier is asked only what it can answer

Status: Accepted (2026-08-24)

**Context.** D-84 adopted two verification tiers for `libs/ring-proto`
specifically, on the grounds that the queue ABI is the interface this kernel
will live with longest and the one place where writing code before checking the
design wastes the most work. The crate had unit tests and nothing else.

**Decision.** Three tiers, and the third is new: **the model comes before the
protocol**, and each tier is asked only the question it can answer.

*TLA+, for the whole protocol.* `models/Ring.tla` states D-79's admission rule
and four properties — no dropped completion, reservations are exactly the
unreported operations, never more completions than operations, nothing lost in
between. TLC enumerates all 1040 reachable states of four actors, two of them a
userspace given **no fairness at all**: it may stop reaping for ever, and every
property must still hold. `tla2tools.jar` is vendored and digest-pinned (D-25),
and `cargo xtask model-check` runs it.

*proptest, for the index arithmetic.* Four properties over sequences no
hand-written test would reach, including counters started across the `u32` wrap
with `adopt`. Both are run under `cargo test`, so the gate covers them.

*shuttle, for the interleavings.* Two sides advancing one shared counter, which
is the shape a submission tail and a completion head will have.

**Not in `ci`, for the model check only.** It needs a JVM — the one tool this
project cannot vendor into a Rust build — and a gate step that fails on a
machine without Java is a gate that teaches people to skip it. It is a named
task the phase runs, the treatment `vmware` already gets.

**Three things found by breaking the checks, which is the point of D-28.**

*TLC found an error in my own invariant.* `TypeOK` said `inflight` was bounded
by the submission queue. It is not: operations taken from the SQ stay in flight
while more are submitted, so what limits them is the *reservation* each holds,
and TLC produced the eleven-step trace reaching five in flight with a queue of
four. The model's first job turned out to be correcting the person writing it.

*Shuttle does not model memory ordering.* The first must-fail test was a model
whose only fault was `Relaxed` where `Release`/`Acquire` belonged, and it passed
ten thousand schedules — because shuttle explores thread *schedules* under
sequential consistency and never reorders a store. Memory ordering belongs to
the model and to the reviewer; interleaving is shuttle's. Asking the wrong tool
and believing the answer is the failure mode, and it was one command away.

*And shuttle only sees its own primitives.* The corrected model, built on
`std::sync::atomic`, also passed — because the scheduler has no preemption
points around an atomic it did not instrument, so every schedule was the same
schedule. `shuttle::sync::atomic` finds the split read-modify-write in
milliseconds. Two rounds of a passing wrong version, in one sitting, on the
tier whose whole purpose is to catch them.

**Consequences.** The must-fail tests stay, and the module doc now records what
each tier cannot see rather than only what it can. Breaking the model's
admission rule produces a counterexample naming `NoDroppedCompletion` — the
invariants are ordered in the config so a failure is reported by the name that
says what went wrong. `verify-vendor` iterates every listing rather than the
first, because a vendored directory added and not added there is one nothing
checks.

`getrandom` is pinned to `=0.3.1`: proptest's default features reach it and
0.3.2+ needs a `windows-sys` this toolchain's `dlltool.exe` cannot link — the
same wall `fatfs` avoids by dropping chrono. Recorded as the toolchain tax
`010` §9 measured rather than left to be rediscovered.

Left owed: the kernel has no ring yet. What is settled is the protocol it will
implement and the arithmetic it will implement it with, which is the order this
decision is about.
