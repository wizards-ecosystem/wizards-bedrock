# D-112 The queue protocol is checked before it is written, and each tier is asked only what it can answer

Status: Accepted, 2026-08-24.

**Decision.** Three tiers for `libs/ring-proto`, extending D-84's two: **the
model comes before the protocol**, and each tier is asked only the question it
can answer.

*TLA+, for the whole protocol.* `models/Ring.tla` states D-79's admission rule
and four properties: no dropped completion, reservations equal the unreported
operations, never more completions than operations, nothing lost in between. TLC
enumerates every reachable state of four actors, two of them a userspace with
**no fairness at all**: it may stop reaping for ever, and every property must
still hold. `tla2tools.jar` is vendored and digest-pinned (D-25), and
`cargo xtask model-check` runs it. The config orders the invariants so a
failure is reported by the name that says what went wrong.

*proptest, for the index arithmetic.* Properties over sequences no hand-written
test would reach, including counters started across the `u32` wrap with
`adopt`. It runs under `cargo test`, so the gate covers it.

*shuttle, for the interleavings.* Two sides advance one shared counter, the
shape a submission tail and a completion head will have, on
`shuttle::sync::atomic`.

**Not in `ci`, for the model check only.** It needs a JVM, which this project
cannot vendor into a Rust build, and a gate step that fails without Java teaches
people to skip it. It is a named phase task, like `vmware`.

**Rationale.** Breaking the checks, as D-28 intends, showed what each tier can
see. TLC corrected the model's own invariant: the *reservation* each operation
holds, not the submission queue, bounds `inflight`. Shuttle does not model
memory ordering: a `Relaxed` where `Release`/`Acquire` belonged passed, because
shuttle explores thread *schedules* under sequential consistency and never
reorders a store, so memory ordering belongs to the model and the reviewer and
interleaving belongs to shuttle. And shuttle sees only its own primitives: on
`std::sync::atomic` every schedule was the same schedule.

**Consequences.** The must-fail tests stay, and the module doc records what each
tier cannot see as well as what it can. Breaking the model's admission rule
yields a counterexample naming `NoDroppedCompletion`. `verify-vendor` iterates
every listing, so no vendored directory goes unchecked.

`getrandom` is pinned to `=0.3.1`. proptest's default features reach it, and
0.3.2+ needs a `windows-sys` this toolchain's `dlltool.exe` cannot link. It is
recorded as the toolchain tax `010` §9 measured, so nobody rediscovers it.
