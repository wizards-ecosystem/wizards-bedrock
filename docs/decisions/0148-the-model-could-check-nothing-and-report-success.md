# D-148 The model could check nothing and report success

Status: Accepted, 2026-08-25.

**Decision.** Each of these is pinned against something outside itself:

- **A host test requires every invariant the model defines to be named by
  `models/Ring.cfg`, and the `INVARIANTS` block to be non-empty.** Empty the
  block and TLC still reports *"Model checking completed. No error has been
  found"*, because a run with no invariants cannot find one. The test runs in
  `cargo test` at every commit, while the model check itself needs a JVM and is
  deliberately not in `ci`: the two-tier argument `mutate.rs` makes, applied to
  the other tool that lives outside the gate.
- **`ring_proto::VERSION` is asserted to be 1, as a literal.** It appeared on
  both sides of every comparison, on a field whose purpose is to be read by a
  *program*.
- **The exception constants index the name table**, so a wrong constant gives
  the wrong name, and `COUNT`'s neighbours are checked unnamed.
- **`ring_proto::is_consistent` is driven to the edge**: submit until the ring
  is full, asserting the invariant at every step and after the refusal, so it is
  exercised where it could say no rather than only where it says yes.

**Rationale.** The failure mode is *silence that reads as success*: a model with
no invariants completes, prints a green line, and reports a state count that
looks like work. D-84 made the ring protocol modelled before it was written,
and D-112's counterexample run proved once that the model could fail; this is
the standing version, and the cheap tier is enough because the way it breaks is
deletion, not subtlety.

**Consequences.** The model's invariant list is stated in two places,
`Ring.cfg` and the test, and they must agree. As with `ci`'s step list (D-147),
that is a transcription and the right trade for a tool the gate cannot run.

**Left owed.** The remaining Tier 3 findings are smaller still and none is
closed here: `ps2-keys` anchors 21 of about 89 base scancodes, so a swap of two
unanchored keys survives; `libs/timebase`'s round-to-nearest survives becoming
round-up for the multipliers its tests use; `libs/pixel`'s `span` is only ever
exercised where `pitch == width * bytes_per_pixel`, so a padded framebuffer
would map short. Each is a real hole in a real check and each needs a table of
values from outside the crate, which is a session's work rather than an hour's.
