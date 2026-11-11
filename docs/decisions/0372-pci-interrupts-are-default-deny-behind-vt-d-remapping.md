# D-372 PCI interrupts are default-deny behind VT-d remapping

Status: Accepted, 2026-10-09. Amends D-358 and D-371; closes PW-710.

**Decision.** VT-d lookup now requires the DMAR interrupt-remapping flag and
refuses x2APIC opt-out. This first implementation supports exactly one live
DRHD: the selected unit's bounded requester inventory must contain every
present function in the sealed segment-zero topology. A second, absent or
incompletely described unit fails closed. The unit must advertise IR, queued
invalidation and extended interrupt mode as well as the existing two-direction
DMA drain.

Takeover inherits none of firmware's TE, QIE, IRE or CFI state. With every
requester exclusively claimed and BME-fenced, it first masks and reads back
FECTL while preserving its reserved bits: VT-d fault-event MSI does not traverse
interrupt remapping, so fault records remain polled and that source stays
masked for the owner's lifetime. It then quiesces all four global facilities,
drains earlier DMA, publishes and invalidates the DMA root, and publishes a
kernel-owned two-entry interrupt-remapping table. IRTA has EIME set and both
128-bit entries remain not present, so every requester interrupt is denied. A
private one-page queued-invalidation ring issues a global, not index-selective,
interrupt-entry-cache invalidation followed by a status-write wait descriptor.
Software enables QI before accepting IQA readback, publishes IQT, observes the
completion word and disables QI. It then enables IRE with CFI clear, reads
status back, and only then enables DMA translation.

`Vtd::interrupt_isolation` mints a non-`Clone`, non-`Copy` proof only when IRES
is set, CFIS and QIES are clear, FECTL.IM still reads masked, IRTA is the
owner's exact EIME two-entry table, and all four qwords in that table read back
zero. PCI bus-master admission consumes that proof beside the exact function's
`DmaDomain` and rejects a function mismatch. After a failed storage candidate,
retirement drains and disables TE but never disables IRE, unmaps the register
window or drops the empty IR table. The next candidate rebuilds and republishes
only the DMA tables through that same continuously live interrupt owner. If
storage selection leaves only the parked owner, it still guards `sti`; if no
candidate established one, boot creates an interrupt-only owner. Any unsealed
topology, second or foreign-segment DRHD, uncovered requester, lost FECTL mask
or absent owner refuses `sti`.

The final admission also audits all eight local-APIC IRR banks before the first
`sti` and refuses any pending vector. This is defense in depth, not proof that
the platform never accepted an in-flight message: firmware/SMM, devices and
remapping-hardware event sources are trusted to remain quiescent for
compatibility-format interrupt traffic until the first successful kernel IRE
establishment. Once IRE is established, PCI devices are hostile; the kernel
never reopens that pre-establishment assumption during candidate fallback.
Firmware/SMM is separately trusted not to reprogram VT-d or APIC state after
the kernel's exact readbacks.

The empty table admits no I/O APIC source either. Every I/O APIC line therefore
stays masked on a PCI boot, its routing owner is returned after verified masked
rollback, and the keyboard uses its bounded poller. That poller sleeps only
when calibrated timer interrupts are actually enabled; otherwise it yields, so
the no-DMAR refusal path cannot block forever waiting for a timer that `sti`
correctly withheld. The fixed QEMU profile enables `intremap=on,eim=on` and
keeps caching mode on.

**Rationale.** Codex Security occurrence `occ_66ce12f335b236e1daa2b291`
identified a privilege-boundary gap separate from DMA reach: a hostile PCI
function with BME authority could program MSI or MSI-X data for an arbitrary
vector, while the IDT's timer, keyboard, syscall and exception handlers assume
their vector came from a kernel-owned source and an unknown vector is
terminal. The previous takeover preserved firmware IRE and compatibility-format
state and QEMU qualification disabled interrupt remapping, so neither BME nor
`sti` carried evidence that a device could not inject a privileged vector. The
kernel has no IRTE allocator, no source-identifier-bound interrupt route and no
driver that needs interrupts, since NVMe is polled, so a deny-all table is the
smallest complete policy; it also avoids claiming that clearing a device's BME
bit is a security boundary against hostile interrupt generation.

**Alternatives.** Trusting firmware's live IRE/CFI and IRTA preserves an
unverified table and compatibility bypass. Enabling IRE without a global IEC
invalidation permits a stale present entry. Treating BME clear as interrupt
containment assumes hostile device cooperation. Allowing multiple DRHDs before
the kernel can retain and prove every corresponding table creates an uncovered
requester. Programming one permissive keyboard or storage IRTE would require a
source-identifier-bound allocator and lifecycle the current drivers do not
need. Leaving I/O APIC routing live against an empty table creates a silent
input failure; polling makes the deny-all policy explicit and testable.
Treating an IRR snapshot as a drain for a message still in the PCI/APIC fabric
was rejected because the architecture supplies no such completion boundary;
the pre-IRE quiescence assumption above is explicit instead.

**Consequences.** Release and debug machine boots show the live deny-all
owner, exact masked I/O APIC rollback, zero legacy keyboard interrupts, polling
progress and the unchanged storage/filesystem result. The shipped boot
requires the masked/polling records and proves injected input does not
increment IRQ1. The no-DMAR boot refuses storage and interrupt admission yet
settles. Mutations 1,703 through 1,742 cover the platform, capability,
sole-unit topology, takeover, table, invalidation, FECTL, continuous-owner
lifecycle, requester, proof, admission, IRR/`sti`, routing, poller and
QEMU-profile predicates.
