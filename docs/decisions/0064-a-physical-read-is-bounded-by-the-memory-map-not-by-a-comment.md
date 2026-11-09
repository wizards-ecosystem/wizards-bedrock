# D-64 A physical read is bounded by the memory map, not by a comment

Status: Accepted, 2026-08-23. Amended by D-76 and D-77.

**Context.** The phase 0–4 audit found two `// SAFETY:` comments asserting
checks that no code performed. `mm::read_physical` said `direct_pointer` had
confirmed the direct map covers the range; `direct_pointer` only checks that
the arithmetic does not overflow. `DeviceMapping::claim` said its region was
four-byte aligned; nothing established that, and the window deliberately keeps
the device's own offset from a page boundary.

Neither was reachable. `read_physical` runs before `activate`, where the
bootloader's map covers everything; every device this kernel maps happens to
be page-aligned. Both were one firmware quirk from being real, and
`read_physical`'s length comes from a firmware table's own header.

**Decision.** There is no general safe physical-read operation. A
`FirmwareMemory` capability copies with volatile reads only from ACPI
reclaimable or ACPI NVS regions, and refuses overflow, gaps, every other memory
kind, and buffers outside a one-megabyte table budget. ACPI parsers also enforce
their declared length, containing buffer, checksum, fixed header and complete
entry stream before exposing any entry. A device whose registers are not
four-byte aligned is refused at map time, with an error that says so.

**Rationale.** The previous audit's standing lesson was that an abstraction
promising a bound must be given something that can check it. A safety comment
is not that thing, and a memory-map description is not authority to read live
kernel data. The capability names both the permitted provenance and the copy
semantics; the parser supplies the second, hostile-input bound.

**Consequences.**

- The machine check proves both acceptance of the real firmware bytes and
  refusal of non-ACPI storage. Host checks separately break every ACPI length,
  checksum and entry-stream bound.
- `FirmwareMemory::contains` walks adjacent regions but permits only the two
  ACPI kinds throughout the span. No safe caller can borrow an alias to the
  underlying physical bytes.
- The `Frame` methods kept their claim but got the true argument for it: a
  frame is allocatable memory, and every allocatable region is direct-mapped.
