# D-148 The model could check nothing and report success

Status: Accepted (2026-08-25)

**Context.** The last of the audit's Tier 3 findings, and one of them is worse
than its tier suggests.

**TLC checks what `INVARIANTS` names, and nothing checked that it names
anything.** `models/Ring.cfg` lists five. Empty the block and TLC still reports
*"Model checking completed. No error has been found"* — because a run with no
invariants cannot find one — so `cargo xtask model-check` passes with nothing
checked. The config's own comment says the block exists so "a failure is
reported by the name that says what went wrong"; nothing established that a
failure could be reported at all.

D-84 made the ring protocol modelled before it was written, and D-112's
counterexample run is what proved the model could fail. That was a one-time act,
at the moment the model was written — the same shape D-119 made a task for.

**Three constants that only equalled themselves.** `ring_proto::VERSION`
appeared on both sides of every comparison — the kernel writes it into the
header, the checks compare the header against it — so `= 7` survived the whole
workspace, on a field whose entire purpose is to be read by a *program*.
`exception::name(14)` was spelled as a literal on both sides, so `PAGE_FAULT`
was read by nothing and could have named any vector; `COUNT` was satisfied by
anything at or above 31. And `ring_proto::is_consistent` — the oracle six
machine checks use, including the one that catches a lying completion head —
was asserted *true* at every call site and never false, so `{ true }` survived.

**Decision.** Each is pinned against something outside itself:

- A host test requires every invariant the model defines to be named by the
  config, and the block to be non-empty. Milliseconds, in `cargo test`, so it
  runs at every commit while the model check itself needs a JVM and is
  deliberately not in `ci` — which is the two-tier argument `mutate.rs` makes,
  applied to the other tool that lives outside the gate.
- `VERSION` is asserted to be 1, as a literal.
- The exception constants are used to *index* the name table, so a wrong
  constant gives the wrong name, and `COUNT`'s neighbours are checked unnamed.
- `is_consistent` is driven to the edge — submit until the ring is full,
  asserting the invariant at every step and after the refusal — so it is
  exercised where it could say no rather than only where it says yes.

**Rationale.** The model finding is the one worth remembering, because the
failure mode is *silence that reads as success*. Every other check in this
project fails loudly when broken; a model with no invariants completes, prints a
green line, and reports a state count that looks like work. D-112 verified the
model could produce a counterexample once. What was missing was the standing
version of that — and the cheap tier is enough, because the way it breaks is
deletion, not subtlety.

The three constants are D-124's rule at its most literal, and their being Tier 3
is right: none reaches hardware, and `VERSION` is the only one a *program* can
observe. It is also the one that would be hardest to notice, since a wrong
version is not wrong until something else is reading it.

**Consequences.**

- Four host tests. All broken once: the emptied `INVARIANTS` block, a renumbered
  `PAGE_FAULT`, a bumped `VERSION`, and `is_consistent` returning `true`.
- The model's invariant list is now stated in two places — `Ring.cfg` and the
  test — and they must agree. As with `ci`'s step list (D-147), that is a
  transcription and the right trade for a tool the gate cannot run.

**Left owed.** The remaining Tier 3 findings are smaller still and none is
closed here: `ps2-keys` anchors 21 of about 89 base scancodes, so a swap of two
unanchored keys survives; `libs/timebase`'s round-to-nearest survives becoming
round-up for the multipliers its tests use; `libs/pixel`'s `span` is only ever
exercised where `pitch == width * bytes_per_pixel`, so a padded framebuffer
would map short. Each is a real hole in a real check and each needs a table of
values from outside the crate, which is a session's work rather than an hour's.
