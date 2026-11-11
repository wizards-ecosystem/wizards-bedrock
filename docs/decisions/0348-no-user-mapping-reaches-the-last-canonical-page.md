# D-348 No user mapping reaches the last canonical page

Status: Accepted, 2026-09-29. Amends D-102 and D-124; closes PW-558.

**Decision.** The last canonical page is never user memory. `libs/x86-paging`
exports `USER_MAPPING_END = USER_END - PAGE_SIZE` and `user_mapping_fits`, a
pure bound with a doctest. `AddressSpace::may_reach` admits a user range only
while it fits below `USER_MAPPING_END`; the ELF loader applies the same bound
to every placed segment. The kernel half and the reclaimed-space rules are
unchanged. The last byte a mapping can now hold is `USER_MAPPING_END - 1`, so
the largest saved return address is `USER_MAPPING_END`, still canonical.

**Rationale.** User memory ran to `USER_END` at `0x0000_8000_0000_0000`, the
first address of the canonical hole, so an instruction ending on the last
page, `0x0000_7fff_ffff_f000`, would push the non-canonical return address
`USER_END`. The CPU raises `#GP` when `iretq` loads a non-canonical `RIP`, in
ring 0 on the kernel's stack, so a user program could turn its own boundary
into a kernel fault that D-102's ring-3 fault retirement cannot attribute.
Mutations 1,524 through 1,526 retain the pure, `wost` and loader breaks.

**Alternatives.** Adding a `#GP` recovery arm that retires the program was
rejected because the fault is taken during the kernel's own `iretq`, after the
program is no longer current, and refusing the mapping removes the state that
reaches it, so the fault is unreachable rather than handled. Keeping
`USER_END` and rejecting only a stack there was rejected because code, not
only a stack, can end on that page.
