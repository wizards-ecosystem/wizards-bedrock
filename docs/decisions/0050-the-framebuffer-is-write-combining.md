# D-50 The framebuffer is write-combining

Status: Accepted, 2026-08-22. Amended by D-77.

**Decision.** The page attribute table's second entry changes from
write-through, which this kernel never asks for, to write-combining. Every
other entry keeps the value the machine supplied. The framebuffer's only
direct-map alias selects that entry. The transition writes PAT before
activating the new tables and reloads `CR3`; if activation fails, it
restores the prior PAT byte and reloads again.

**Rationale.** WC coalesces writes in a fill buffer instead of putting each
on the bus, and makes reads far worse. That is the trade a framebuffer wants
once the back buffer (D-48) holds every pixel: after attachment nothing
reads the WC mapping. One physical alias avoids the undefined cache-type
conflict a second alias would admit.

**Consequences.** The check has two halves: the PAT entry must be
write-combining, *and* the mapping must select it. A hypervisor may decline
the MSR write, so `write_combining_is_enabled` reads it back.

A wrong memory type is not merely slow: conflicting cache types for aliases
can violate the processor's rules. Safety rests on one alias, a checked
framebuffer extent and format, a read-modify PAT update, and an activation
guard that either commits or restores the transition.
