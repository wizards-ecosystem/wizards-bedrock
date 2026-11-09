# D-233 Restoring mutation source invalidates its compiled package

Status: Accepted, 2026-08-29. Amends D-208 and D-232; advances PW-001.

**Context.** The first successor run after D-232 reproduced the supposedly
incidental user-boundary failures in a causal sequence. Mutation 12 removed
`wost`'s address-space installation and Cargo built that changed dependency.
The runner then copied pristine `task.rs` bytes from its snapshot and applied
mutation 13 in the kernel. Cargo rebuilt only the kernel and linked the stale
mutated `wost`; mutations 13 and 14 consequently ran with the wrong page tables
and could not answer their own questions. The source and journal digests were
correct throughout. On Windows, `fs::copy` restored the snapshot's old
timestamp, so Cargo had no reason to discard the newer object. D-232's retry
could only repeat the contaminated build. Its bounded retry policy remains
valid, but its attribution of these four observations to incidental machine
failure is superseded by this reproduced build-cache cause.

**Decision.** Restoring or recovering a mutated compiled source invalidates
that package in the session's private Cargo target before another witness may
run. The mapping is exact: `kernel` names `wizard-kernel`, `wost` and `xtask`
name themselves, and `libs/<package>` names that library. Documentation,
models, fixtures and supply-chain inputs have no compiled package and require
no clean. The invalidation uses `cargo clean --package` inside the isolated
workspace and target; it never touches the user's ordinary build artifacts.

**Alternatives.** Touching the restored file was rejected because timestamp
granularity and future-time handling differ by filesystem. Deleting the whole
private target after every mutation was rejected because it would discard all
unrelated dependencies and turn a bounded sweep into repeated cold builds.
Assuming the next mutation changes the same crate was rejected because the
observed transition was exactly from `wost` to the kernel.

**Consequences.** A host reproduction builds a mutated mini-`wost`, restores
its pristine source and requires every package artifact to be absent before
the next build. Removing only the package invalidation left those artifacts
live and failed that exact test; mutation 278 retains the defect. The aborted
277-entry run has no closing-evidence value, although its first twelve exact
outcomes remain diagnostic. PW-001 now belongs to a clean 278-entry successor
run. No target code, unsafe operation, wire ABI or machine-check count changed.

**Superseded in part by D-237 (2026-08-29).** The cause recorded here is
right, and D-237 acts on it directly: the remedy chosen instead — invalidating
the package by hand — treated a clock as a cache. What survives is the
diagnosis and the sequence that produced it.
