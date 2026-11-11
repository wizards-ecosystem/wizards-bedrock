# D-362 The remaining manifest and macro source-check evasions fail closed

Status: Accepted, 2026-10-07. Amends D-345, D-351 and D-360; closes PW-576.

**Decision.** Outside normal dependency sections, member-manifest assignment
keys must be bare keys or one bare key followed by `.workspace`. A bare
`dependencies` key is refused, as are inline `target` and `workspace` tables.
This deliberately bounded spelling keeps the offline reader independent of
Cargo's resolver and excludes the alternate routes to a runtime dependency
table. Existing modeled dependency sections retain their declaration rules.

The port visitor also reads macro token trees. An input naming `port` refuses
any raw-door identifier, star token or rename of `port` or `self`, including
grouped imports whose alias is used elsewhere, because matching only adjacent
`port::claim` tokens would still let a group hide a renamed door or module.
A string literal remains one token and never becomes code. This is a
conservative source restriction, not a macro expander or a type-resolution
claim. The public raw doors remain available to the self-test harness under
D-351.

**Rationale.** D-360 left two observed proof gaps: the manifest line reader
skipped assignments outside a normal dependency section, such as a root
`dependencies.hidden` key or a `dependencies` inline table under a target
header, and the port visitor could not see expressions inside macro
arguments. Neither limit needs a runtime or a Wizzy artifact to resolve.

**Alternatives.** A full TOML parser would admit more manifest spellings but
adds a dependency for a declaration style this repository does not need.
`cargo metadata` would give up the independent offline source check. Expanding
macros would add compiler machinery to a narrow prohibition whose token
spelling is sufficient to refuse the known escape.

**Consequences.** Persistent host mutations 1,597 (the outer-assignment
guard), 1,598 (macro inspection) and 1,602 (a hidden renamed raw door) each
fail their named witness,
`runtime_dependency_tables_cannot_hide_in_outer_assignments` or
`raw_port_claims_inside_macros_are_refused`. These are host proofs; the
complete current-source sweep was left to PW-001.
