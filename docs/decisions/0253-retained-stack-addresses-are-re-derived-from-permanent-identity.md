# D-253 Retained stack addresses are re-derived from permanent identity

Status: Accepted, 2026-08-30. Amends D-94, D-125 and D-245; advances PW-113 and PW-120.

**Context.** A copied stack-top integer could outlive the mapping identity that
justified it. Independently, a fixed device ledger and first-fit scratch arrays
put several kilobytes on debug kernel stacks, obscuring whether a fault was an
ownership defect or ordinary stack exhaustion.

**Decision.** Pinning the task arena is permanent. The task table owns one
`TaskStackPin`, each slot stores only an arena index, and every TSS or switch
write obtains a `KernelStackTop<'_>` borrowed from the corresponding retained
`StackIdentity`. Dropping an earlier token cannot clear the global pin.

`AddressSpace` keeps device extents in a fallible `Vec`. Mapping reserves all
ledger and geometry state before page-table mutation, so publication after the
commit cannot allocate or refuse. First-fit arithmetic stays in the host-tested
pure crate without a fixed projection array.

**Evidence.** Complete first/middle/last stack rewalks, stale-root checks,
pinned guard/map refusals and device-ledger rollback pass in the 247-check
release boot. Measured debug frames fell from 2,888 to 312 bytes for
`AddressSpace::new`; the direct nested MMIO path uses 1,936 bytes. The
independent first-fit oracle covers 175,232 cases.
