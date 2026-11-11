# D-141 A slot handed out and not marked

Status: Accepted, 2026-08-25.

**Decision.** `Arena::claim(entry)` finds a free slot **and installs the
entry** under the one guard that found it; `find_free` is the private scan.
`insert` builds the `Entry` inside the retry loop (`ObjectRef` is an `Arc`, so
the clone is a refcount) and takes the guard once. The reservation stays
outside the lock, because growing allocates, and allocating under this guard
is the `ARENA → HEAP` order D-128 removed.

**Rationale.** `take` returned an index without marking the slot, and `insert`
filled it under a second acquisition. Two callers interleaving there would both
receive the same index, and the second write would drop a live entry in place
under the leaf guard, reaching the lock-under-a-leaf class D-121, D-126 and
D-128 exist for. "The room is reserved" and "the slot is mine" were
established under separate guards with nothing carrying the first across, the
shape of D-138's port stranding; here nothing between them allocates, so both
go under one guard. Not reachable at adoption: spawn was single-threaded and no
syscall minted a handle.

**What the check does and does not establish.** It fills a table past a grow,
and requires every handle to be distinct, to still resolve to what it named,
and the live count to match: the invariant an overwrite breaks. It **does not
reproduce the interleaving**, which needs two threads inside a window between
two lock acquisitions; a check that spawned racers to try would be flaky,
which is worse than a narrower check honestly described.

**Left owed.** The interleaving is unreproduced. If a handle-minting syscall or
a second CPU arrives, the honest way to test this class is a stress check with
a bounded flake budget, and that is a decision for whoever adds either.

`mem::with_active_space` still hands safe code `&mut` the kernel's own
`AddressSpace`, whose `Reach::Everywhere` disables both of D-105's guards, so
a caller could reclaim the kernel's tables or unmap the stack it is standing
on. No caller does. Same family as this entry, not closed.
