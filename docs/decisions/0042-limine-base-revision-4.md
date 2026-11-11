# D-42 Limine base revision 4

Status: Accepted, 2026-08-22.

**Decision.** Base revision 4, requested explicitly through `with_revision`
instead of inherited from `new()` (which in `limine` 0.5.0 is revision 3).

**Rationale.** Revision 4 adds Reserved (Mapped), ACPI reclaimable and ACPI
NVS to the HHDM, adds the `RESERVED_MAPPED` memory-map type, and guarantees
ACPI tables are mapped, which is how slice 5 finds the I/O APIC through the
MADT. Revisions 5 and 6 were declined on the no-speculative-change rule: 5's
guarantees cover state this kernel establishes itself, and 6 changes nothing
on x86-64. Taking the decision explicitly keeps either a one-argument
change.

**Consequences.** The crate pin does not block the revision: `=0.5.0` stands
because 0.6.x needs nightly, but it exposes
`BaseRevision::with_revision(u64)` as a public `const fn`. The crate version
and the protocol revision are independent choices.

`is_supported()` is checked and asserted in `kernel_main`, so a bootloader
that will not honour it stops the boot with a message instead of misbehaving
quietly.

`limine_glue` maps memory-map entry types through an exhaustive match with
an explicit unknown arm and treats anything unknown as reserved: the
`#[non_exhaustive]` discipline `005` mandates, applied to a foreign enum, so
a future revision bump is a compile-time question instead of a silent
misclassification.

Base revision 4 does **not** change the plan to map device MMIO through
`MmioRegion` with an explicit memory type. It removes the wall, but the
reason for explicit device mappings remains, and D-43's direct-map contract
still excludes device holes on purpose.
