# D-342 The `xtask` grade seals what cargo-mutants writes and witnesses its own completeness

Status: Accepted, 2026-09-28. Amends D-324, D-329 and D-330; advances PW-025; closes PW-551 and PW-552.

**Decision.** The published command passes `--output <dir>`, and the seal
reads `<dir>/mutants.out` through one constant that the command's test also
reads. The seal runs the independent `--list --json --diff --no-times`
inventory with the resolved grader and runs `cargo test -p xtask --doc
--locked` from the frozen checkout, each through the bounded helper with a
time and output limit, and records both results; no operator file stands in
for either. The request format `wizard-cargo-mutants-request-v2` names both as
seal-run and binds the grader's SHA-256, which must equal the reviewed
executable at begin and at seal. The doctest transcript must name exactly the
`xtask` package. The seal refuses a grade directory inside
`target/qualification` and refuses operator JSON that begins with a UTF-8 or
UTF-16 byte-order mark, naming the mark.

The reviewed grader is `cargo-mutants 27.1.0` from crates.io, installed by
`cargo install` with rustc 1.97.1 (`8bab26f4f`, 2026-07-14) in its release
profile: 15,363,104 bytes with SHA-256
`4ef442752564625eef9786739a1d57cc692263fca071dd16751f45ba2e322d9c`. Another
build of the same version needs its own decision.

D-330's grade timing, success count, recomputed caught counter, single
baseline and candidate bijection, its four adjudication rules, five
Deep-coverage rules, the manifest-digest and request-byte comparisons, the
parent-diff status, mode, order and framing rules and three CLI rules, and
D-329's rule for the checked MMIO slice body, each now have one retained
defect, as D-28 requires. Two checks that could not fail are removed: the Deep
`openQuestions` test, which followed an exact key-set check, and the
parent-diff parser's non-empty test, which followed framing that already
refuses a diff without one complete row. Cargo delegates to compiler and test
processes, so the two seal-run commands join the delegated commands whose
process trees PW-013 owns.

**Rationale.** D-330's command passed `--output <dir>/mutants.out`, but
cargo-mutants 27.1.0 creates `mutants.out` inside the directory it is given,
so no real run could seal; mutation 1 guarded the defect's direction and now
makes the command write one level deeper than the seal reads. The seal also
took completeness from operator files, so a subset run paired with a matching
subset inventory sealed as complete; it identified the grader by version text,
did not bind the doctest transcript to `xtask`, could remove a grade directory
inside the records, and met PowerShell 5.1's UTF-16 `>` output as an
unexplained syntax error. Mutations 1,422 through 1,426 retain those breaks,
mutations 1,427 through 1,450 the D-330 rules and mutations 1,451 and 1,452
the MMIO slice rule.

**Alternatives.** Keeping operator-supplied inventory and doctest files with
stricter parsing was rejected because a consistent subset still agrees with
itself. Accepting any executable that reports version 27.1.0 was rejected
because version text does not identify bytes. Launching the grade itself from
`xtask` stays with D-330: the seal's two commands are bounded to minutes,
while the grade runs for days under the operator's authority. Keeping the two
unreachable checks was rejected because a check that cannot fail still reads
as a claim.

**Consequences.** The operator redirects no output, and the seal takes one
inventory listing and one doctest build longer. A new cargo-mutants build
requires a decision that updates the pin. PW-025 remains REQUIRED until the
complete campaign is sealed.
