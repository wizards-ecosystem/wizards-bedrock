# D-408 Windows canonical path spelling cannot reject the owned Deep qualifier

Status: Accepted, 2026-10-15. Amends D-406.

**Decision.** Redirect resistance still comes from the no-follow metadata walk
over every ancestor and the canonical directory result. Equality checks now
compare the canonical and supplied paths after both pass through the existing
Windows child-process path normalization, which removes only verbatim disk or
UNC prefixes. Different directories, relative components and reparse-point
ancestors remain refused. The host test
`an_ordinary_windows_codex_home_matches_its_canonical_verbatim_spelling`
accepts ordinary/verbatim spellings of one directory and rejects a different
home; mutation 2,062 restores raw `Path` equality and fails only that witness.
The owned Deep run must still pass every D-406 authentication, sandbox,
runtime, source and artifact check before it can publish qualification.

**Rationale.** The first owned Deep attempt stopped with `ChatGPT Codex home is
redirected` because Windows canonicalization had only added the verbatim `\\?\`
prefix to an ordinary `USERPROFILE\.codex`. Overriding `USERPROFILE` with a
verbatim path would hide the defect and change the environment whose provenance
D-406 records, and the ordinary spelling must still name the same canonical
directory, so the equality check stays.
