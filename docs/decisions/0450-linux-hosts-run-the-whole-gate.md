# D-450 Linux hosts run the whole gate

Status: Accepted, 2026-11-07. Amends D-275, advances PW-708.

**Decision.** On Windows hosts the audit keeps the sysroot `rust-lld`. On
other hosts it resolves `cc` from the absolute entries of `PATH`, binds the
canonical driver by SHA-256 in the compiler identity as it binds `rust-lld`,
and passes it as the host target's linker. The driver's collect2 needs `ld`, so
the audit takes the first such directory that also holds `ld`, binds the
canonical `ld` by SHA-256 the same way, and passes only that directory to the
driver through `COMPILER_PATH`. `PATH` stays toolchain-only. With no `cc` and
`ld` in one directory the audit refuses. This amends D-275's linker evidence.

The `libc` 0.2.189 build script joins the reviewed set under its `Cargo.lock`
checksum. Its 393 lines read `CARGO_CFG_*`, `LIBC_*`, `RUST_LIBC_UNSTABLE_*`
and `WIND_RELEASE_ID`. They run `$RUSTC --version` directly, since the audit
sets an empty `RUSTC_WRAPPER`. They try `emcc -dumpversion` on every target and
`freebsd-version` only when `LIBC_CI` is set, and neither tool is on the
audit's toolchain-only `PATH`. The script prints only `rustc-cfg`,
`rustc-check-cfg` and rerun lines and writes no files.

The `nix` 0.29.0 build script joins the same set under its `Cargo.lock`
checksum. It expands `cfg_aliases!`, which reads only `CARGO_CFG_*`, and prints
only `rustc-cfg` and `rustc-check-cfg` lines. It runs no tool, compiles nothing
and writes no files.

Host tests that need Windows paths compile only on Windows, and one test
proves the owned Security Deep campaign refuses elsewhere. That campaign stays
a Windows-host qualification.

**Rationale.** `cargo xtask ci` must pass on Linux hosts as well as on the
Windows release host. D-275's sysroot `rust-lld` is self-contained on
`x86_64-pc-windows-gnu`, but on a glibc host it finds no C runtime objects or
library search path, so every audited host link failed; Linux also activates
`libc` through `xtask`'s unix dependency and `nix` through `fs_at`, whose build
scripts the audit refused as unreviewed.

**Consequences.** Windows behaviour is unchanged. Mutation 2,452 makes a
Windows host select `cc` and fails the linker selection test. PW-708 still
needs a recorded Linux run of the whole gate before it relies on this.
