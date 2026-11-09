# D-135 The boot that was every phase's definition of done checked nothing

Status: Accepted (2026-08-25)

**Context.** Three adversarial audits were run against Phases 0–5 to test the
claim that they were finished and polished. The soundness pass returned the
result that matters most — *no path from safe `kernel/` code, or from ring 3,
to undefined behaviour is reachable on today's kernel* — and the other two
returned roughly forty checks that cannot fail and a long list of prose the
code contradicts.

Two of those findings are about the **verification apparatus itself**, and
they are the ones this entry is for. A defect in a check costs one property; a
defect in the thing that runs the checks costs all of them.

**The VMware boot ran none of the checks.** `Task::Vmware` built its image
with `image::build(&paths, profile, &[])` — an empty feature list, so no
`selftest` feature, so none of the 137 checks were in it. `vmware.rs` mentions
neither `selftest` nor `verdict`; it wraps a disk and prints a path.

D-18 makes a VMware boot **every phase's definition of done**. `AGENTS.md` says
it "is not decoration — it reaches SMEP/SMAP/UMIP, the compacted `XSAVEC` path,
a real x2APIC and a processor that reports `MD_CLEAR`, none of which the gate's
QEMU does." Every word of that is true about the *machine*. None of it was
being checked *there*. Five phase closes cited that boot as evidence, and what
it evidenced was that the shipped kernel comes up and settles — which is
`boot-check`, on hardware.

**One character could disable the mutation sweep, and the gate would stay
green.** `qemu::SELFTEST_PASSED`/`SELFTEST_FAILED` are 33 and 35: QEMU's
`(value << 1) | 1` applied to `debug_exit.rs`'s `PASSED = 0x10` and
`FAILED = 0x11`. They were "kept in step" by a comment saying so.
`the_serial_contract_matches_the_kernel_source` reads `selftest.rs`, `main.rs`
and `panic.rs` — not the file that writes the port.

Change `FAILED` to `0x12` and `cargo xtask ci` passes, because every boot in
the gate is a *passing* one and `PASSED` is untouched. But `mutate` can no
longer tell a red verdict from a broken build, so all twenty-one mutations
report "inconclusive" — D-119's tool, and with it D-28's guarantee, switched
off by a hex digit with nothing to say so.

**Decision.**

`cargo xtask vmware` builds with `SELFTEST_FEATURE`. The boot now reports a
verdict on the serial log the way the gate's does, and the task prints the two
things a human must do that the QEMU harness does for itself: press `a` when
the keystroke marker appears — `await_key` allows ten seconds, after which
that one check goes red and names itself — and read the verdict.

The harness's exit codes are **derived** from the kernel's rather than
restated: the test reads `PASSED`/`FAILED` out of `debug_exit.rs` and applies
QEMU's own doubling. A twin that agrees with itself is not a check (D-124).

Both are held by host tests, and both were broken once and watched go red.

**Rationale.** The VMware finding is the more serious and the less obvious,
because nothing about it looked wrong: the task works, the VM boots, the serial
log fills, and a human reading it sees a healthy kernel. What was missing was
the one thing the phase-close ritual claimed to be getting from it. That is
D-124's shape — a check measuring a description — one level up: the *procedure*
was measuring a description of verification.

Why the whole self-test rather than a hardware-only subset: the checks that
would most repay running on real hardware are not knowable in advance. The
compacted `XSAVEC` path and a real x2APIC are the named ones; the value is in
the ones nobody predicted. And a subset needs a second image, a second feature
and a second thing to keep in step, which is what produced the exit-code twin.

**Consequences.**

- The VMware image is a self-test image. It is not the shipped kernel — that is
  `boot-check`'s job, in the gate, on QEMU. A phase close that wants both now
  runs two boots, and should.
- `the_vmware_image_is_built_with_the_checks_in_it` reads the dispatch's source
  and requires the feature. A typed `Kernel::{Shipped, SelfTest}` at
  `image::build` would make the test unnecessary by making the mistake
  unspellable, and is the better fix; this is the one that fits in the commit
  that found it.
- `the_exit_codes_are_the_kernels_doubled` derives 33 and 35 from `0x10` and
  `0x11`. Changing either without the other now fails `cargo test` in
  milliseconds instead of silently retiring the mutation sweep.
- No prior VMware result is retroactively worth more. The Phase 5 close's boot
  showed what it showed; this entry does not upgrade it, and the next close is
  the first one whose VMware line means what the ritual says it means.

**Left owed.** `image::build` still takes `&[&str]`, so any future caller can
make the same mistake in a new place and only the one call site is guarded.
The typed-kernel refactor is the real fix and is not done here.

The audits returned far more than these two — roughly forty checks that cannot
fail, four live code defects, and several ADR citations pointing at the wrong
decision. Those are being worked separately; this entry claims only the two
findings about the apparatus.
