# D-345 Supply-chain records refuse what their line readers do not model

Status: Accepted, 2026-09-28. Amends D-220; closes PW-556.

**Decision.** A locked package's source must equal the crates.io index source
exactly, which D-275's API audit already requires; any other source is
refused. The manifest reader refuses the declaration shapes it does not model:
a detailed dependency table, a dotted dependency key other than `.workspace`
and a `package` rename. Each runtime component's notice entry opens with its
exact inventory name and version as two backticked fields, and the set of
those entries must equal the runtime inventory rows with no entry repeated.
The notices now use that form.

**Rationale.** Three retained Daybreak findings reproduced in D-220's offline
verification. The lock reader only asked whether a `source` began with
`registry+`, so an alternate registry's package was attributed to crates.io
and a Git dependency dropped out of the inventory comparison. The manifest
reader skipped a detailed `[dependencies.<name>]` table, so a runtime
dependency declared that way never entered the runtime closure. The notices
check accepted a component whose name and version each appeared anywhere in
the document, so unrelated prose could satisfy it. Mutations 1,503 through
1,509 retain one break of each rule.

**Alternatives.** Replacing the line readers with `cargo metadata` was
rejected for this check because D-220 keeps supply-chain verification offline
and independent of Cargo's resolver state. A complete TOML parser was rejected
because the project's manifests use one small declaration style, and refusing
every other shape keeps the reader's assumptions explicit.
