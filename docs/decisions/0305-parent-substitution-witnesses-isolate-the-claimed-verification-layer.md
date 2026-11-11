# D-305 Parent-substitution witnesses isolate the claimed verification layer

Status: Accepted, 2026-09-12. Amends D-244, D-301 and D-302; advances PW-001.

**Decision.** A parent-authority witness changes only parent authority, and a
redundant layer gets its own observation. The snapshot fixture renames the
complete accepted model directory to an adjacent real directory and links its
original pathname back to that same directory. Every file, byte and filesystem
object remains present. The evidence fixture selects the registered model
record, installs the same linked parent and observes immediately after the outer
bundle-path check but before hashing begins. The production path and hashing
checks remain independent and unchanged.

**Rationale.** The old fixtures copied only `model/record` before linking the
parent, so iteration failed at the next absent transcript and credited an
unrelated incomplete-fixture error. With the tree complete, the bundle check
stayed masked because the bounded hashing layer deliberately repeats its own
real-parent check before opening the file.

**Alternatives.** Copying the full directory entry by entry was rejected
because it adds fixture machinery and can accidentally omit a future
registered artifact. Keeping the partial fixture was rejected because any
missing file could satisfy the broad error assertion before the mutated path
was observed. Removing the hashing layer's repeated validation was rejected
because a boundary proof must not weaken the defense it is meant to test.
