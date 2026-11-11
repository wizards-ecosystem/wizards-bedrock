# D-64 A physical read is bounded by the memory map, not by a comment

Status: Accepted, 2026-08-23. Amended by D-76 and D-77.

**Decision.** There is no general safe physical-read operation. A
`FirmwareMemory` capability copies with volatile reads only from ACPI
reclaimable or ACPI NVS regions. It refuses overflow, gaps, every other
memory kind, and buffers past a one-megabyte table budget. ACPI parsers
check declared length, containing buffer, checksum, fixed header and the
complete entry stream before exposing any entry. A device with registers not
four-byte aligned is refused at map time, with an error saying so.

**Rationale.** Two `// SAFETY:` comments asserted checks no code performed,
one firmware quirk from being real. An abstraction promising a bound must be
given something that can check it. A safety comment cannot, and a memory-map
description is not authority to read live kernel data. The capability names
the permitted provenance and the copy semantics; the parser supplies the
second, hostile-input bound.

**Consequences.**

- `FirmwareMemory::contains` walks adjacent regions but permits only the two
  ACPI kinds across the span. No safe caller can alias the physical bytes.
- The `Frame` methods kept their claim with its true argument: a frame is
  allocatable memory, and every allocatable region is direct-mapped.
