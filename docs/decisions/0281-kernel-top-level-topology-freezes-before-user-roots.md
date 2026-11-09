# D-281 Kernel top-level topology freezes before user roots

Status: Accepted, 2026-09-05. Amends D-45, D-104, D-106, D-139 and D-199; advances PW-540.

**Context.** A user address space copied the kernel's present higher-half PML4
entries once. Later device mapping was allowed to allocate an absent PML4
entry, so a root created earlier could omit that mapping. Task switching still
treated the bind-time comparison as permanent, and a safe `MmioRegion` could
then be used after a switch into the stale root.

**Decision.** Activation reserves the supervisor-only top-level table covering
the complete device window before installing the kernel root. Once activation
is published, the active kernel space refuses allocation of every new
higher-half top-level table. Later device mappings can allocate only beneath
the already-shared entry, so roots created both before and after a mapping
reach the same subordinate topology. Binding continues to compare the whole
higher half, while the freeze makes that comparison stable for later switches.

**Alternatives and proof obligation.** Re-copying entries at each switch was
rejected because it silently mutates a process root and still needs an
authority and rollback story. Sampling only live code, stack and direct-map
addresses was rejected because it misses a newly populated slot. A root
generation check would detect the defect but strand an otherwise runnable
task. The retained proof instead creates a user root before a late device map
and requires the identical mapping to appear there; a reversible active-root
probe requires a new absent higher-half slot to be refused without changing
tables. Reservation-removal and freeze-removal mutations must red those exact
checks before PW-540 closes, followed by restored debug, release and VMware
boots.
