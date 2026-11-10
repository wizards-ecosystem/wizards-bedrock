# D-67 The `unsafe_code` default is deny, and `wost` lifts it in one visible place

Status: Accepted, 2026-08-23.

**Context.** D-13 confines `unsafe` to `wost`, and every crate carried the
attribute saying so: thirteen `libs/` crates `forbid`, `kernel` denies, `wost`
said nothing. The 2026-08-23 research pass found the gap: `unsafe_code` is
*allow*-by-default in rustc, so the discipline was thirteen separate opt-ins,
not a workspace property. A later crate that forgot the attribute would permit
`unsafe` silently.

**Decision.** `unsafe_code = "deny"` in `[workspace.lints.rust]`, inherited by
every member through the `[lints] workspace = true` they already declare.
`wost` lifts it with a single `#![allow(unsafe_code)]` at its crate root.
`libs/` keeps `forbid`, which is stricter and cannot be lifted locally.
`xtask`, which takes no workspace lints (D-22), carries `#![forbid]`
in-source; the 2026-08-23 audit found it unguarded and closed it that day.

**Rationale.** The default now matches the rule, and the exception is one
greppable line in the crate that exists to be it, not an absence of lines
everywhere else. Both directions were verified first: with the workspace lint,
an `unsafe` block in a crate missing `forbid` is an error; without it, the
same code compiles clean.

Workspace-level `forbid` was considered and rejected. `kernel` cannot use it:
`wost::entry!` expands `#[allow(unsafe_code)]` boot glue there, and an `allow`
cannot lift `forbid` even inside a macro expansion (D-21's finding,
unchanged).

**Consequences.**

- The workspace, not review, catches a new `libs/` crate that forgets
  `#![forbid(unsafe_code)]`; broken once to prove it.
- The `Cargo.toml` comment about per-crate escalation names both directions:
  the `forbid` in `libs/` and the one `allow` in `wost`.
- `wost`'s crate root gains an attribute that reads as a confession, as
  intended.
