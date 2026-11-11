# D-247 Type errors and typed ownership make two review claims durable

Status: Accepted, 2026-08-30. Amends D-85, D-150 and D-223; closes PW-529 and PW-530.

**Decision.** The actual-`wost` probe now carries D-85 as well as D-223. Two
negative programs use `SpinLock::lock` and the real `ThreadContext`: one takes
a second token-bearing lock and one passes the still-borrowed token to a
blocking-shaped operation. Each compilation must fail with exactly one coded
diagnostic, `E0499`, at the probe source. A positive control explicitly drops
the guard and must compile. The assertion classifies rustc's diagnostic code;
it does not depend on English diagnostic wording.

The direct user-call surface has a typed registry beside that compiler
harness. A structural `syn` walk requires the registry, the `Call` enum and the
`Call::decode` dispatch to name exactly the same variants, with no wildcard or
duplicate arm. Every variant is classified `CannotWait` or
`CompletionPlane`. The sole completion-plane owner must invoke `wait_ring`
directly, and a cannot-wait arm may not. A new direct door therefore cannot
compile its host proof until its ownership is classified.

This does not pretend to infer transitive waiting from arbitrary Rust. The
reviewer still owns the semantic classification D-150 requires; the machine
now makes omitting that classification impossible and pins the one deliberate
trap-side wait to the completion plane that owns it.

**Rationale.** D-85's three compiler experiments (a second lock and reuse of a
live guard's `ThreadContext` fail with `E0499`, dropping the guard compiles,
and removing the guard's `Drop` makes both refusals vanish under non-lexical
lifetimes) were the right breaks but a one-time transcript, and D-223's
cross-compiling harness covered only the guards' `!Send` contract. D-150's
rule is deliberately not a whole-program analysis, because whether an
operation can wait is a design fact, not a function-name fact, but its
direct-call surface had no inventory, so a new variant or wildcard arm could
bypass the point where that fact must be decided.

**Alternatives.** Copying `ThreadContext` and a guard into a host-only crate was
rejected because it would test a model that can drift from `wost`. Matching
rustc's prose was rejected because wording is not the language contract;
`E0499` is. A name search for `wait`, `yield` or `block` was rejected because
both false positives and renamed helpers would turn prose convention into a
claimed call-graph proof. Whole-program analysis remains rejected for D-150's
original reason.

**Evidence and consequences.** Compiling out `Drop for SpinLockGuard` makes the
two negative programs compile and fails only
`contracts::a_live_spin_guard_keeps_the_thread_context_borrowed`. Deleting
`RingEnter`'s typed row fails `contracts::every_direct_door_has_an_explicit_wait_owner`
with the enum/registry set difference. Both breaks are retained as host
mutations.
