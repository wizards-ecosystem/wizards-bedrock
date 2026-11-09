# D-24 The toolchain pin is exact

Status: Accepted, 2026-08-12.

**Context.** D-17 recorded the toolchain as "pinned (1.97.x)" while
`rust-toolchain.toml` actually said `channel = "stable"`.

**Decision.** `channel = "1.97.1"`, `profile = "minimal"`. D-17's bullet is
corrected in place.

**Rationale.** Clippy gains lints every six weeks. With the lint policy of
D-22 gating the build, a floating channel means CI can go red because a
toolchain shipped, not because a commit did — and a reproducible OS build is
worth more than automatic updates. Bump deliberately.

**Consequences.** `components` lists only `rustfmt` and `clippy`. `rust-mingw`
was listed too at first, on the theory that the minimal profile might drop a
component the windows-gnu host needs to link — but naming it breaks CI outright,
because it does not exist for a Linux host and rustup resolves `components`
against whatever host it is running on. It is unnecessary anyway: rustup
installs it as part of the windows-gnu host toolchain regardless of profile
(verified with `rustup component list --installed` after the pin landed).
Anything host-specific belongs in the workflow or the README, never in
`rust-toolchain.toml`, which every host reads.
