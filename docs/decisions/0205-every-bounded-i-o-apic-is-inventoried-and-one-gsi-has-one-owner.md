# D-205 Every bounded I/O APIC is inventoried and one GSI has one owner

Status: Accepted, 2026-08-28. Amends D-49 and D-86; closes PW-304.

**Context.** The MADT walker logged every I/O APIC structure and retained only
the first. Paging consequently mapped and masked only that controller, and
keyboard routing fell back to polling whenever an interrupt-source override
named a GSI owned by a later controller. ACPI 6.5 section 5.2.12.3 says that
each structure supplies only a GSI base; software must read that controller's
Max Redirection Entry field to learn the other end of its range. Ordering the
structures or comparing their bases therefore cannot identify an owner.

**Decision.** `acpi-tables` builds one checked MADT interrupt inventory. The
kernel accepts at most sixteen I/O APIC descriptions, preserves every one up
to that bound, and refuses the complete inventory rather than silently
truncating a seventeenth. Sixteen is an explicit boot-resource policy: it
temporarily spends at most half of the address space's thirty-two device
mapping records and sixteen pages while leaving headroom for existing device
callers. A system above the bound keeps the safe polled-keyboard path.

After the kernel owns its active tables and before it accepts interrupts, it
maps every described controller with releasable ownership, reads the hardware
entry count and masks every reported line. Pure `x86-apic` range logic rejects
zero length, a range whose last `u32` GSI is unrepresentable, and any overlap
between controllers. It then selects the unique range containing the resolved
keyboard GSI. Every non-owner mapping is returned; the chosen controller alone
converts its mapping to permanent access and survives for routing and readback.
Any map, register, range, ownership or cleanup refusal leaves the keyboard
polled and never enables its interrupt source.

**Rationale.** The hardware count is the authority ACPI names, so reading all
bounded candidates is the only way to distinguish a gap, a later owner and an
ambiguous overlap. Masking precedes the CPU's interrupt-enable transition just
as it did for the former single controller. Releasable probing prevents a
larger correct firmware description from permanently consuming the device
window merely because most controllers do not own the one line used today.

**Alternatives.** Keeping the first controller was the defect. Choosing the
largest base not above the GSI was rejected because the preceding controller's
hardware range may end early or overlap it. Treating the next base as the
previous range's end was rejected for the same reason and cannot bound the
last controller. Mapping all descriptions permanently was correct but spent a
scarce registry slot per unused controller. Dynamic allocation without a count
limit was rejected because a checksummed but hostile one-megabyte MADT could
turn boot into unbounded device mapping work.

**Consequences.** The host MADT corpus retained two controllers in firmware
order and refused a third against a two-controller bound. Pure APIC tests
rejected empty and overflowing ranges, rejected an overlap anywhere in the
inventory, and selected a later controller across a real gap. Each of those
five properties was broken with its exact production mutation and reddened
only its named host test; their persistent mutations bring the register to 150
entries.

Release QEMU then exercised the complete hardware path: it mapped the reported
controller releasably, read and masked its hardware-sized range, returned the
probe mapping, remapped only the selected owner, routed GSI 1 to vector 33 and
passed the existing route, readback, refusal and delivered-IRQ checks. The
complete image passed 211/211. That boot also exposed an older signal-wakeup
witness racing the D-203 service worker: its global blocked-thread count could
advance before the event waiter blocked. Waiting for that exact spawned slot
removed the witness race; the repaired image passed 211/211 without changing
the signal protocol or adding a claim. No unsafe operation was added.
