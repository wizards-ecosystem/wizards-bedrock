# D-340 Direct Cargo-subcommand probes name their entry point

Status: Accepted, 2026-09-28. Amends D-330; advances PW-025.

**Decision.** The direct identity probe uses the exact argument vector
`mutants --version`. One helper constructs that command, and its host test
observes the arguments through `Command::get_args`; tool resolution, executable
hashing, version parsing and every operator-facing `cargo mutants` command stay
unchanged. Retained mutation 1,413 restores the missing subcommand argument and
must make only that exact host test red.

**Rationale.** Cargo invokes an external subcommand with `mutants` as its first
argument, so `cargo mutants --version` succeeds, but the qualification code
started `cargo-mutants.exe --version` directly, and the pinned
`cargo-mutants 27.1.0` parsed that as its top-level Cargo interface and
rejected the flag before `qualify-mutants-grade --begin` could write a
source-bound request. The complete independent grade and its sealed result
remain PW-025 rather than being inferred from a successful preflight.

**Alternatives.** Probing through `cargo mutants` was rejected because the
record binds and hashes the resolved external executable, so the identity
check should execute those same bytes. Accepting any successful version output
was rejected because a PATH wrapper would no longer prove the recorded binary.
Removing the preflight was rejected because it would let a multi-day run begin
with a grader the sealer must later refuse.
