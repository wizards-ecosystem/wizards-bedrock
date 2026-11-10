# D-23 xtask is a library with a thin binary

Status: Accepted, 2026-08-12.

**Context.** `xtask/src/main.rs` had grown to 254 lines in one file, with
hardcoded paths, twelve fallible calls carrying no context, and no tests.
Its one arithmetic defect, a truncating cylinder division declaring a VMDK
geometry 1.25 MiB smaller than its extent, hid because nothing could
exercise it.

**Decision.** Split into `lib.rs` plus modules (`cli`, `paths`, `cargo`,
`tasks`, `image/{mbr,esp}`, `qemu`, `vmware`, `vendor`, `sha256`) with a
~15-line `main.rs`. Pure byte-layout and arithmetic live in functions that
take and return values, tested in-module. Package names, target triple and
profile are constants in `paths`, and every path derives from them.
Argument parsing is hand-rolled: twelve tasks and six flags are a testable
`match`, not a reason to take `clap` and three transitive crates.

**Consequences.** 42 host tests, including a byte-exact MBR assertion and a
cylinder-rounding regression test. `CARGO_TARGET_DIR` is honoured, and
`--debug` builds now find their artifact. `std::env::consts::EXE_SUFFIX`
and `QEMU`/`QEMU_DIR` overrides replace the hardcoded `.exe` and install
path, so the tooling is not Windows-only. New tasks: `check`, `clippy`,
`fmt`, `test`, `verify-vendor`, `ci`, `clean`. **A bare `cargo xtask` is now
an error printing usage.** The old default silently booted a virtual machine
whenever a task name was mistyped.
