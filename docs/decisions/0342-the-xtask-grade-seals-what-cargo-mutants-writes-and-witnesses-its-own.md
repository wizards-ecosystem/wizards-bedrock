# D-342 The `xtask` grade seals what cargo-mutants writes and witnesses its own completeness

Status: Accepted, 2026-09-28. Amends D-324, D-329 and D-330; advances PW-025; closes PW-551 and PW-552.

**Context.** An adversarial audit of D-330's grade workflow found that it
could not seal a real run. Its published command passed
`--output <dir>/mutants.out`, but cargo-mutants 27.1.0 creates `mutants.out`
inside the directory it is given, so a run that followed the command wrote
`<dir>/mutants.out/mutants.out` while the seal read `<dir>/mutants.out`.
Mutation 1 guarded the defect's direction: its witness asserted the doubled
path. The seal also took completeness from the operator. It accepted the
independent inventory and the doctest transcript as files placed beside the
run, so a subset run paired with a matching subset inventory would seal as
complete. The grader was identified by its version text while nextest was
hash-pinned, the doctest transcript was not bound to the `xtask` package, a
grade directory inside the qualification records could be removed when the
seal replaced its kind directory, and PowerShell 5.1's `>` writes UTF-16,
which reached the JSON parser as an unexplained syntax error.

The audit also found checks that had never failed once, which D-28 requires.
D-330's grade timing, success count, recomputed caught counter, single
baseline and candidate bijection, its four adjudication rules, five
Deep-coverage rules, the manifest-digest and request-byte comparisons, the
parent-diff status, mode, order and framing rules and three CLI rules had no
retained defect, and neither had D-329's rule for the checked MMIO slice body.
Two checks could not fail: the Deep `openQuestions` test followed an exact
key-set check that already required the key, and the parent-diff parser's
non-empty test followed framing that already refuses a diff without one
complete row.

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

Every check listed above now has one retained defect, and both unreachable
checks are removed. Cargo delegates to compiler and test processes, so the two
seal-run commands join the delegated commands whose process trees PW-013 owns.

**Evidence.** On 2026-09-28 a one-candidate cargo-mutants 27.1.0 run with
`--output <dir>` wrote `mutants.json` and `outcomes.json` under
`<dir>/mutants.out`. Mutation 1 now makes the published command write one
level deeper than the seal reads and fails the output-directory witness.
Mutations 1,422 through 1,426 accept any grader digest, accept another
package's doctest transcript, seal a directory inside the records, pass
UTF-16 JSON to the parser and read the grade directory itself. Mutations 1,427
through 1,450 each weaken one D-330 rule named above, and mutations 1,451 and
1,452 make the checked MMIO slice publish one word or use ordinary writes.
Each of the 32 failed its exact host witness and was restored byte for byte.

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
