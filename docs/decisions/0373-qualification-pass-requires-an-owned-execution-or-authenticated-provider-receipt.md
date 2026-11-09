# D-373 Qualification PASS requires an owned execution or authenticated provider receipt

Status: Accepted (2026-10-09), amends D-330, D-342, D-344 and D-347; closes PW-582

**Context.** Codex Security occurrence `occ_b06f0364edc18c50bc4df22d`
identified that both exceptional qualification paths trusted files in a
caller-selected directory. `qualify-mutants-grade --seal` replayed internally
consistent cargo-mutants JSON, and `qualify-security-deep --seal` replayed
internally consistent zero-finding scan artifacts. In both cases an attacker
with the operator's file authority could manufacture the complete directory,
including its hashes and claimed provider fields, and cause `xtask` to write
`result=pass`. Source binding, strict schemas and semantic replay detect stale
or malformed bytes; they do not authenticate who performed the computation.

**Decision.** Qualification format v5 and evidence format v9 invalidate every
record and bundle from the unauthenticated formats. The `--seal` action is
removed from both tasks. `qualify-mutants-grade --run` is now the only grade
authority. It resolves the pinned cargo-mutants and cargo-nextest identities,
creates an exclusive private output directory under
`target/qualification-runs/cargo-mutants`, and directly starts the exact
multi-day command through the bounded-process owner. On Windows the existing
D-347 job contains the complete descendant tree; all hosts retain the finite
deadline and output cap. The command has exact concurrency, nextest-first
`PATH` and source-bound request fields. Before every campaign, inventory and
doctest command, `xtask` removes ambient Cargo, Rust, rustup, nextest, mutants,
target-runner, temporary-directory and C/C++ tool-selection variables; supplies
one private configuration-free Cargo home; pins `TEMP`, `TMP` and `TMPDIR` to
an exclusive child of the owned campaign; rejects Cargo configuration on that
generated discovery chain and permits only the exact host-dlltool compatibility
setting above the repository; and sets exact Cargo, rustc, rustdoc, host linker
and dlltool paths whose lengths and SHA-256 identities enter the v5 request.
The exact source and every executable are remeasured after the final
inventory and doctest use and immediately before PASS. A nonzero exit, changed
source, changed tool, incomplete output or independent post-run inventory
mismatch preserves the scratch directory and creates no record.

Non-caught dispositions are no longer accepted from the result directory.
They live in tracked `xtask/mutants-adjudications.json` and bind the exact
mutant name, summary and diff. Its path and digest enter the request, its bytes
must remain unchanged through the run, and every observed non-caught result
must have exactly one allowed, nonempty-rationale disposition. Only the
`MutantsAttempt` capability type can enter the observed-campaign PASS door;
the separate `DeepAttempt` type has no such method. A
successful record copies the six fixed artifacts into the qualification root
and removes only the canonical direct child that `xtask` reserved.

Codex Security Deep has no equivalent local execution path and the provider
artifacts expose no signature or authenticated retrieval receipt. Its
`--begin` action still publishes the source-bound request, and
`--verify=<DIR>` may report structural and semantic consistency for operator
diagnosis, but verification first invalidates any legacy pass and never writes
one. Both online evidence construction and offline evidence verification
reject Deep records through one shared authority boundary with a fixed
missing-authentication error. The phase-close
gate therefore remains deliberately blocked until the provider supplies a
receipt authenticated over the request/challenge, source revision, scan
identity, scope, findings, coverage and report, or `xtask` can invoke and fetch
the scan through an authenticated provider channel.

**Proof required.** Parser tests must reject `--seal` for both tasks and keep
the remaining actions task-local. The Deep diagnostic path must contain no
record commit; Deep and mutation-grade attempts must remain separate
capability types; and the sole Deep authority boundary must stop its protected
continuation before both online and offline consumers can read unsigned
records. Grade tests must prove the exact command, cleared and pinned
environment, bounded owner, output path, final post-use identity checks,
owned temp discovery chain, tracked policy digest and exact-diff adjudications.
Failed campaigns, changed source or tools, caller-supplied policies and nested
cleanup targets must fail closed. Each authority predicate receives an exact
retained red mutation.

**Evidence.** The host implementation compiles with qualification v5 and
evidence v9. Mutations 1,743 through 1,758 cover the retired seal spelling,
sole Deep authority boundary, hostile compiler environment, exact toolchain
contract, parent and scratch Cargo configuration, owned temporary directory,
bounded process owner, successful exit, stable source and tools, tracked policy
bytes and digest, exact mutant diff, direct-child cleanup and final post-use
remeasurement. Final focused, ordinary-gate and fresh
post-patch security results are recorded by the closing commit rather than
claimed in advance here.

**Alternatives.** A nonce in a request is still forgeable when the same caller
writes the result. Hashes authenticate bytes, not their producer. A provider
name or version field is a claim inside the unauthenticated document. File
permissions and a caller-selected path do not separate the operator from the
verdict. Keeping an emergency `--seal` flag would preserve the vulnerability.
Treating a semantic Deep check as a release PASS would hide the absent trust
anchor instead of repairing it.

This decision's attacker-controlled directory is the supported input to the
removed seal operation. It is not a claim that a checksummed record under
`target/` authenticates against another process with the operator account's
concurrent filesystem authority. Such a process can rewrite the checkout,
toolchain and every local qualification kind, so the project-wide trust
boundary remains an exclusive clean run under that account; evidence hashes
detect later transport or storage substitution. Cargo-mutants needs no remote
provider receipt because `xtask` owns its live execution inside that boundary.
Deep remains fail-closed because its computation still occurs outside it.
