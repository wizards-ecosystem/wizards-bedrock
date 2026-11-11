# D-113 The ring's pages are the kernel's, and every read of them happens once

Status: Accepted, 2026-08-24.

**Decision: the pages.** Kernel-allocated, kernel-**kept**. `map_shared` maps a
`USegment` without writing the `OWNED` bit, so the space can never hand those
frames to the allocator, and the ring holds the run for its own life. A program
that unmapped its ring could not free memory the kernel will write into. This
makes D-79's "kernel-allocated ring pages, single-owner rings" structural, and
it reuses the ownership discipline of D-88: not-owned means not the space's to
give back.

`map_shared` states the obligation it cannot enforce: the run must outlive the
mapping, an `AddressSpace` has no `Drop` and a `USegment` does, and the one
caller keeps the run in the process's `Bootstrap`. The function names what its
type does not enforce.

**Amended by D-185.** That obligation was itself a safe-API defect. The space
now retains a non-accessing ownership reference until its mappings become
unreachable; callers no longer carry the lifetime proof in prose.

**Decision: the reads.** Every read of the shared pages goes through a
`Snapshot` taken once per `serve` and never refreshed. No function takes an
index it did not compute itself. CVE-2025-39816 is "always use `READ_ONCE()` to
read ring pointers"; a double fetch turns a validated value into a different one
before use. `Submission` is a *copy*, not a view, so the type forbids a re-read
after validation.

Three things follow, and each has a check:

- **A submission rewritten after the kernel took it changes nothing.** The
  completion already posted is what the copy said.
- **Admitting is not publishing.** A program may reserve room for three and
  write one; the kernel serves one. The reservation says where a completion
  *can* go and the tail says what is there. Serving the difference would
  complete entries still holding stale page contents.
- **The kernel reaps only up to the head userspace published.** Reclaiming a
  slot the program has not read would be the kernel deciding what it has seen.

**Decision: the completions.** `perform` has no path to its end without a
completion. An opcode this kernel does not serve is refused *with* a completion.
This is D-79's terminal-completion rule as control flow. The opcode surface is
two, none privileged, because io_uring's exploit record warns about surface
growth.

Every spawned program gets a ring at a fixed address in its own space, which
costs nothing when no two programs share a table.

**Rationale.** These pages are the one part D-79's shape, D-112's model and
`libs/ring-proto`'s arithmetic do not cover: memory ring 3 can rewrite between
two instructions. Keeping the frames and copying once makes both hazards
structural instead of disciplined.

**Consequences.** `translate_in_root` is public: since D-106 no program is in
the running space, so nothing else can ask what a program's own tables say. It
has no feature gate, because `wost` has none and one would add a pass to the
gate's matrix (D-22) for a function that reads.

Kernel stacks are thirty-two kilobytes, because building a process moves an
`AddressSpace` through several frames and a debug build elides none of the
copies. The overflow arrived as a double fault on a **guard page** in the debug
boot D-65 added for this class (D-52, D-89).

Left owed: the program cannot yet *drive* its ring. Nothing blocks on a ring
yet, so the wait queue is built and unused. No completion carries a handle.
