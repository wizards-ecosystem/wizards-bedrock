# D-142 Hardware constants checked against the manual, not against themselves

Status: Accepted, 2026-08-25.

**Context.** D-136 pinned five `libs/` properties that a mutation could defeat.
The audit's Tier 1 list had three more of the same family, and all three sit
where a wrong value reaches the hardware directly.

**A MADT entry could be read past its end.** `MadtEntries` validates each
entry's length against a per-kind minimum before walking it — 8 for a processor
local APIC, 12 for an I/O APIC, 10 for an interrupt source override. Only kind
1's minimum was pinned by a test. Lower kind 2's to 8 and the suite stays green,
and `next` then reads `entry[9]` for an eight-byte entry: an **index-out-of-bounds
panic on firmware input**, which this module's own doc says it will not do.

**The APIC's base flags could be swapped.** `TYPICAL = 0xFEE0_0900` sets both
`BOOTSTRAP` (bit 8) and `ENABLE` (bit 11), so every test that used it was
satisfied by the two constants exchanged — fifty-seven of them. `Base::enabled()`
would then set the **read-only** bootstrap bit and never enable the APIC.

**The TSS descriptor could be given a scaled limit.** Bits 52–55 of the
task-state descriptor were asserted by nothing. `TASK_STATE_TYPE | 1 << 55` —
granularity — survives, and granularity multiplies the limit by 4 KiB: a
104-byte segment becomes `0x67FFF`, so the TSS's `iomap_base` of 104 lands
**inside** the segment and the CPU fetches an I/O permission bitmap from
whatever follows the TSS in memory. Ring 3's port access would be granted by
whatever bytes happened to be there.

**Decision.** Each is now checked against its manual, as a literal:

- ACPI 6.5 table 5-22's structure lengths, every kind, both directions — one
  byte short is refused, exactly enough is walked without panicking.
- SDM Vol. 3A §11.4.4's `IA32_APIC_BASE` bit numbers, pairwise distinct, and
  none overlapping the address field.
- SDM Vol. 3A figure 8-4's flag byte: 52 available, 53 reserved, 54 default/big
  and 55 granularity all clear, with present and the type nibble asserted set
  so an empty descriptor does not satisfy it.

**Rationale.** This is D-136's rule at a third site and the pattern is now
established enough to state as a convention rather than a finding: **a constant
that describes hardware is checked against the document that fixes it, never
against another constant from the same file.** A value that appears on both
sides of an assertion is a definition; a value that appears beside a literal
from the SDM is a transcription, and a wrong transcription fails the first time
it is compared.

The APIC case is the clearest argument for *pairwise distinct* test values.
`TYPICAL` is a real register value with both bits set, which is exactly why it
could not distinguish them — a test using a realistic input was worse here than
one using a contrived one.

**Consequences.**

- Three `libs/` crates gain a check each, all broken once and watched go red
  with the exact mutation the audit named.
- No production code changed. Every constant was correct; none of them could
  be shown to be.
- `005`'s testing section already carries "write the check from the claim, not
  from the implementation" (D-124). The hardware-constant corollary is now
  demonstrated three times and belongs beside it.

**Left owed.** The audit's Tier 2 and 3 lists hold roughly twenty more of this
family — `ps2-keys`' scancode table anchors 21 of ~89 codes, `exception::name`
never reads `PAGE_FAULT` back, `ring-proto::VERSION` appears on both sides of
every comparison. Each is smaller than these three and none reaches hardware
the same way; they are worth doing and are not done here.
