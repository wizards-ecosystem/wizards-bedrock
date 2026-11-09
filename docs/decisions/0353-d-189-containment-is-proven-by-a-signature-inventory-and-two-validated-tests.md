# D-353 D-189 containment is proven by a signature inventory, and two validated tests land

Status: Accepted (2026-09-29), amends D-189; closes PW-563

**Context.** The 2026-08-29 handoff left two validated test files unlanded and
two audit findings on no register row. The first finding: D-189's containment,
that no safe kernel module borrows the running `AddressSpace` with its reclaim
and arbitrary-unmap authority, was proven by a name-specific string search over
`kernel/src/mem.rs`. It asserts `with_active_space` is present as a private `fn`
and absent as `pub(crate) fn`. A rename is caught loudly; a second borrow route
cut under a different name is not, because the search checks that one door is
locked without looking at whether another was opened. The two unlanded tests
each guard a defect nothing else checks: `trap-cause` and `x86-tables` privately
transcribe the same architectural table and once disagreed about vector 28, and
the handle is the most identity-sensitive integer in the kernel.

**Decision.** The name search is replaced by a signature inventory. A `syn`
walk over `mem` records every item visible outside the module whose signature
or accessible field names `AddressSpace`; the set must be empty but for an
audited allowlist, which is empty. A rename still fails compilation; a new
borrow route under any name now fails the inventory. The two tests land with
the reach their module docs state: `libs/trap-cause/tests/exhaustive.rs` walks
all 256 vectors and requires `trap-cause` and `x86-tables` to agree on which
are architectural and which push an error code, and
`libs/handle-space/tests/properties.rs` checks the four properties its module
doc names under `proptest` (D-242).

**Evidence.** On 2026-09-29 the trap-cause suite passed all 256 vectors and the
handle-space suite passed its four properties. The inventory passed on the
current `mem`, and under mutation 301, which re-exports the private borrow as
`pub(crate)`, it reddened naming `with_active_space`. The old name search is
removed and mutation 301 now names the inventory.

**Alternatives.** A compile probe that tries to take the borrow from outside
`mem` and must fail, the staged audit's proposal, was rejected because the
inventory needs no throwaway crate and reads the exact source that ships.
Leaving the name search in place was rejected because its blind spot is the
finding itself.
