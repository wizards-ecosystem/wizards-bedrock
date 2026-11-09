# D-340 Direct Cargo-subcommand probes name their entry point

Status: Accepted, 2026-09-28. Amends D-330; advances PW-025.

**Context.** The first current `qualify-mutants-grade --begin` attempt found
the pinned `cargo-mutants 27.1.0` binary on `PATH` but stopped at its version
probe. `cargo mutants --version` succeeded because Cargo invokes the external
binary with `mutants` as its first argument. The qualification code instead
started `cargo-mutants.exe --version` directly; the binary parsed that as the
top-level Cargo interface and rejected the otherwise valid flag before a
source-bound request could be written.

**Decision.** The direct identity probe uses the exact argument vector
`mutants --version`. One helper constructs that command, and its host test
observes the arguments through `Command::get_args`; tool resolution, executable
hashing, version parsing and every operator-facing `cargo mutants` command stay
unchanged. Retained mutation 1,413 restores the missing subcommand argument and
must make only that exact host test red.

**Evidence.** On 2026-09-28 direct execution with only `--version` exited 1
with `unexpected argument '--version'`, while direct execution with `mutants
--version` exited 0 and reported `cargo-mutants 27.1.0`. Replacing the helper's
two arguments with the defective one-argument form made only
`direct_cargo_mutants_probe_names_its_subcommand` fail, showing the observed
vector `['--version']` instead of `['mutants', '--version']`; exact restoration
made that named test pass. The complete independent grade and its sealed result
remain PW-025 rather than being inferred from a successful preflight.

**Alternatives.** Probing through `cargo mutants` was rejected because the
record binds and hashes the resolved external executable, so the identity
check should execute those same bytes. Accepting any successful version output
was rejected because a PATH wrapper would no longer prove the recorded binary.
Removing the preflight was rejected because it would let a multi-day run begin
with a grader the sealer must later refuse.
