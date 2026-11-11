# D-331 External bytes clear before release, and mapping refusal retains their owner

Status: Accepted, 2026-09-28. Amends D-107 and D-115; closes PW-117.

**Decision.** `USegment` retains its `Segment` until one of two terminal
transitions. A successful `map_user` first commits every leaf and only then
forgets the run into page-table ownership. Every other natural drop takes the
run, clears it and lets the cleared `Segment` return to the allocator. If that
clear refuses, the run is deliberately forgotten and therefore withheld.
`SharedAllocation` contains the same `USegment`, so destruction of its last
reference cannot bypass this rule.

Mapping refusal is an owning result rather than a reason alone.
`map_run_geometry` borrows the external run; only its successful result reaches
`commit_mapping`, while `UserMapError::Mapping` carries the unchanged
`USegment` on refusal. The caller may recover that exact owner for a retry or
drop it through the same zero-before-free path.

**Rationale.** `USegment::reclaim` cleared explicit ownership recovery, but
dropping a `USegment`, dropping the last `Arc<SharedAllocation>` or returning
from `map_user` after ownership had been extracted early could return
ring-3-controlled bytes to the allocator uncleared. Releasing a run whose clear
refused would preserve the same disclosure through a different path. Four
retained breaks of the zero, the drop, the fail-closed `mem::forget` and the
owning refusal fail `external user bytes are zeroed before free` or
`a refused external clear withholds its run`.

**Alternatives.** Relying on zero-on-allocation was rejected because it admits
externally controlled contents into the kernel-owned free pool and makes every
future allocation path carry the secrecy obligation. Panicking on a clear
refusal was rejected because it turns a safe failure into a terminal machine
failure without making reuse safer. Extracting ownership before page-table
construction and rebuilding it during rollback was rejected because the
fallible path would again depend on every exit remembering how to reconstruct
the authority; borrowing until commit makes the compiler retain it.
