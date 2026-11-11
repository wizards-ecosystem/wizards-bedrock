# D-28 On-machine tests via QEMU's exit device

Status: Accepted, 2026-08-12.

**Decision.** A `selftest` cargo feature plus QEMU's `isa-debug-exit`
device, rather than nightly's `custom_test_frameworks` (D-01 rules it out)
or leaving everything that touches hardware untested. A kernel built with
`--features selftest` runs `kernel/src/selftest.rs` after boot and writes
its verdict to port `0xF4`. QEMU exits with `(value << 1) | 1`, so 0x10
becomes 33 and 0x11 becomes 35. `cargo xtask test-boot` builds that kernel,
boots it headless under a timeout, and maps the exit code to a pass or a
failure. It is part of `cargo xtask ci`.

**Rationale.** `cargo test` cannot host a `#![no_std] #![no_main]` binary.
Writing to a port is ordinary port I/O through an already-safe `Port`: no
nightly feature, no custom test runner, no `unsafe` in the kernel. It is the
only mechanism that can test interrupt entry, paging and context switching.

**Consequences.** The feature is never enabled in a shipped image, and
`debug_exit::report` is a no-op when the device is absent, so a self-test
kernel on real hardware still reaches its idle loop instead of hanging. The
harness's own timeout makes a hang a failure; that is how a boot panic looks
from outside. Breaking one check turned the harness red. The latest phase
close records the current check total.

**Not covered.** Anything whose failure is visual, not logical. The VMware
boot of D-18 remains the definition of done for a phase.
