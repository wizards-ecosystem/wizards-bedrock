# D-205 Every bounded I/O APIC is inventoried and one GSI has one owner

Status: Accepted, 2026-08-28. Amends D-49 and D-86; closes PW-304.

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

**Rationale.** The walker had kept only the first controller, so an override
naming a later controller's GSI fell back to polling. ACPI 6.5 section 5.2.12.3
gives each structure only a GSI base; software must read that controller's Max
Redirection Entry field for the other end. The hardware count is the authority
ACPI names, so reading all bounded candidates is the only way to distinguish a
gap, a later owner and an ambiguous overlap. Masking precedes the CPU's
interrupt-enable transition just as it did for the former single controller.
Releasable probing prevents a larger correct firmware description from
permanently consuming the device window merely because most controllers do not
own the one line used today.

**Alternatives.** Choosing the largest base not above the GSI was rejected
because the preceding controller's hardware range may end early or overlap it.
Treating the next base as the previous range's end was rejected for the same
reason and cannot bound the last controller. Mapping all descriptions
permanently was correct but spent a scarce registry slot per unused controller.
Dynamic allocation without a count limit was rejected because a checksummed but
hostile one-megabyte MADT could turn boot into unbounded device mapping work.

**Consequences.** The host MADT corpus retains two controllers in firmware order
and refuses a third against a two-controller bound. Pure APIC tests reject
empty and overflowing ranges, reject an overlap anywhere in the inventory, and
select a later controller across a real gap; each property has a persistent
mutation that reddens only its named host test. Release QEMU exercises the
complete hardware path: it maps the reported controller releasably, reads and
masks its hardware-sized range, returns the probe mapping, remaps only the
selected owner, routes GSI 1 to vector 33 and passes the existing route,
readback, refusal and delivered-IRQ checks. The signal-wakeup witness waits for
its exact spawned slot, because its global blocked-thread count could advance
before the event waiter blocked while racing the D-203 service worker. No
unsafe operation was added.
