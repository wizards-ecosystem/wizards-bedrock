# D-348 No user mapping reaches the last canonical page

Status: Accepted, 2026-09-29. Amends D-102 and D-124; closes PW-558.

**Context.** User memory ran to `USER_END` at `0x0000_8000_0000_0000`, the
first address of the canonical hole. A ring-3 program could therefore hold
code or a stack on the last mapped page, `0x0000_7fff_ffff_f000`. An
instruction that ended at `USER_END` would push a return address of
`USER_END`, which is non-canonical. The CPU raises `#GP` when `iretq` loads a
non-canonical `RIP`, and that fault is taken in ring 0 with the kernel's
stack, so a user program could turn its own boundary into a kernel fault.
D-102 retires a ring-3 fault, but this one arrives inside `iretq` on the
return path, not as a fault the scheduler can attribute to the program before
resuming it.

**Decision.** The last canonical page is never user memory. `libs/x86-paging`
exports `USER_MAPPING_END = USER_END - PAGE_SIZE` and `user_mapping_fits`, a
pure bound with a doctest. `AddressSpace::may_reach` admits a user range only
while it fits below `USER_MAPPING_END`; the ELF loader applies the same bound
to every placed segment. The kernel half and the reclaimed-space rules are
unchanged. The last byte a mapping can now hold is `USER_MAPPING_END - 1`, so
the largest saved return address is `USER_MAPPING_END`, still canonical.

**Evidence.** On 2026-09-29 the `x86-paging` host test and its doctest fixed
the two constants and checked `user_mapping_fits` on both sides of the edge.
Two machine checks were added: one maps a page at `USER_MAPPING_END` and is
refused with `NotYours` while the page below maps and is released, and the
loader check refuses a segment doctored to that page before any mapping, with
its cleanup and pre-mapping evidence intact. A release self-test then passed
264/264. Mutation 1,524 relaxes the pure bound to `USER_END` and fails the
host test; mutations 1,525 and 1,526 relax the `wost` and loader bounds and
fail the two machine checks. The `iretq` recovery arm is not added: no
mapping can now produce the non-canonical return, so the fault is unreachable
rather than handled.

**Alternatives.** Adding a `#GP` recovery arm that retires the program was
rejected because the fault is taken during the kernel's own `iretq`, after the
program is no longer current, and refusing the mapping removes the state that
reaches it. Keeping `USER_END` and rejecting only a stack there was rejected
because code, not only a stack, can end on that page.
