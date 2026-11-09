# D-324 The `xtask` independent grade fails fast under one bounded worker

Status: Accepted (2026-09-27), amends D-246; advances PW-025

**Context.** D-246 left the large `xtask` grade open. Its current candidate
set is 3,453 mutants and its host suite contains 649 unit tests, several
of which compile adversarial workspaces. A four-worker `cargo-mutants 27.1.0`
discovery run exhausted this 32 GiB host and produced rustc allocation
failures, invalid metadata and a process stack-buffer-overrun status, so its
classifications are not evidence. One mutation worker still let libtest run
multiple high-memory tests together. Restricting both layers to one worker was
memory-safe, but its pristine test phase took 916.7 seconds, which projects to
roughly 36 days if repeated for every candidate.

`cargo-mutants 27.1.0` can invoke nextest without changing the generated
mutants. Nextest executes one test per process and stops scheduling after the
first failure. The pinned `cargo-nextest 0.9.145` probe, with one build job and
one test thread, caught two representative mutants in 158.9 seconds while
retaining safe memory headroom. Nextest does not execute doctests; an explicit
`cargo test -p xtask --doc --locked` found zero doctests in this package.

**Decision.** The closing `xtask` grade uses unmodified `cargo-mutants 27.1.0`
with the exact prebuilt `cargo-nextest 0.9.145` runner, `--test-tool nextest`,
one mutation job, one jobserver task, `CARGO_BUILD_JOBS=1` and
`NEXTEST_TEST_THREADS=1`. It runs from one source-frozen clean commit with the
ordinary pristine baseline enabled. The qualification record binds the
runner's release commit, download URL and SHA-256 hashes of both its archive
and executable. It also records the separate zero-doctest command; if that
command ever finds a doctest, the grade stops until that coverage is supplied
without pretending nextest ran it.

Calibration, interrupted, resource-corrupted and baseline-skipped runs are
labelled as such and cannot enter evidence. No compiler, QEMU, model or other
memory-heavy qualification runs beside the grade. Every non-caught outcome is
still adjudicated under D-246; changing the test runner changes neither a
mutant nor the standard for accepting its result.

**Rationale.** Fail-fast process isolation pays the full suite cost only for a
survivor while a caught mutant can stop at its first exact witness. Bounding
both build and test concurrency preserves enough memory that a tool failure is
distinguishable from a source verdict. Pinning and hashing the auxiliary
runner gives the optimization the same reproducibility as the grader itself.

**Alternatives.** Four mutation workers were rejected because the measured run
corrupted compiler outcomes. One mutation worker with ordinary libtest
parallelism was rejected because two large fixtures still exhausted memory.
Fully serialized libtest was rejected because its measured projection was
about 36 days. Skipping tests, the pristine baseline or candidate shards was
rejected because it would narrow D-246's complete grade. Treating nextest as a
doctest runner was rejected because it is not one.

**Consequences.** The runner and its provenance live under
`target/qualification/tools/cargo-nextest/`; the final source-bound output
remains under `target/qualification/cargo-mutants/<commit>/`. The two-mutant
probe is calibration only, not a PW-025 result. PW-025 and D-246 remain open
until a complete clean-commit grade and every required adjudication are
recorded.

**Amended by D-342.** The grader is pinned by its executable's SHA-256 as well
as its version, and the command passes `--output <dir>`, under which
cargo-mutants writes `mutants.out`.
