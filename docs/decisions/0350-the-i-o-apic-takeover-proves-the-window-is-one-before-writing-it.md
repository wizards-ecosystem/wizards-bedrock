# D-350 The I/O APIC takeover proves the window is one before writing it

Status: Accepted, 2026-09-29. Amends D-121; closes PW-560.

**Context.** Firmware's MADT supplies each I/O APIC's register-window address,
and the takeover read the version register there to learn how many redirection
entries the controller has. It used bits 23:16 of whatever it read as the
maximum entry index and never checked that the value came from an I/O APIC. A
window firmware pointed at another device, or at nothing, therefore yielded a
plausible-looking entry count, and the kernel masked and later routed
redirection entries into that foreign MMIO. Firmware is trusted (the hardware
threat model), but reading one register and validating it costs nothing and
turns a firmware defect or a hostile MADT from arbitrary MMIO writes into a
refused takeover.

**Decision.** `libs/x86-apic` gains `entries_from_version`, a pure decoder that
returns a redirection-entry count only when the version byte is 0x11 through
0x20, the range every shipped I/O APIC reports, and the count is at most 240,
the most the field can hold. `IoApic::take` uses it and refuses a window whose
version register is anything else with `TakeError::ImplausibleVersion`, before
it masks a line or writes a redirection entry. The GSI-range and mask checks
are unchanged.

**Evidence.** On 2026-09-29 an `x86-apic` host test and its doctest checked the
decoder on real versions with 1 to 240 entries and on implausible bytes and
over-counts. A machine check refuses a window reporting version zero, a
console-byte value and an over-count, and still accepts the plausible version
the other I/O APIC checks use. A release self-test passed 265/265. Mutation
1,531 relaxes the decoder to admit any version byte and fails the host test;
mutation 1,532 restores the old inline count in `take` and fails the machine
check.

**Alternatives.** Trusting the MADT window outright was the prior behaviour and
is what the threat model still assumes for the address itself; the version
check is defence in depth that a real I/O APIC always passes. Requiring an
exact version was rejected because the parts in service report a small range,
not one value.
