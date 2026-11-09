# D-259 Retained stack identity is checked at map admission and hardware use

Status: Accepted (2026-08-31), amends D-94, D-125, D-214 and D-253; closes PW-113

**Context.** D-253's complete stack rewalks attacked a disposable
`MappedStack`. They did not substitute a page after the boot, task or
double-fault stack had become irrevocable, and map-side pin admission had no
retained causal mutation. `TaskStackPin::stack_top` borrowed the retained
identity but derived its address without rewalking it at the hardware-use
boundary.

**Decision.** Every safe map and unmap reaches the same complete permanent and
task-pin overlap ledger, covering each guard and stack byte. Each task stack
pointer rewalks the actual loaded CR3 immediately before use and requires the
original supervisor, write-back, four-kilobyte physical leaves with `PRESENT`,
`WRITABLE`, `NO_EXECUTE` and `OWNED`. The returned `KernelStackTop` still
borrows that exact retained identity.

The actual root matters. A switch may start while a live bound user CR3 is
loaded; its checked higher half contains the kernel stacks. Requiring the
published kernel root instead panicked the first real release boot when the
service worker followed a user task. Rewalking the loaded tables verifies what
the next hardware access will reach without accepting a stale recorded root.

Boot and double-fault stacks cannot be rechecked before every use: one is
already executing and hardware selects the other before handler code runs.
Their permanent ledger therefore makes safe remapping impossible. Self-test
may transiently replace only a noncurrent lower leaf under interrupt exclusion;
it observes rejection through the shared identity walk and restores the exact
entry and translation before returning. The map-admission probe similarly uses
an adjacent existing physical leaf, then rolls back any deliberately admitted
guard alias and table before reporting failure.

**Alternatives and evidence.** Checking only the top page was rejected because
all three retained stacks have lower leaves hardware can reach. Requiring the
kernel root was rejected by the real user-to-worker switch above. Allocating a
replacement frame or relying on normal unmap to clean an intentionally broken
pin check was rejected because either would add ownership noise or leave the
hostile guard mapped.

Release QEMU passed 251/251 with exact guard, lower and upper lookups and free
frame counts preserved around lower-leaf substitutions in the running boot
stack, a noncurrent task stack and the TSS double-fault stack. Skipping the
first leaf failed all three new checks. Disabling map-side pin admission failed
the exact protected-map check and its three older consumers, while the restored
substitution checks and all later task switches still passed, proving cleanup
left no residual alias. Restoration returned 251/251. The map-admission and
task hardware-use breaks are retained beside the existing first/middle/last
rewalk mutations.
