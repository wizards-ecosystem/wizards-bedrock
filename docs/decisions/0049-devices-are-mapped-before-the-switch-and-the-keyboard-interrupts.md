# D-49 Devices are mapped before the switch, and the keyboard interrupts

Status: Accepted, 2026-08-22. Amended by D-76 and D-77.

**Decision.** Devices are discovered *before* the tables are built, rather
than reopening mapping after activation or mapping lazily on fault. ACPI is
read while the bootloader's direct map is still live. The I/O APIC's window
is mapped during `build`, and the claim on it is redeemed after the switch.
A device the kernel has not decided to map is unmappable, not merely
unmapped.

`AddressSpace::map_device` puts device windows in a reserved range
(`0xffff_ff00_..`), uncacheable and never executable, well clear of the
direct map below and the image above. It returns a `DeviceMapping`, neither
`Copy` nor `Clone`, so `DeviceMapping::claim` is a safe function returning
an `MmioRegion`. The types discharge each clause of `MmioRegion::new`'s
contract: the mapping is live (checked against the active tables), permanent
(nothing unmaps), and exclusive in both virtual and physical interval
registries (the token cannot be duplicated). RAM and firmware-backed storage
are ineligible for device mapping. `port::claim` answers "may two drivers
have this?", and ownership of a `DeviceMapping` answers "may anyone have it
at all?" (`008` §4.1).

**Firmware is the least trustworthy input.** `libs/acpi-tables` reads the
RSDP, the XSDT and the MADT from `&[u8]`, validating both RSDP checksums and
every table's, and refuses rather than panics on anything malformed,
including a zero-length entry that would otherwise loop forever.
`wost::mm::FirmwareMemory` copies only from ACPI storage, and the parser
refuses a corrupt tail before exposing a valid prefix.

**Routing does not assume `irq == gsi`.** The MADT's interrupt source
overrides are read and honoured, with the MPS INTI polarity and trigger
encodings decoded, not defaulted; assuming identity is the classic way to
program an I/O APIC that then delivers nothing.

**Rationale.** `activate` **consumes** the address space (D-45), so nothing
can map into a table the CPU is walking, yet a device is found by reading
ACPI. Ordering discovery first resolves that without weakening `activate`.

**Consequences.** D-37 is retired: the idle loop no longer polls.
`route_keyboard` falls back to the old path if firmware described no
controller, if the GSI is not this controller's, or if the controller will
not raise its line. The i8042's first port raises its line **exactly when**
something routed it, translation is off in both worlds, and the second port
is never enabled.

The keystroke check watches what the *handler* delivered, not what a poll
found. A check that has never failed is an untested claim, not evidence.

**Left for a future phase:** a device discovered *after* boot (anything
hot-plugged, and every PCI device in Phase 8) cannot be mapped under this
design. The answer is to give the kernel address space a way to grow that
keeps the same property, not to weaken `activate`. Phase 5's address-space
work should take that decision on purpose, not by accident.
