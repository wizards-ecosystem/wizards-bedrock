# D-50 The framebuffer is write-combining

Status: Accepted, 2026-08-22. Amended by D-77.

**Context.** `008-research-notes-2.md` §3.4 argued, with numbers, that
write-combining and the console back buffer are one decision. Linux's MTRR
documentation puts WC at "2.5 times or more" for image writes; field reports
on framebuffer redraw run far higher. D-48 accepted the coupling and fixed
the sequencing (back-buffer → WC): the back buffer landed in slice 4, and
this waited for D-49's device-mapping machinery.

**Decision.** The page attribute table's second entry changes from
write-through, which this kernel never asks for, to write-combining. Every
other entry keeps the value the machine supplied. The framebuffer's only
direct-map alias selects that entry. The transition writes PAT before
activating the new tables and reloads `CR3`; if activation fails, it restores
the prior PAT byte and reloads again.

**Rationale.** WC coalesces writes in a fill buffer instead of putting each
on the bus, and makes reads far worse. That is the trade a framebuffer
wants, and why it could not be taken before slice 4. After the back buffer,
the console reads the device once, when the shadow is seeded.

The shadow holds every pixel, so after attachment nothing reads the WC
mapping. One physical alias also avoids the undefined cache-type conflict
the original two-alias design admitted.

**Consequences.** One on-machine check, with two halves: the PAT entry
must be write-combining, *and* the mapping must select it. A hypervisor may
decline the MSR write, so `write_combining_is_enabled` reads it back. Either
half alone leaves writes crossing the bus one at a time while the log claims
otherwise.

The old claim that a wrong memory type could only be slow was false:
conflicting cache types for aliases can violate the processor's rules.
Safety now rests on one alias, a checked framebuffer extent and format, a
read-modify PAT update, and an activation guard that either commits or
restores the transition.

This closes Phase 3's slice 6, and with it the phase: every debt
`007-memory.md` collected is paid except D-46's KASLR, deferred to Phase 12
with its cost named.
