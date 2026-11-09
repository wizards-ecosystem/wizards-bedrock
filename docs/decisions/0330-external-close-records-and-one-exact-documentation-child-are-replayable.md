# D-330 External close records and one exact documentation child are replayable

Status: Accepted, 2026-09-28. Amends D-218, D-244, D-246, D-262 and D-324; advances PW-009, PW-010, PW-025, PW-026 and PW-028.

**Context.** The final independent `xtask` mutation grade and managed Codex
Security Deep campaign take external tools, human authority and potentially
days, but the qualification bundle had no typed record for either. A request
file alone could not make a partial cargo-mutants run complete, version text
could not bind executable bytes, and a prose scan report could not substitute
for the provider's sealed JSON. Requiring every long qualification to run
again after the phase-close prose recorded its exact results would make the
closing documentation invalidate the evidence it described. Copying a parent
pass into a child, accepting any descendant or selecting an older passing
mutation report after a newer failure would instead destroy D-244's source and
latest-result authority.

**Decision.** Qualification format v3 adds two closed artifact classes and
deliberately invalidates earlier qualification records; evidence format v7
separately invalidates v6 bundles. `qualify-mutants-grade --begin`
publishes the clean source, cargo-mutants 27.1.0 identity, pinned
`cargo-nextest` release commit, download URL, archive and executable digests,
resolved paths, exact one-worker environment, full-grade command, independent
`--list --json --diff --no-times` command and zero-doctest command.
`--seal=GRADE_DIR` accepts only bounded direct regular files, requires the
independent name/diff inventory to equal the run inventory, one successful
baseline, one typed outcome per candidate, exact nextest Build/Test arguments
with a nonempty executable plus exact phase/status relations, completed UTC
timestamps, recomputed counters, allowed
per-outcome adjudications and an exact successful zero-doctest transcript. The
seal invocation declares `external-operator-v1` execution authority. This is a
human trust boundary like a VMware boot: `xtask` proves the sealed artifacts'
internal consistency and current tool bytes, but does not claim that it
spawned, timed out or contained cargo-mutants' delegated process tree.

`qualify-security-deep` retains the provider's v1 scan manifest, findings and
coverage JSON plus the provider's opaque UTF-8 report. The sealed result must be
completed for the exact Git revision and full `.` scope, its manifest hashes
must reproduce the other canonical documents, and coverage must be
`deep_repository`, repository-inventoried and complete with no surfaces,
explicit exclusions, deferred work or `openQuestions`. This first strict
record admits zero findings at every severity; there is no hidden disposition
or deferral ledger. The begin request names the managed-read-only,
variance-reducing multi-pass and canonical-rerun procedure, while the provider
artifacts attest to the final aggregate rather than the operator's launch history.

Evidence format v7 consumes both v3 records. Ordinarily every qualification
source equals the evidence source. A phase-close may instead use exactly one
`parent-docs-only-v1` child: it must be a direct single parent, and its nonempty
raw Git diff may contain only sorted, unique `M` rows from mode 100644 to
100644 for `README.md`, `docs/004-decision-log.md`, `docs/014-storage.md`,
`docs/016-pre-wizzy-foundation.md` and
`docs/017-pre-wizzy-hostile-surface-audit.md`. The manifest binds the parent
commit/tree, changed paths and the raw diff digest including both blob IDs.
Before copying, verification acquires the ordinary global mutation-sweep lock,
selects the newest immutable report in the parent's commit/registry state and
requires that exact report to be the qualified pass; the lock lives through
evidence publication. Merges, additions, deletions, renames, mode/type changes,
source edits, dirty staged or untracked state and a newer failed report refuse
the relation. The child reruns its ordinary gate and documentation contracts,
not the parent's externally operated qualifications.

**Evidence.** Bounded parser fixtures reject interrupted or subset grades,
inventory/diff disagreement, contradictory outcome phases, wrong nextest arguments,
unadjudicated outcomes and doctest output carrying a later error. Deep fixtures
reject a foreign revision, partial coverage, bad artifact digest and every
remaining finding. Relation fixtures reject a merge or wrong parent, additions
and nonallowlisted paths, bind changed blob IDs, and prove that a newer failed
parent report supersedes an older pass while the selected pass holds the sweep
lock. Sixteen retained mutations independently weakened inventory, request,
phase/status/argv, doctest, Deep source/finding, closure-parent/path and newest
report selection checks; each made its named host witness red before exact
restoration. No real cargo-mutants grade, Deep scan, VMware boot, complete
mutation sweep or phase close is claimed by this decision.

**Alternatives.** Letting `xtask` launch cargo-mutants was rejected because its
portable direct-child helper does not own nextest/cargo/rustc descendants and
would fire PW-013. Treating the begin request as execution proof was rejected;
it records the command the trusted operator must attest at seal. Importing raw
multi-gigabyte log trees or a self-authored digest list was rejected because
the former is disproportionate and the latter is not replayable. Treating
`report.md` as the Deep verdict was rejected in favor of provider-sealed JSON.
Allowing arbitrary documentation descendants or reconstructing a historical
Git index was rejected in favor of one exact parent relation plus the already
source-bound immutable mutation record and locked newest report.

**Amended by D-342.** The command above made cargo-mutants write one directory
deeper than the seal read, so no real run could seal. The seal now reads
`<dir>/mutants.out`, runs the independent inventory and the doctest command
itself and pins the grader's digest. The sixteen retained mutations above
covered part of this decision's checks; mutations 1,427 through 1,450 retain
the rest, and two checks that could not fail are removed.

**Amended by D-344.** Qualification v4 and evidence v8 add the identity of the
Git executable that established each source; the artifact classes and the
closing-child relation above are unchanged.
