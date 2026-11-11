# D-142 Hardware constants checked against the manual, not against themselves

Status: Accepted, 2026-08-25.

**Decision.** **A constant that describes hardware is checked against the
document that fixes it, never against another constant from the same file.**
Three sites where a wrong value reaches the hardware directly are checked
against their manuals, as literals:

- ACPI 6.5 table 5-22's MADT structure lengths, every kind, both directions:
  one byte short is refused, exactly enough is walked without panicking. A
  lowered minimum let `next` index past an entry, a panic on firmware input.
- SDM Vol. 3A §11.4.4's `IA32_APIC_BASE` bit numbers, pairwise distinct, and
  none overlapping the address field. `TYPICAL = 0xFEE0_0900` sets both
  `BOOTSTRAP` (bit 8) and `ENABLE` (bit 11), so every test using it was
  satisfied by the two constants exchanged.
- SDM Vol. 3A figure 8-4's TSS descriptor flag byte: 52 available, 53 reserved,
  54 default/big and 55 granularity all clear, with present and the type nibble
  asserted set so an empty descriptor does not satisfy it. A granularity bit
  scales a 104-byte limit so `iomap_base` lands inside the segment and the CPU
  reads an I/O permission bitmap from whatever follows.

**Rationale.** A value that appears on both sides of an assertion is a
definition; a value beside a literal from the SDM is a transcription, and a
wrong transcription fails the first time it is compared. The APIC case argues
for *pairwise distinct* test values: `TYPICAL` is a real register value with
both bits set, which is exactly why it could not distinguish them, so a
realistic input was worse here than a contrived one.

**Consequences.** No production code changed. Every constant was correct; none
of them could be shown to be. The hardware-constant corollary belongs beside
`005`'s "write the check from the claim, not from the implementation" (D-124).

**Left owed.** The audit's Tier 2 and 3 lists hold roughly twenty more of this
family: `ps2-keys`' scancode table anchors 21 of ~89 codes, `exception::name`
never reads `PAGE_FAULT` back, `ring-proto::VERSION` appears on both sides of
every comparison. Each is smaller than these three and none reaches hardware
the same way; they are worth doing and are not done here.
