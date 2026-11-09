# D-212 The local gate is one registry rendered three ways

Status: Accepted (2026-08-28), amends D-22, D-124 and D-147; closes PW-501

**Context.** `PASSES` already drives the Cargo configurations used by check,
clippy and rustdoc, but `ci` is a handwritten sequence. Two repository tests
recover that sequence by splitting `tasks.rs` as text: one counts calls whose
spelling looks like a boot, while the other searches for eight exact call
fragments and then special-cases the ninth. The README separately spells the
boot count and still says Cargo checks three configurations although
`refuse-pci` made four. Help says `ci` runs “every check above” even though the
standalone `check` task is deliberately not a second CI pass. Each copy can be
locally plausible and globally wrong.

**Decision.** One typed `gate::Registry` owns both dimensions of the local
gate: every Cargo crate graph and every ordered CI stage. Cargo-driven tasks
iterate its graph slice. `ci` can obtain the stage slice only through an
accessor which first verifies README's marked Markdown rendering, then it
exhaustively dispatches each enum variant. The help renderer lists both slices
directly. A host check exercises the same preflight.

The registry remains a local, self-contained sequence. It adds no hosted
service, network lookup, generator prerequisite or tool the repository cannot
vendor. The standalone `check`, VMware, TLC and mutation tasks retain their
existing policy; only the authority for what `ci` executes moves.

**Rationale.** Data can be iterated, rendered and compared without parsing the
Rust that consumes it. One entry now determines execution order, help output
and the reviewable documentation snapshot. The snapshot is intentionally
checked in rather than silently rewritten: changing the gate must produce a
diff that says which coverage was added or removed.

**Alternatives.** Strengthening the source scanner was rejected because a new
wrapper, reordered expression or renamed function would require another
parser convention without making the body authoritative. Generating README at
build time was rejected because documentation changes would disappear into
untracked output. Listing only boot stages was rejected because the already
stale Cargo-graph count demonstrates that the non-machine half is the same
contract. Running standalone `check` in addition to clippy was rejected as an
unrelated duplicate build, not evidence missing from the present gate.

**Consequences.** The handwritten CI body, source scanners and prose number
words are removed. The registry retains the same four Cargo graphs and nine
ordered stages the green gate ran before this change. Help now shows those
graphs and stages directly; README's marked table is their exact Markdown
rendering.

Removing the `refuse-pci` graph made
`the_ci_registry_drives_help_and_documentation` red with the missing table row.
The real `cargo xtask ci` also refused that mismatch before its first stage,
so removing the host-test stage cannot remove the only observer. Adding a
second `ShippedBoot` made the same exact host check red with a tenth stage. Both
transformations compiled, were restored and persist as host mutations, bringing
the registry to 190.
