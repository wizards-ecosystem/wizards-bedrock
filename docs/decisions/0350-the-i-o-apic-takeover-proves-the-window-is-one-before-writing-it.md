# D-350 The I/O APIC takeover proves the window is one before writing it

Status: Accepted, 2026-09-29. Amends D-121; closes PW-560.

**Decision.** `libs/x86-apic` gains `entries_from_version`, a pure decoder that
returns a redirection-entry count only when the version byte is 0x11 through
0x20, the range every shipped I/O APIC reports, and the count is at most 240,
the most the field can hold. `IoApic::take` uses it and refuses a window whose
version register is anything else with `TakeError::ImplausibleVersion`, before
it masks a line or writes a redirection entry. The GSI-range and mask checks
are unchanged.

**Rationale.** The takeover used bits 23:16 of whatever the MADT's window
returned as the maximum entry index, so a window pointing at another device,
or at nothing, yielded a plausible count, and the kernel masked and later
routed redirection entries into that foreign MMIO. Firmware is trusted (the
hardware threat model), and that still holds for the window address itself,
but reading and validating one register costs nothing and turns a firmware
defect or a hostile MADT from arbitrary MMIO writes into a refused takeover;
a real I/O APIC always passes. An exact version was not required because the
parts in service report a small range, not one value. Mutations 1,531 and
1,532 retain the decoder and `take` breaks.
