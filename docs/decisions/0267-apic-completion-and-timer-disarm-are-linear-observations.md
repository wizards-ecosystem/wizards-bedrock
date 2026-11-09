# D-267 APIC completion and timer disarm are linear observations

Status: Accepted (2026-08-31), amends D-251 and D-256; advances PW-122 and PW-123

**Context.** A non-`Copy` token can still be `Clone`, and a type alone does not
make its constructor unique. One APIC delivery could therefore mint competing
EOI owners, while timer reprogramming could consume a stale claim that the
countdown had stopped.

**Decision.** `EoiToken` and `DisarmedTimer` are neither `Copy` nor `Clone`.
EOI claim first proves the exact live trap and ISR dismissal, atomically latches
that opportunity, and releases the latch only when the token completes or is
dropped. Timer disarm returns its proof only after observing zero current count;
one-shot programming consumes it and rechecks the count immediately before the
LVT write. A changed observation performs no write and mints no replacement
authority.

**Alternatives and current evidence.** Treating non-`Copy` as linear was
rejected because a manual `Clone` remains possible. Rechecking only at the
first observation was rejected because hardware can progress afterward. Pure
register planners cover running and refused timers and exact ISR ownership;
compiler probes reject both duplication traits, with independent Clone
mutations. The latch, drop and live-count breaks remain registered. Current
machine transport is still owned by the rows. External-delivery vector
assignment is a separate admission problem.
