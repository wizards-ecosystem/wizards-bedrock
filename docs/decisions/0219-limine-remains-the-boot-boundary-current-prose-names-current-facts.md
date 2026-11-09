# D-219 Limine remains the boot boundary; current prose names current facts

Status: Accepted (2026-08-29), amends D-03 and closes PW-514

**Context.** The post-audit register named a small but misleading residue in
current-facing prose. Phase 5's close still presented closure-under-leaf-guard
and stale `MappedStack` defects as live after D-121 and D-125 closed them, did
not name D-189's later closure of the broad active-space door, called the
per-CPU lock count global after D-154, paired D-164's implemented timed wait
with the still-uncalled wait on multiple queues, and could be read as claiming
dynamic stacks or a first-class `MemoryObject`. The Phase 4 close still called
timed waits and both guarded stacks open; the boot and research notes called a
shipped initrd and D-162's anchored backslash code future or unresolved. The
I/O-port module still warned that i8042 chained two permanent claims even
though D-138 made that pair atomic.

One contradiction required a decision rather than editing. D-03 and the
roadmap retained an own-bootloader Phase 12 side quest, while the later
foundation audit explicitly declined it: Limine is maintained, vendored and
digest-pinned, and writing the same UEFI handoff again has no unmet kernel or
Wizzy caller.

**Decision.** Historical phase proposals, dated counts and observed results
stay historical and are labelled as such where a nearby current summary could
confuse them. Current-facing summaries name the decisions which superseded
their old debt and distinguish infrastructure which actually exists from the
triggered first-class abstractions which do not.

The project keeps Limine as its boot boundary. An own bootloader is not roadmap
work and is not a standing side quest. Reconsideration needs a concrete defect
in the required handoff which the maintained dependency cannot satisfy; the
mere opportunity to rewrite it is not a trigger.

**Rationale.** An ADR and a phase narrative serve different purposes. The ADR
must preserve what was decided and why; a current summary must not make a
reader reconstruct every later amendment before learning what exists. Adding
"historical" to genuine evidence preserves it. Repeating a closed defect as a
live one does not.

Limine already supplies the versioned handoff, has exact vendored provenance
and participates in release evidence. Replacing it would consume a complete
platform project while advancing neither kernel correctness nor the closed
Wizzy compiler gate. That violates D-183/D-184's finite-work rule unless an
actual unsupported requirement appears.

**Alternatives.** Silently removing the Phase 12 item was rejected because it
would contradict D-03 without recording why. Keeping it as an optional
side quest was rejected because roadmap entries are invitations even when
called optional. Rewriting historical phase evidence to present-day totals or
capabilities was rejected because it would invent evidence the old run did not
produce.

**Consequences.** D-03 and the current roadmap point here; the boot contract
states that module loading now carries the initrd. The Phase 4 and Phase 5
closes distinguish paid debts, absent caller-driven abstractions and genuinely
open work. The i8042 comment describes the atomic pair it now uses. A prose
host check pins these current claims while separately requiring the dated
Phase 5 check and unsafe-line totals to remain explicitly historical. The one
measured `redundant_clone` in the ACPI test is removed without adopting the
noisy nursery lint group. Targeting that lint with `deny`, all-graph Clippy,
rustdoc and the complete host suite passed; the suite contained 193 passing
tests and one intentionally ignored interruption child. Restoring the retired
own-bootloader roadmap failed only
`current_facing_foundation_prose_names_the_later_resolutions`; the restored
transformation persists as mutation 219. The full reviewed port abstraction
still contains 16 unsafe blocks and 10 unsafe declarations, and its new exact
source fingerprint records the prose-only review.
