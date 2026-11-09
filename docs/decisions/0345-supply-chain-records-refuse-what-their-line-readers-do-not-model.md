# D-345 Supply-chain records refuse what their line readers do not model

Status: Accepted (2026-09-28), amends D-220; closes PW-556

**Context.** Three retained Daybreak findings reproduce in D-220's offline
supply-chain verification. The lock reader reduced a package's `source` to
whether it began with `registry+`, so an alternate registry's package was
attributed to crates.io and a Git dependency, having no registry flag, dropped
out of the inventory comparison. The member-manifest reader recognized
`[dependencies]` and `[target.<cfg>.dependencies]` sections but skipped a
detailed `[dependencies.<name>]` table, so a runtime dependency declared that
way never entered the runtime closure. The notices check accepted a runtime
component when its name and its version each appeared anywhere in the notices
document, so unrelated prose could satisfy it.

**Decision.** A locked package's source must equal the crates.io index source
exactly, which D-275's API audit already requires; any other source is
refused. The manifest reader refuses the declaration shapes it does not model:
a detailed dependency table, a dotted dependency key other than `.workspace`
and a `package` rename. Each runtime component's notice entry opens with its
exact inventory name and version as two backticked fields, and the set of
those entries must equal the runtime inventory rows with no entry repeated.
The notices now use that form.

**Evidence.** On 2026-09-28 fixtures refused an alternate registry, a sparse
index and a Git source; both detailed-table forms, a dotted key and two
spellings of a rename; and a prose decoy, a wrong version, an extra entry and
a repeated entry in the notices. The real lock file, member manifests and
notices passed. Mutations 1,503 through 1,509 each weaken one rule and each
failed its exact witness before byte-for-byte restoration.

**Alternatives.** Replacing the line readers with `cargo metadata` was
rejected for this check because D-220 keeps supply-chain verification offline
and independent of Cargo's resolver state. A complete TOML parser was rejected
because the project's manifests use one small declaration style, and refusing
every other shape keeps the reader's assumptions explicit.
