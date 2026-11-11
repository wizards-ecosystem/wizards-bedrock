# D-176 A boot target without the device is not a second boot target

Status: Accepted, 2026-08-25.

**Decision.** The first VMware boot of the green Phase 6 kernel failed 63
checks, a measurement that had never been taken, for two reasons: the VMX
template declared only a SATA boot disk, so the NVMe checks that `012` §2 chose
the controller for could not run, and the clock refused to calibrate without
saying why.

**The VMware machine gets the controller Phase 6 drives.** `qemu::nvme_image` is
one function both runners call, so the namespace behind VMware's second VMDK is
byte-for-byte the file QEMU's `-drive` points at: one corpus, one builder, two
machines. The VMX test asserts the controller is present, so this cannot
silently revert.

**The clock says which guard tripped and what it measured.** `measure` wraps a
`Result<Calibration, Refusal>` with every refusal named, and the two measured
rates are recorded whether or not they are believed, because "which number was
wrong" is the rest of the diagnosis. There are seven refusals: a crystal that
stepped, a window that never closed, a counter that read backwards, an
overshoot, a rate outside belief, no measurement at all, and `Implausible`
reached by five believable windows that never agreed within
`AGREEMENT_DIVISOR`, the one an emulated crystal reaches first.

**The rule:**

> **A boot target that cannot present the device is not testing the driver, it
> is testing the absence.** D-18 makes a VMware boot every phase's definition of
> done because that machine reaches hardware QEMU's does not. A phase that adds
> a device must add it to *both* machines in the slice that adds it — otherwise
> the second target's silence is mistaken for agreement.

**Rationale.** Recording the VMware boot as partial with the storage checks
excused would be a green result whose exclusions live in prose, the claim this
project refuses. D-18's value is entirely in the checks QEMU cannot run. On the
diagnosis half, this is D-28's argument moved one step earlier: a **refusal that
cannot be attributed** is worth less than a crash, because it looks like
information and is not.

**Consequences.**

- `cargo xtask vmware` builds a second VMDK and copies the namespace into it.
- A machine check sweeps every refusal and requires it to decode as the one that
  was recorded, because a diagnosis that names the wrong guard is worse than one
  that names none. Mutation 65 breaks it.
- Phase 6's VMware boot is **not** recorded as done until it is re-run against a
  machine with the controller. `014`'s close says so rather than claiming a boot
  it did not get.
