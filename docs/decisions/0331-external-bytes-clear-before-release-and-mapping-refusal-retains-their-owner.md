# D-331 External bytes clear before release, and mapping refusal retains their owner

Status: Accepted, 2026-09-28. Amends D-107 and D-115; closes PW-117.

**Context.** `USegment::reclaim` already cleared explicit ownership recovery,
but the same invariant did not cover natural destruction. Dropping a
`USegment`, dropping the last `Arc<SharedAllocation>` or returning from
`map_user` after ownership had been extracted early could therefore return
ring-3-controlled bytes to the allocator without clearing them. A failed clear
was a separate fail-closed boundary: releasing the run after deciding that its
contents could not be overwritten would preserve the disclosure through a
different path.

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

**Evidence.** On 2026-09-28 release QEMU wrote distinct markers into a direct
external run, a last-shared run and a run passed to a mapping outside the user
half. The restored implementation recovered or dropped each exact owner,
observed allocator release only after every byte was zero and separately
injected a clear refusal that kept the frame unavailable. It passed all
262 checks.

Four retained breaks were run serially and restored between boots. Skipping
the zero and compiling out `USegment::drop` each failed only `external user
bytes are zeroed before free` and `a refused external clear withholds its run`
at 260/262. Replacing fail-closed `mem::forget` with `drop` failed only the
withholding check at 261/262. Treating `map_user` refusal as a committed success
failed `external user bytes are zeroed before free`, `cleanup refusals retain
reachable tables` and `a user mapping cannot widen a supervisor table` at
259/262. Exact restoration then passed 262/262.

**Alternatives.** Relying on zero-on-allocation was rejected because it admits
externally controlled contents into the kernel-owned free pool and makes every
future allocation path carry the secrecy obligation. Panicking on a clear
refusal was rejected because it turns a safe failure into a terminal machine
failure without making reuse safer. Extracting ownership before page-table
construction and rebuilding it during rollback was rejected because the
fallible path would again depend on every exit remembering how to reconstruct
the authority; borrowing until commit makes the compiler retain it.
