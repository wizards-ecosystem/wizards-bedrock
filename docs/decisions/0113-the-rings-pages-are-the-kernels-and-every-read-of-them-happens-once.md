# D-113 The ring's pages are the kernel's, and every read of them happens once

Status: Accepted, 2026-08-24.

**Context.** D-79 settled the queue's shape, D-112 checked the protocol against
every reachable state before this code existed, and `libs/ring-proto` holds the
index arithmetic under proptest. What none of those can be is the part that
touches pages ring 3 can rewrite between two instructions.

**Decision — the pages.** Kernel-allocated, kernel-**kept**. `map_shared` maps
a `USegment` without writing the `OWNED` bit, so the space can never hand those
frames to the allocator, and the ring holds the run for its own life. A program
that unmapped its ring could not free the memory the kernel is about to write a
completion into. That is D-79's "kernel-allocated ring pages, single-owner
rings" made structural rather than documented, and it reuses the ownership
discipline D-88 already built: not-owned means not the space's to give back.

The obligation `map_shared` cannot enforce is stated on it: the run must outlive
the mapping, an `AddressSpace` has no `Drop` and a `USegment` does, and the one
caller keeps the run in the process's `Bootstrap`. Naming what a signature does
not enforce is the alternative to pretending it does.

**Amended by D-185.** That obligation was itself a safe-API defect. The space
now retains a non-accessing ownership reference until its mappings become
unreachable; callers no longer carry the lifetime proof in prose.

**Decision — the reads.** Every read of the shared pages goes through a
`Snapshot` taken once per `serve` and never refreshed, and no function takes an
index it did not compute itself. CVE-2025-39816 is literally "always use
`READ_ONCE()` to read ring pointers"; a double fetch is how a validated value
becomes a different value before it is used. `Submission` is a *copy* rather
than a view, so nothing can be re-read after validation — the type enforces it
instead of a comment asking for it.

Three things follow, and each has a check:

- **A submission rewritten after the kernel took it changes nothing.** The
  completion already posted is what the copy said.
- **Admitting is not publishing.** A program may reserve room for three and
  write one; the kernel serves one. The reservation says where a completion
  *can* go, the tail says what is actually there, and serving the difference
  would report completions for entries still holding whatever the page held.
- **The kernel reaps only up to the head userspace published.** Reclaiming a
  slot the program has not read would be the kernel deciding what a program has
  seen.

**Decision — the completions.** `perform` has no path that reaches its end
without a completion, including for an opcode this kernel does not serve — that
is refused *with* a completion rather than ignored. D-79's terminal-completion
rule as control flow. The opcode surface is two, and no privileged ones:
io_uring's exploit record is a warning about surface growth, and Google's
kernelCTF disables it outright.

Every spawned program gets a ring at a fixed address in its own space, which is
what having a space of its own buys — a fixed ABI address costs nothing when no
two programs share a table.

**Consequences.** Five checks, at 124, each broken and red: the shared pages
made ownable, the tail trusted rather than bounded, an unserved opcode
reporting nothing, admission not reserving, and the reap unbounded. Two of the
five needed a second attempt before they could fail at all — the tail bound was
redundant with the kernel's own ring until the check asked about *publishing*
rather than about a runaway value, and the reap bound could not be told from
"reap everything" until the check reaped only half.

`translate_in_root` is new and public: since D-106 no program is in the running
space, so nothing could ask what a program's own tables say. Not behind a
feature, because `wost` has none and adding one would add a pass to the gate's
matrix (D-22) for a function that reads.

**And the stacks grew.** Building a process moves an `AddressSpace` — eight
hundred bytes of device table — through several frames, and a debug build
elides none of the copies. Sixteen kilobytes stopped being enough, and the
overflow arrived as a double fault on a **guard page** in the debug boot D-65
added for exactly this class. That is the whole arrangement working: an
unguarded stack would have written into its neighbour and been found much
later, by something else (D-52, D-89). Thirty-two kilobytes now.

Left owed: the program cannot yet *drive* its ring — the 22-byte hand-written
initrd would need to write an entry and bump a tail, which is a bigger program
than the loader's checks want. The kernel side is exercised through the shared
page, which is the boundary that matters, and a program crossing it is what
slice 10's libc arrives to make ordinary. Nothing blocks on a ring yet, so the
wait queue is built and unused. And no completion carries a handle.
