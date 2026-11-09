# D-408 Windows canonical path spelling cannot reject the owned Deep qualifier

Status: Accepted, 2026-10-15. Amends D-406.

**Context.** The first owned Deep attempt on the frozen D-407 candidate stopped
before authentication or scan start with `ChatGPT Codex home is redirected`.
The home and every ancestor were ordinary directories. Windows canonicalization
had only added the verbatim `\\?\` prefix, while the guard compared that result
directly with the ordinary `USERPROFILE\.codex` spelling. The qualifier
therefore rejected one path expressed in two equivalent Windows forms.

**Decision.** Redirect resistance still comes from the no-follow metadata walk
over every ancestor and the canonical directory result. Equality checks now
compare the canonical and supplied paths after both pass through the existing
Windows child-process path normalization, which removes only verbatim disk or
UNC prefixes. Different directories, relative components and reparse-point
ancestors remain refused.

**Proof.** The focused host test
`an_ordinary_windows_codex_home_matches_its_canonical_verbatim_spelling`
accepts ordinary/verbatim spellings of one directory and rejects a different
home. Mutation 2,062 restores raw `Path` equality and fails only that witness.
The owned Deep run must still pass every D-406 authentication, sandbox,
runtime, source and artifact check before it can publish qualification.

**Alternatives.** Overriding `USERPROFILE` with a verbatim path for one launch
was rejected because it hides a release-tool defect and changes the environment
whose provenance D-406 records. Dropping the equality check was rejected
because the ordinary spelling still must name the same canonical directory.
