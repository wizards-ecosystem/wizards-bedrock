# D-24 The toolchain pin is exact

Status: Accepted, 2026-08-12.

**Decision.** `channel = "1.97.1"`, `profile = "minimal"`. D-17's bullet is
corrected in place.

**Rationale.** Clippy gains lints every six weeks. With D-22's lint policy
gating the build, a floating channel lets CI go red because a toolchain
shipped, not because a commit did. A reproducible OS build is worth more
than automatic updates. Bump on purpose.

**Consequences.** `components` lists only `rustfmt` and `clippy`. Naming
`rust-mingw` breaks non-Windows hosts, because rustup resolves `components`
against whatever host it runs on, and rustup installs it with the
windows-gnu host toolchain regardless of profile. Anything host-specific
belongs in the workflow or the README, never in `rust-toolchain.toml`, which
every host reads.
