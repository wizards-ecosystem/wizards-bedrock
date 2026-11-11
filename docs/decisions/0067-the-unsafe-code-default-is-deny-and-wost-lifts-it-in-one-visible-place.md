# D-67 The `unsafe_code` default is deny, and `wost` lifts it in one visible place

Status: Accepted, 2026-08-23.

**Decision.** `unsafe_code = "deny"` in `[workspace.lints.rust]`, inherited
by every member through the `[lints] workspace = true` they already declare.
`wost` lifts it with a single `#![allow(unsafe_code)]` at its crate root.
`libs/` keeps `forbid`, which is stricter and cannot be lifted locally.
`xtask`, which takes no workspace lints (D-22), carries `#![forbid]`
in-source.

**Rationale.** `unsafe_code` is *allow*-by-default in rustc, so per-crate
attributes made the discipline a set of opt-ins a new crate could forget.
The default now matches the rule, and the exception is one greppable line in
the crate that exists to be it. Workspace-level `forbid` was rejected:
`wost::entry!` expands `#[allow(unsafe_code)]` boot glue into `kernel`, and
an `allow` cannot lift `forbid` even inside a macro expansion (D-21).

**Consequences.** The workspace, not review, catches a new `libs/` crate
that forgets `#![forbid(unsafe_code)]`. The `Cargo.toml` comment about
per-crate escalation names both directions: the `forbid` in `libs/` and the
one `allow` in `wost`.
