# D-176 A boot target without the device is not a second boot target

Status: Accepted (2026-08-25)

**Context.** Phase 6 closed green: 180 machine checks under QEMU, 64 mutations
each caught, the model clean. Then the first VMware boot of that kernel came
back `verdict v1 FAIL passed=117 failed=63`.

Sixty-three failures on a machine whose kernel passes 180 checks on another is
not a regression, it is a **measurement that had never been taken**. Two causes,
and each teaches something different.

**The VM had no NVMe controller.** `012` §2's entire argument for choosing NVMe
over AHCI was that it is the one storage controller with a published
specification that spans both of this project's boot targets — and the VMX
template written in Phase 2 declares a SATA disk to boot from and nothing else.
So Phase 6 was developed for ten slices against a machine that could satisfy it
and shipped to one that could not. The checks did exactly what they should:
`pci: 8 functions ... 0 nvme`, then ten dependent checks red.

**The clock refused to calibrate, and would not say why.** `measure` returns
`Option<Calibration>` and has six ways to produce `None` — a crystal that
stepped, a window that never closed, a counter that read backwards, an
overshoot, a rate outside belief, and no measurement at all. The warning printed
one sentence for all six: *no believable rate from any measurement*. That
sentence names the least likely of the six. Roughly forty checks depend on a
running clock, so one unexplained refusal is most of the remaining failures.

**Decision.** Three things.

**The VMware machine gets the controller Phase 6 drives.** `qemu::nvme_image`
becomes a function both runners call, so the namespace behind VMware's second
VMDK is byte-for-byte the file QEMU's `-drive` points at. One corpus, one
builder, two machines. The VMX test asserts the controller is present, so this
cannot silently revert.

**The clock says which guard tripped and what it measured.** `measure` becomes a
wrapper over a `Result<Calibration, Refusal>` with the six named, and the two
measured rates are recorded whether or not they are believed — because "which
number was wrong" is the rest of the diagnosis, and a refusal that keeps it to
itself costs another boot to learn.

**The rule, which is what this entry is for:**

> **A boot target that cannot present the device is not testing the driver, it
> is testing the absence.** D-18 makes a VMware boot every phase's definition of
> done because that machine reaches hardware QEMU's does not. A phase that adds
> a device must add it to *both* machines in the slice that adds it — otherwise
> the second target's silence is mistaken for agreement.

**Rationale.** The alternative was to let Phase 6's VMware boot be a partial
one, recorded with the ten storage checks excused. That is the shape of every
claim this project has spent six phases refusing: a green result whose
exclusions live in prose. D-18's value is entirely in the checks QEMU cannot
run, and a phase whose new device only exists on QEMU has none of that value.

On the diagnosis half: this is D-28's argument moved one step earlier. A check
that cannot fail is worth less than no check because it is also a claim; a
**refusal that cannot be attributed** is worth less than a crash, because it
looks like information and is not.

**Consequences.**

- `cargo xtask vmware` builds a second VMDK and copies the namespace into it.
  The VM is ~64 MiB larger and takes one file copy longer to generate.
- The VMX template gains three lines and a test that reads them.
- `kernel/src/time.rs` grows a `Refusal` enum and three statics. The warning is
  longer by a clause and a pair of numbers.
- **Seven refusals, not six.** `init` reaches `Implausible` a seventh way — five
  windows that were each believable and never agreed within `AGREEMENT_DIVISOR`
  — and on an emulated crystal that is the one it reaches first. It names
  itself too.
- A machine check sweeps every refusal and requires it to decode as the one that
  was recorded, because a diagnosis that names the wrong guard is worse than one
  that names none, and a discriminant drifting from its `match` arm is how that
  happens. 181 checks; the mutation that breaks it is number 65.
- Phase 6's VMware boot is **not** recorded as done until it is re-run against a
  machine with the controller. `014`'s close says so rather than claiming a
  boot it did not get.
