# D-135 The boot that was every phase's definition of done checked nothing

Status: Accepted, 2026-08-25.

**Decision.** `cargo xtask vmware` builds with `SELFTEST_FEATURE`. The boot
reports a verdict on the serial log the way the gate's does, and the task
prints the two things a human must do that the QEMU harness does for itself:
press `a` when the keystroke prompt appears (`await_key` allows ten seconds,
after which that one check goes red and names itself) and read the verdict.

The harness's exit codes are **derived** from the kernel's rather than
restated: `the_exit_codes_are_the_kernels_doubled` reads `PASSED`/`FAILED` out
of `debug_exit.rs` and applies QEMU's own `(value << 1) | 1`. A twin that agrees
with itself is not a check (D-124). `the_vmware_image_is_built_with_the_checks_in_it`
reads the dispatch's source and requires the feature.

**Rationale.** D-18 makes a VMware boot **every phase's definition of done**,
and it built with an empty feature list, so none of the checks ran there: the
*procedure* was measuring a description of verification. A hand-kept exit-code
twin could let one hex digit make every mutation report "inconclusive", D-28's
guarantee switched off with the gate green. The whole self-test runs, not a
hardware-only subset, because the checks that most repay real hardware are not
knowable in advance, and a subset needs a second image and a second thing to
keep in step.

**Consequences.**

- The VMware image is a self-test image. It is not the shipped kernel: that is
  `boot-check`'s job, in the gate, on QEMU. A phase close that wants both runs
  two boots, and should.
- No prior VMware result is retroactively worth more. The next close is the
  first one whose VMware line means what the ritual says it means.

**Left owed.** `image::build` still takes `&[&str]`, so any future caller can
make the same mistake in a new place and only the one call site is guarded.
A typed `Kernel::{Shipped, SelfTest}` would make the mistake unspellable and is
the real fix; it is not done here.
