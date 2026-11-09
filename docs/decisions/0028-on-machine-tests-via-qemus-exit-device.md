# D-28 On-machine tests via QEMU's exit device

Status: Accepted (2026-08-12)

**Context.** The kernel had no automated check of any kind, and by construction
could not have one: `cargo test` cannot host a `#![no_std] #![no_main]` binary,
and the in-QEMU `#[test_case]` harnesses need nightly's
`custom_test_frameworks`, which D-01 rules out. Phase 1 got away with this
because a banner can be verified by looking at it. Phase 2 cannot — nobody
eyeballs an interrupt handler.

**Options.** (a) Keep extracting logic to `libs/` and accept that anything
touching hardware is untested. (b) Adopt nightly for `custom_test_frameworks`.
(c) A `selftest` cargo feature plus QEMU's `isa-debug-exit` device.

**Decision.** (c). A kernel built with `--features selftest` runs
`kernel/src/selftest.rs` after boot and writes its verdict to port `0xF4`;
QEMU exits with `(value << 1) | 1`, so 0x10 becomes 33 and 0x11 becomes 35.
`cargo xtask test-boot` builds that kernel, boots it headless under a timeout,
and maps the exit code to a pass or a failure. It is part of `cargo xtask ci`.

**Rationale.** Writing to a port is ordinary port I/O through an already-safe
`Port`, so this needs no nightly feature, no custom test runner, and no `unsafe`
in the kernel. It is the only mechanism that can test interrupt entry, paging,
and context switching — all of which arrive in the next two phases.

**Consequences.** The feature is never enabled in a shipped image, and
`debug_exit::report` is a no-op when the device is absent, so a self-test kernel
booted on real hardware still reaches its idle loop rather than hanging. The
harness treats a hang as a failure via its own timeout, which is what a panic
during boot looks like from outside. **At adoption**, six checks existed —
serial and console bring-up, framebuffer geometry, non-overlapping colour
masks, port-claim exclusivity, and lock exclusion. That total is a historical
snapshot; the latest phase close records the current total. The harness was
verified by breaking one and confirming it goes red.

**Not covered.** Anything whose failure is visual rather than logical. The
VMware boot of D-18 remains the definition of done for a phase.
