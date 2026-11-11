# D-23 xtask is a library with a thin binary

Status: Accepted, 2026-08-12.

**Decision.** Split `xtask` into `lib.rs` plus modules (`cli`, `paths`,
`cargo`, `tasks`, `image/{mbr,esp}`, `qemu`, `vmware`, `vendor`, `sha256`)
with a ~15-line `main.rs`. Pure byte-layout and arithmetic live in functions
that take and return values, tested in-module. Package names, target triple
and profile are constants in `paths`, and every path derives from them.
Argument parsing is hand-rolled: twelve tasks and six flags are a testable
`match`, not a reason to take `clap` and three transitive crates.

**Rationale.** In one untested file, a truncating VMDK cylinder division hid
because nothing could exercise it.

**Consequences.** `CARGO_TARGET_DIR` is honoured.
`std::env::consts::EXE_SUFFIX` and `QEMU`/`QEMU_DIR` overrides replace the
hardcoded `.exe` and install path, so the tooling is not Windows-only. **A
bare `cargo xtask` is an error printing usage**, because a silent default
booted a virtual machine whenever a task name was mistyped.
