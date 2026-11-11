# D-237 A restored mutation is written, not copied

Status: Accepted, 2026-08-29. Supersedes the mechanism of D-233 and D-236; advances PW-001.

**Decision.** The harness writes source files and never copies them. Restoring
a mutation and re-copying a tracked workspace both read the snapshot's bytes
and write them to the destination, so every source the harness lays down is
newer than anything already built from it. Cargo's own dependency tracking
does the invalidation from there, transitively, for the host and the kernel
target alike, through however many crates inlined the value. No package
mapping, no target flag, no reverse-consumer graph, and no `cargo clean`.

**Rationale.** D-233 already named the cause: on Windows `fs::copy` restores
the snapshot's old timestamp, so Cargo sees a source older than the object
built from the mutation and has no reason to rebuild it. It rejected touching
the restored file as filesystem-dependent and cleaned the package instead;
D-236 then grew that into a target-aware, graph-aware clean. Writing bytes is
not touching a file: it chooses no timestamp, so there is no granularity to
reconcile and no future time to handle. `Session` already applied a mutation
with `fs::write` and only the undo used `fs::copy`; `copy_tracked` carried the
same defect because a completed sweep keeps its workspace and objects for the
next sweep to re-copy over.

**Alternatives.** Keeping D-236's clean was rejected because it restates the
dependency graph inside the harness, over-cleans to stay safe, and still
invalidates only what someone remembered to name. Hash-based freshness
(`-Z checksum-freshness`) would retire the timestamp question outright but is
nightly, and D-24 pins a stable toolchain. A private target directory per
mutation is correct and unaffordable. Stamping a timestamp explicitly after a
copy was rejected as the same write by a longer route, and it is the only
reading under which D-233's portability objection holds.

**Consequences.** The fixture is a three-crate chain whose value is inlined
twice, and both checks assert the printed value rather than the bytes on disk,
because byte equality passes under this defect. One check restores a mutation,
the other re-copies a tracked workspace over a live build directory. Mutations
278 and 279 put the copy back in each path, and the resume mutation drops the
write where it used to drop the copy. Five clean-based mutations retire with
the code they broke, and `invalidate_package`, `clean_packages` and
`source_package` are gone.
