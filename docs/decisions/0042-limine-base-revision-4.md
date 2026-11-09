# D-42 Limine base revision 4

Status: Accepted, 2026-08-22.

**Context.** The kernel asked for base revision 3, and it asked by accident:
`wost::entry!` emits `BaseRevision::new()`, and in the pinned `limine` 0.5.0
that constructor is `with_revision(3)`. No decision was ever taken.

That default has been shaping the design. `BootInfo`'s doc comment records the
direct map's absence as a consequence of it — "under base revision 3 it covers
only the regions the memory map lists — not the device holes" — and D-33
deleted the field for that reason. `007-memory.md` inherits the same sentence
as "device holes unmappable — the wall itself".

The protocol has since reached base revision 6, with 0–5 marked deprecated.
Reading the revision history rather than the deprecation notice is what
matters here:

| Revision | Change relevant to this kernel |
|---|---|
| 3 | HHDM covers Usable, Bootloader-reclaimable, Executable-and-modules, Framebuffer |
| **4** | HHDM **adds Reserved (Mapped), ACPI reclaimable, ACPI NVS**; adds the `RESERVED_MAPPED` memory-map type; **guarantees ACPI tables are mapped** |
| 5 | SMBIOS mapped; stricter x86-64 control-register handoff; more I/O APIC entries masked; IOMMUs disabled; local APIC left in a defined state on every processor |
| 6 | Drops identity mapping; register-state guarantees for aarch64, RISC-V, loongarch64 — nothing x86-64 |

**The obvious blocker is not one.** `001-boot.md` lists "Limine crate 0.6.x
when its nightly requirement lands on stable" as an open item, which reads like
the revision is gated behind it. It is not: `limine` 0.6.5 still carries
`#![feature(ptr_metadata)]` and fails to build on stable 1.97.1 — confirmed by
building it — so the `=0.5.0` pin stands, but the base revision is a *protocol*
parameter and 0.5.0 exposes `BaseRevision::with_revision(u64)` as a public
`const fn`. The crate version and the protocol revision are independent
choices, and only the first is blocked.

**Options.** (a) Stay on 3 and keep the wall; (b) take 4; (c) take 5 for the
cleaner handoff; (d) take 6.

**Decision.** (b), base revision 4, requested explicitly through
`with_revision` rather than inherited from `new()`.

**Rationale.** 4 is the revision that buys something this phase needs twice
over: it is what lets the HHDM reach a device hole at all, and it guarantees
the ACPI tables are mapped, which is how slice 5 finds the I/O APIC through
the MADT. 5 and 6 were declined for now on the no-speculative-change rule —
5's guarantees are about state this kernel already establishes for itself
(it masks the legacy controllers and programs the local APIC in
`start_interrupts`), and 6 is entirely non-x86-64. Both remain available as a
one-argument change, which is the point of taking the decision explicitly.

**Consequences.** `entry!` asks for 4; `is_supported()` is already checked and
asserted in `kernel_main`, so a bootloader that will not honour it stops the
boot with a message rather than misbehaving quietly. The vendored Limine v10
does honour it — verified by boot, which is the only way this could be
verified.

The memory map gains a type the pinned crate does not know
(`RESERVED_MAPPED`), so `limine_glue` maps entry types through an exhaustive
match with an explicit unknown arm and treats anything unrecognised as
reserved. That is the `#[non_exhaustive]` discipline `005` mandates for our own
error enums, applied to a foreign one — and it is what makes a future
revision bump a compile-time question rather than a silent misclassification.

Base revision 4 does **not** change the plan to map device MMIO through
`MmioRegion` with an explicit memory type. It removes the wall; it does not
remove the reason for the door, and D-43's direct-map contract still excludes
device holes deliberately.
