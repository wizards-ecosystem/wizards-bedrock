# D-49 Devices are mapped before the switch, and the keyboard interrupts

Status: Accepted, 2026-08-22. Amended by D-76 and D-77.

**Context.** `007-memory.md`'s slice 5 owes three things: the I/O APIC up,
IRQ 1 routed, and D-37's polled keyboard drain retired. D-33 called the
unmappable device hole "the wall itself".

The design surfaced a consequence of D-45 that neither `007` nor
`008-research-notes-2.md` anticipated. `activate` **consumes** the address
space, so nothing can map into a table the CPU is already walking; that
makes `AddressSpace::map` a safe function. But a device's register window is
a mapping, and the device is found by reading ACPI, after boot. Taken
naively, the two requirements contradict.

**Options.** (a) Reopen mapping after activation behind a second guarded
API; (b) reserve a device window and map lazily on fault; (c) discover
devices *before* the tables are built, and map their windows during
construction.

**Decision.** (c). ACPI is read while the bootloader's direct map is still
live, so the tables are reachable. The I/O APIC's window is mapped during
`build`, and the claim on it is redeemed after the switch. The design forces
this ordering, and it is stronger for it: a device the kernel has not decided
to map is unmappable, not merely unmapped.

`AddressSpace::map_device` puts device windows in a reserved range
(`0xffff_ff00_..`), uncacheable and never executable, well clear of the direct
map below and the image above. It returns a `DeviceMapping`, neither `Copy` nor
`Clone`, so `DeviceMapping::claim` is a safe function returning an `MmioRegion`.
The types discharge each clause of `MmioRegion::new`'s contract: the mapping is
live (checked against the active tables), permanent (nothing unmaps), and
exclusive in both virtual and physical interval registries (the token cannot be
duplicated). RAM and firmware-backed storage are ineligible for device mapping.
This is the sensitivity discipline `008` §4.1 took from Asterinas, in the form
this kernel can enforce: `port::claim` answers "may two drivers have this?", and
ownership of a `DeviceMapping` answers "may anyone have it at all?"

**The parsing is host-tested, and that mattered.** `libs/acpi-tables` reads
the RSDP, the XSDT and the MADT from `&[u8]`, validating both RSDP checksums
and every table's, and refuses rather than panics on anything malformed.
Firmware is the kernel's least trustworthy input. The host suite covers
truncated tables, corrupt checksums, an entry claiming zero length
(which would otherwise loop forever), an entry running past the table, and a
truncated entry of a known kind. `wost::mm::FirmwareMemory` copies only from
ACPI storage, and the parser refuses a corrupt tail before exposing a valid
prefix.

**Routing does not assume `irq == gsi`.** The MADT's interrupt source
overrides are read and honoured, with the MPS INTI polarity and trigger
encodings decoded, not defaulted. On QEMU the keyboard is GSI 1 either way.
On hardware that remaps it, assuming identity is the classic way to program
an I/O APIC that then delivers nothing.

**Consequences.** D-37 is retired: the idle loop no longer polls.
`route_keyboard` falls back to the old path if firmware described no
controller, if the GSI is not this controller's, or if the controller will
not raise its line.

Two on-machine checks; the *third* taught the most. The existing "an injected
keystroke arrives" check kept passing, because `await_key` polled the
controller itself: it proved a keystroke arrived, not how. Rewritten to watch
what the *handler* delivered, it turned red at once. The line was routed and
the vector right, but nothing was delivered, because Phase 2 configured the
i8042 to raise no interrupts (D-37, correct at the time). One
`enable_interrupt` later it went green.

It is the phase's third check whose *shape* hid a defect: D-43's zeroing
check short-circuited on an identity comparison, D-47's reuse check depended
on allocator call ordering, and this one measured the wrong path. All three
looked reasonable. All three were found by D-28's break-it-once rule or by
tightening a check that had never failed. A check that has never failed is
an untested claim, not evidence.

Phase 2's "the controller raises no interrupts and does not translate" is
replaced by one that ties the configuration to the routing. The first port
raises its line **exactly when** something routed it, translation is off in
both worlds, and the second port is never enabled.

**Left for a future phase:** a device discovered *after* boot (anything
hot-plugged, and every PCI device in Phase 8) cannot be mapped under this
design. The answer is to give the kernel address space a way to grow that
keeps the same property, not to weaken `activate`. Phase 5's address-space
work should take that decision on purpose, not by accident.
