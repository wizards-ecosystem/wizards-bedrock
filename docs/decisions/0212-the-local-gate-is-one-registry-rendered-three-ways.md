# D-212 The local gate is one registry rendered three ways

Status: Accepted, 2026-08-28. Amends D-22, D-124 and D-147; closes PW-501.

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

**Rationale.** `ci` had been a handwritten sequence recovered by tests that
split `tasks.rs` as text, while README and help restated its counts, and each
copy could be locally plausible and globally wrong. Data can be iterated,
rendered and compared without parsing the Rust that consumes it. One entry now
determines execution order, help output and the reviewable documentation
snapshot. The snapshot is intentionally checked in rather than silently
rewritten: changing the gate must produce a diff that says which coverage was
added or removed.

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
ordered stages the green gate ran before this change. Help shows those graphs
and stages directly; README's delimited table is their exact Markdown
rendering. The real `cargo xtask ci` refuses a mismatch before its first stage,
so removing the host-test stage cannot remove the only observer. Two persistent
host mutations, a removed `refuse-pci` graph and a second `ShippedBoot`, keep
`the_ci_registry_drives_help_and_documentation` red.
