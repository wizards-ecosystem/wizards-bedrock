# D-233 Restoring mutation source invalidates its compiled package

Status: Accepted, 2026-08-29. Amends D-208 and D-232; advances PW-001.

**Decision.** Restoring or recovering a mutated compiled source invalidates
that package in the session's private Cargo target before another witness may
run. The mapping is exact: `kernel` names `wizard-kernel`, `wost` and `xtask`
name themselves, and `libs/<package>` names that library. Documentation,
models, fixtures and supply-chain inputs have no compiled package and require
no clean. The invalidation uses `cargo clean --package` inside the isolated
workspace and target; it never touches the user's ordinary build artifacts.

**Rationale.** The first successor run after D-232 reproduced the supposedly
incidental user-boundary failures causally. Mutation 12 changed `wost`; the
runner copied pristine bytes back from its snapshot and applied mutation 13 in
the kernel, and Cargo rebuilt only the kernel and linked the stale mutated
`wost`, so mutations 13 and 14 ran with the wrong page tables while source and
journal digests were correct. On Windows, `fs::copy` restored the snapshot's
old timestamp, so Cargo had no reason to discard the newer object. D-232's
bounded retry policy remains valid, but its attribution of these four
observations to incidental machine failure is superseded by this build-cache
cause.

**Alternatives.** Touching the restored file was rejected because timestamp
granularity and future-time handling differ by filesystem. Deleting the whole
private target after every mutation was rejected because it would discard all
unrelated dependencies and turn a bounded sweep into repeated cold builds.
Assuming the next mutation changes the same crate was rejected because the
observed transition was exactly from `wost` to the kernel.

**Consequences.** A host reproduction builds a mutated mini-`wost`, restores
its pristine source and requires every package artifact to be absent before
the next build; mutation 278 keeps it red without the invalidation. The aborted
277-entry run has no closing-evidence value, although its first twelve exact
outcomes remain diagnostic.

**Superseded in part by D-237 (2026-08-29).** The cause recorded here is
right, and D-237 acts on it directly: the remedy chosen instead — invalidating
the package by hand — treated a clock as a cache. What survives is the
diagnosis and the sequence that produced it.
