# D-362 The remaining manifest and macro source-check evasions fail closed

Status: Accepted, 2026-10-07. Amends D-345, D-351 and D-360; closes PW-576.

**Context.** D-360 left two observed proof gaps. The manifest line reader
skips assignments outside a normal dependency section, including a root
`dependencies.hidden` key and a `dependencies` inline table under a target
header. The port visitor cannot see expressions inside macro arguments.
Neither limit needs a runtime or a Wizzy artifact to resolve.

**Decision.** Outside normal dependency sections, member-manifest assignment
keys must be bare keys or one bare key followed by `.workspace`. A bare
`dependencies` key is refused, as are inline `target` and `workspace` tables.
This deliberately bounded spelling keeps the offline reader independent of
Cargo's resolver and excludes the alternate routes to a runtime dependency
table. Existing modeled dependency sections retain their declaration rules.

The port visitor also reads macro token trees. An input naming `port` refuses
any raw-door identifier, star token or rename of `port` or `self`, including
grouped imports whose alias is used elsewhere. Matching only adjacent
`port::claim` tokens would still let a group hide a renamed door or module.
A string literal remains one token and never becomes code. This is a
conservative source restriction, not a macro expander or a type-resolution
claim. The public raw doors remain available to the self-test harness under
D-351.

**Proof required.** Host fixtures cover root and target key assignments,
quoted and dotted spellings, inline target tables, nested macros, grouped
imports and literal decoys. Persistent mutations remove the outer-assignment
guard and the macro inspection separately and hide a renamed raw door; each
must fail its named host witness before exact restoration. The ordinary gate
must pass before this slice is committed.

**Evidence.** On 2026-10-07 the new host witnesses passed. Mutation
1,597 bypassed outer-assignment validation and failed
`runtime_dependency_tables_cannot_hide_in_outer_assignments`; mutation 1,598
removed macro tokens from inspection and failed
`raw_port_claims_inside_macros_are_refused`. A grouped renamed-import fixture
first failed the adjacent-token guard; the restored token inventory passed
it, and mutation 1,602 hid the renamed door and failed that same witness.
Mutation 1,598 failed again against the complete fixture set. Each command
selected one test, returned exit 101 with that exact test marked `FAILED`, and its source was
restored byte for byte. These are host proofs; the complete current-source
sweep remains PW-001.

**Alternatives.** A full TOML parser would admit more manifest spellings but
adds a dependency for a declaration style this repository does not need.
`cargo metadata` would give up the independent offline source check. Expanding
macros would add compiler machinery to a narrow prohibition whose token
spelling is sufficient to refuse the known escape.
