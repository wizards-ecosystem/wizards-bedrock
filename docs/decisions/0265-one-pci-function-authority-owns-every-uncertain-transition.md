# D-265 One PCI function authority owns every uncertain transition

Status: Accepted (2026-08-31), amends D-187, D-197 and D-251; advances PW-118, PW-125 and PW-129

**Context.** Address equality was not function authority. A command snapshot
could be replayed through another function or another registry with the same
generation, nested command transitions could overlap, and a failed BAR restore
could retain uncertain hardware without returning the one owner allowed to
retry it.

**Decision.** The exclusive function claim mints one non-duplicable authority
bound to its issuing registry, canonical function address and generation.
Memory decode, bus mastering, command commit/restore and BAR probe/retry borrow
or consume registry-bound subleases from that authority. The verified header,
not a caller count, fixes the BAR slots. A possibly effective write is settled
only after exact current-state observation; uncertain restoration returns a
linear retry owner and continues to exclude every competing transition.

**Alternatives and current evidence.** Raw `Address` write APIs and caller BAR
counts were rejected because neither proves ownership or header shape. Dropping
uncertain state as a leak was rejected because it loses both recovery and
exclusion. Fake configuration tests cover cross-function, cross-registry,
nested, changed-word, bridge-header and one-shot restore cases. Compiler probes
reject raw calls, forgery, `Copy` and `Clone`; retained host mutations attack
each identity component. Native observations remain PW-201 and the current
boot rows. This authority decision does not settle the separately audited NVMe
lifecycle sequencing or physical BAR-admission constraints.
