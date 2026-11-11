# D-289 Every privilege entry installs the kernel stack selector before Rust

Status: Accepted, 2026-09-05. Amends D-49 and D-96; advances PW-543.

**Decision.** Common entry first saves `RAX`, uses its low half to load
`KERNEL_DATA` into live `SS`, then saves the remaining registers and enters
Rust. The CPU-pushed frame is unchanged, original `RAX` is restored, and the
same-privilege path performs an idempotent selector load. The first ring-3
call records live `SS` separately from the saved user stack selector, while the
later breakpoint retains the context-switch witness.

**Rationale.** A ring-3 interrupt changes privilege and leaves the live `SS`
null in long mode even though the CPU frame contains the old user `SS:RSP`.
Trap dispatch may switch to another saved kernel context before the original
`iretq`; that context then inherits the null live selector. A later kernel
breakpoint consequently carried a valid frame span and saved user selectors
but failed the independently expected kernel stack selector. AMD's
[Architecture Programmer's Manual, Volume 2, section 8.9](https://docs.amd.com/v/u/en-US/24593_3.44_APM_Vol2)
specifies the privilege-entry behavior.

**Alternatives and proof obligation.** Weakening the late frame check would
erase the symptom. Normalizing only scheduler return sites would miss other
trap-side exits and multiply architectural policy. Removing the common-entry
load fails the exact live-selector witness while the direct frame span still
passes; that defect remains mutation 1,313. Restored repeated debug,
release and VMware boots remain PW-543's close.

**Amended 2026-10-03.** The mutation cited above as 1,313 was this defect's
array position when the record was written, before D-341 gave every mutation
a permanent id. Its permanent id is 1,335 (`a ring-three trap can carry a null
stack selector into kernel work`); id 1,313 now names a PW-540 shared-topology
break.
