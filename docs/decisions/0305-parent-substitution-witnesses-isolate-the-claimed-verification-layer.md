# D-305 Parent-substitution witnesses isolate the claimed verification layer

Status: Accepted (2026-09-12), amends D-244, D-301 and D-302; advances PW-001

**Context.** Complete-sweep mutations 42 and 44 removed the parent checks from
outer evidence-bundle and qualification-snapshot verification, yet their named
tests survived. Each fixture copied only `model/record` into its alternate
directory before linking that directory over the accepted `model` parent. Even
without the mutated check, iteration failed at the next absent model transcript.
The assertions therefore credited an unrelated incomplete-fixture error instead
of observing whether identical accepted bytes could arrive through a linked
parent. After preserving the complete tree, mutation 42 still remained masked:
the bounded hashing layer deliberately repeats its own real-parent check before
opening the file.

**Decision.** A parent-authority witness changes only parent authority, and a
redundant layer gets its own observation. The snapshot fixture renames the
complete accepted model directory to an adjacent real directory and links its
original pathname back to that same directory. Every file, byte and filesystem
object remains present. The evidence fixture selects the registered model
record, installs the same linked parent and observes immediately after the outer
bundle-path check but before hashing begins. The production path and hashing
checks remain independent and unchanged.

Copying the full directory entry by entry was rejected because it adds fixture
machinery and can accidentally omit a future registered artifact. Keeping the
partial fixture was rejected because any missing file could satisfy the broad
error assertion before the mutated path was observed. Removing the hashing
layer's repeated validation was rejected because a boundary proof must not
weaken the defense it is meant to test.

**Proof.** Removing `checked_bundle_file` from qualification digesting made the
pre-hash observer run and failed
`a_bundle_parent_link_is_rejected_before_qualification_hashing`; the hashing
layer still refused the link afterward. After restoration, removing the
snapshot's contained-file check made
`a_snapshot_parent_link_cannot_substitute_qualification_bytes` accept the
complete linked tree. Each exact test passes with its guard restored; the
registry source and check-name proofs pass. The remaining complete-report
outcomes and a clean successor sweep stay in PW-001.
