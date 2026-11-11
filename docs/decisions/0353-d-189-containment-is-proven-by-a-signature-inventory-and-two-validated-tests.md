# D-353 D-189 containment is proven by a signature inventory, and two validated tests land

Status: Accepted, 2026-09-29. Amends D-189; closes PW-563.

**Decision.** The name search is replaced by a signature inventory. A `syn`
walk over `mem` records every item visible outside the module whose signature
or accessible field names `AddressSpace`; the set must be empty but for an
audited allowlist, which is empty. A rename still fails compilation; a new
borrow route under any name now fails the inventory. Mutation 301, which
re-exports the private borrow as `pub(crate)`, now names the inventory. The
two tests land with the reach their module docs state:
`libs/trap-cause/tests/exhaustive.rs` walks all 256 vectors and requires
`trap-cause` and `x86-tables` to agree on which are architectural and which
push an error code, and `libs/handle-space/tests/properties.rs` checks the four
properties its module doc names under `proptest` (D-242).

**Rationale.** The name search asserted that `with_active_space` in
`kernel/src/mem.rs` was a private `fn` and not a `pub(crate) fn`, so it checked
that one door was locked without looking at whether another was opened under a
different name. The two tests each guard a defect nothing else checks:
`trap-cause` and `x86-tables` privately transcribe the same architectural table
and once disagreed about vector 28, and the handle is the most
identity-sensitive integer in the kernel.

**Alternatives.** A compile probe that tries to take the borrow from outside
`mem` and must fail, the staged audit's proposal, was rejected because the
inventory needs no throwaway crate and reads the exact source that ships.
Leaving the name search in place was rejected because its blind spot is the
finding itself.
