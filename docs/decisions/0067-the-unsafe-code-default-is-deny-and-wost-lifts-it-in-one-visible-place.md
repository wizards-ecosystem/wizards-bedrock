# D-67 The `unsafe_code` default is deny, and `wost` lifts it in one visible place

Status: Accepted (2026-08-23)

**Context.** D-13 confines `unsafe` to `wost`, and every crate carried the
attribute that says so: thirteen `libs/` crates `forbid`, `kernel` denies,
`wost` said nothing. The gap the 2026-08-23 research pass found is that
`unsafe_code` is *allow*-by-default in rustc, so the discipline was thirteen
separate opt-ins rather than a property of the workspace. A crate added later
that simply forgot the attribute would permit `unsafe` in silence.

**Decision.** `unsafe_code = "deny"` in `[workspace.lints.rust]`, inherited by
every member through the `[lints] workspace = true` they already declare;
`wost` lifts it with a single `#![allow(unsafe_code)]` at its crate root.
`libs/` keeps `forbid`, which is stricter and cannot be lifted locally.
`xtask`, which takes no workspace lints (D-22), carries `#![forbid]` in-source
— found unguarded by the 2026-08-23 audit and closed the same day.

**Rationale.** The default now matches the rule. The exception becomes one
greppable line in the crate whose whole purpose is to be that exception,
rather than an absence of lines everywhere else. Verified in both directions
before the change was made: with the workspace lint, an `unsafe` block in a
crate missing `forbid` is an error; without it, the same code compiles clean.

`forbid` at the workspace level was considered and rejected — `kernel` cannot
use it, because `wost::entry!` expands `#[allow(unsafe_code)]` boot glue there
and `forbid` cannot be lifted by an `allow` even inside a macro expansion.
That is D-21's finding, unchanged.

**Consequences.**

- A new `libs/` crate that forgets `#![forbid(unsafe_code)]` is now caught by
  the workspace rather than by review. Broken once to prove it.
- The `Cargo.toml` comment about per-crate escalation now names both directions
  — the `forbid` in `libs/` and the one `allow` in `wost`.
- `wost`'s crate root gains an attribute that reads as a confession, which is
  the intent.
