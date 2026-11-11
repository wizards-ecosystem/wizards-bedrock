# D-314 The userland is designed before its compiler, as a specification

Status: Accepted, 2026-09-12. Refines D-19, D-183 and D-184.

**Decision.** `018-research-notes-5.md` records the research pass,
`019-userland.md` designs the userland and `020-style-guide.md` fixes its
visual and interaction language. All three are specification: they add no
target-side code, no consumer scaffolding and no register rows, they state
the gate's standing in one paragraph each, and they assume a complete Wizzy so
that the design is made once rather than shaped by whichever compiler feature
lands first. Their handoff is two ledgers in `019` section 15: the kernel
prerequisites the design forces, each with its Wizzy caller, and the language
milestones it stands on, mapped to the language repository's own rows. Product
names are plain instrument words (the bench, the rail, the strip, the palette,
places, the ledger, the works) and apps are named by function; the wizard
identity lives in the ecosystem brand, the boot messages and the copy voice,
which refines D-19's theme sentence without changing its rule that code stays
conventional.

**Rationale.** D-183 closed the compiler gate and D-184 made `016` the only
pre-gate backlog, but neither said when the userland is designed, and the
owner asked on 2026-09-12 for it now, for one developer at a keyboard-first
desktop. A design document is the "specification" work D-183 admits, and
writing it while the gate is closed is what `015` calls harmless. Every
desktop in `018` that designed each surface when its phase arrived grew two of
everything: two settings applications, two menus, three theming systems, two
workspace concepts. Designing the whole userland at once is the only way to
have one of each. Assuming a complete language is not a claim that it exists:
the assumptions are enumerated so the language repository can price them.

**Alternatives.** Designing each piece as its phase arrives was rejected for
the reason above. Waiting for the gate to open was rejected because the
compiler repository is asking now what the OS needs, and an answer derived
from a design is worth more than one derived from the kernel's current shape.
Writing scaffolding behind the gate was rejected by D-183 and stays rejected.

**Consequences.** The repository check that keeps D-183's wording honest lists
`019` and `020` among the current-facing plans, so both must name the
governing decisions and neither may restore the retired language-selection
wording. The three documents are numbered in creation order after `017`.
Nothing in `016` changes: the kernel prerequisites in `019` are post-gate items
with named callers, exactly as `016`'s "Explicitly after the compiler gate"
classifies such work, and they enter the register only when the gate opens and
a caller exists.
