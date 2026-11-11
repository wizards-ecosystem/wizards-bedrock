# D-198 Hardware-free namespaces do not depend on device discovery

Status: Accepted, 2026-08-27. Amends D-90 and D-172; closes PW-207.

**Decision.** The boot continuation owns three separate steps while interrupts
are still disabled: attempt PCI/NVMe discovery, initialize and exercise the
hardware-free filesystems regardless of that attempt's outcome, then bring up
the interrupt controller and clock. PCI discovery keeps its local early returns
and no longer calls filesystem code. A returned bus outcome is deliberately not
used to authorize `tmpfs` or `devfs`; their constructors and VFS survey are an
unconditional sibling step.

The shipped-only degraded image enables a narrow `refuse-pci` fault-injection
feature. Before discovery it permanently claims one configuration-address port
through `wost`'s safe port capability, making the production all-or-none
CF8/CFC claim refuse without issuing a configuration instruction. The same boot
still uses the CPU without x2APIC required by D-90. Its harness requires the PCI
refusal, tmpfs enumeration, both devfs device resolutions, the no-x2APIC
diagnosis and the final steady-state line.

**Rationale.** `tmpfs` and `devfs` need neither a disk nor PCI, but were
surveyed at the end of PCI discovery, itself called from interrupt bring-up, so
a machine that refused PCI or interrupts skipped the namespace foundation while
the log claimed both backends ran. Moving only the filesystem call past the two
PCI returns would leave it dependent on successful interrupt setup, another
unrelated hardware precondition. Enabling interrupts before the existing polled
NVMe survey would change the lock/preemption environment of the storage path
for no benefit. Keeping discovery and the namespace survey adjacent in
`boot_continue` retains the established interrupt-disabled device order while
making their lack of dependency visible in the call graph.

**Alternatives.** A self-test helper returning counters was rejected because it
would prove a model of boot ordering rather than boot the refusal path. Removing
PCI devices from QEMU was rejected because q35 and its UEFI firmware still
provide configuration space, so it does not exercise either early return.
Adding a fifth boot was rejected because the existing degraded boot can carry
both independent refusals without weakening D-90. Treating `tmpfs` and `devfs`
as optional after PCI failure was rejected because their concrete future Wizzy
callers are precisely programs which must still start without storage hardware.

**Consequences.** The degraded feature is built only for the gate image and is
included in the kernel configuration passes so it cannot become an unlinted
crate graph. The harness's expected-line constants are checked against the
kernel source. Restoring the historical PCI-conditioned call removes only the
filesystem lines and fails the task exactly on missing `tmpfs:`. No self-test
count or unsafe operation was added.
