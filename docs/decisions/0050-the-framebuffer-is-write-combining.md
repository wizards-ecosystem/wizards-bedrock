# D-50 The framebuffer is write-combining

Status: Accepted (2026-08-22) (amended by D-77)

**Context.** `008-research-notes-2.md` §3.4 argued that write-combining and the
console back buffer are one decision, with numbers: Linux's own MTRR
documentation puts WC at "2.5 times or more" for image writes, and field
reports on framebuffer redraw run far higher. D-48 accepted the coupling and
corrected the sequencing — the dependency runs back-buffer → WC, so the back
buffer landed in slice 4 and this waited for the device-mapping machinery
D-49 built.

**Decision.** The page attribute table's second entry is changed from
write-through, which this kernel never asks for, to write-combining; every
other entry is preserved from the value the machine supplied. The
framebuffer's only direct-map alias selects that entry. The transition writes
PAT before activating the new tables, reloads `CR3`, and restores the prior
PAT byte and reloads again if activation fails.

**Rationale.** WC coalesces writes in a fill buffer instead of putting each on
the bus, and makes reads dramatically worse — which is exactly the trade a
framebuffer wants and exactly why it could not be taken before slice 4. After
the back buffer, the console reads the device precisely once, when the shadow
is seeded.

The shadow already holds every pixel, so after attachment nothing reads the WC
mapping. Keeping one physical alias also avoids the undefined cache-type
conflict the original two-alias design admitted.

**Consequences.** One on-machine check, and it has two halves for a reason. The
PAT entry must actually be write-combining — a hypervisor may decline the MSR
write, which is why `write_combining_is_enabled` reads it back rather than
remembering what was written — *and* the mapping must select it. Either alone
leaves writes crossing the bus one at a time while the log claims otherwise.

The old claim that a wrong memory type could only be slow was false: conflicting
cache types for aliases can violate the processor's rules. Safety now rests on
one alias, a checked framebuffer extent and format, a read-modify PAT update,
and an activation guard that either commits or restores the transition.

Phase 3's slice 6 closes with this, and with it the phase: every debt
`007-memory.md` collected is paid except D-46's KASLR, which is deferred to
Phase 12 with its cost named.
