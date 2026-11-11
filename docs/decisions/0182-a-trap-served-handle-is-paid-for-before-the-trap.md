# D-182 A trap-served handle is paid for before the trap

Status: Accepted, 2026-08-26. Amends D-78, D-128, D-141, D-151, D-171, D-180 and D-181.

**Decision.** A handle table is not made reachable until the global arena has
capacity for its entire quota. Construction is fallible and commits that quota
outside trap context. The arena records the sum of every live table's quota;
its `Vec` capacity is always at least that sum. A charged insertion therefore
finds either an existing free slot or unused capacity and never grows the
buffer. Dropping a table releases the commitment after removing its entries,
but retained capacity is harmless and avoids a shrink/allocation cycle.

The invariant is stronger than pre-growing by a convenient number:

> `arena capacity >= sum(live table quotas) >= live handles + promised slots`.

It covers every interleaving, not only the capacity seen by a caller before it
drops the arena lock. Construction may race another construction, so a buffer
is allocated with no lock held, the current required commitment is rechecked,
and a winning buffer is exchanged under the leaf lock. Both the replaced and
losing buffers are freed only after that guard is gone. A failed candidate
allocation also rechecks under the leaf before reporting exhaustion: a
concurrent constructor may already have installed enough backing.

`ObjectRef` now represents stateful Event and Channel objects with `Arc`, while
it carries VFS `File` and `Directory` identities inline. D-171 deliberately
made both filesystem identities copy values with no cursor or per-open mutable
state; allocating a refcount solely to copy one into a handle bought no
lifetime property. OpenAt can consequently resolve, wrap and install either
node kind without touching the heap.

The concrete ring-3 proof needs the authority it exercises. A program's entry
block therefore contains three words: version 1, its launch argument, and the
numeric identity of its sole bootstrap handle. Version comes first so a program
can refuse a shape it does not understand before interpreting either payload
word, paying D-180's versioning requirement with the first new caller. The
handle remains unforgeable authority only because the table validates it;
hiding its number did not strengthen that property and made the bootstrap
channel unusable. A new emitted-program behaviour validates the version, reads
the handle, submits Duplicate through its own ring, validates the completion
and exits. This is the first caller of the versioned entry block rather than a
speculative process-start structure.

D-182 also narrows three rows in D-181's POSIX register. An internal method is
not merely a missing door when invoking it can reclaim memory in trap context:
Close can release the last object reference, while VM unmap and anonymous free
carry flush and reclamation obligations. All three are STUB/PARTIAL until a
deferred or otherwise trap-safe user execution contract exists.

**Rationale.** D-181's reservation covered only the table's count; installing
the result still called `reserve_a_slot`, which grew the global arena's `Vec`
when its buffer was full. The heap correctly refuses allocation in trap
context, because an interrupt can arrive while the heap is refilling itself and
re-entering it would corrupt or deadlock the allocator. So enough valid
Duplicate submissions on the shipped bootstrap channel, which carries
`DUPLICATE`, crossed the arena's next capacity boundary from the `int 0x80`
handler and made a successful ring operation an unprivileged machine panic.
The self-tests missed it because they called `Queue::serve` from kernel-thread
context and the one emitted ring-3 program submitted only Echo. OpenAt held a
second instance of the class: wrapping D-171's copyable `File` and `Directory`
values in a fresh `Arc` allocated after the trap had begun.

**Alternatives.** Growing on demand before taking `ARENA` was the old design.
It kept the lock order sound but could not make a trap allocator-safe. Returning
`ArenaFull` at the capacity boundary was rejected because admission had already
promised the result and free machine-wide handle capacity still existed.
Preallocating all 262,144 arena slots at boot would remove the trap edge by
charging every machine for the maximum; quota commitment pays only for the
tables that exist. Deferring Duplicate and OpenAt to a worker was rejected for
this slice: both operations are bounded and allocation-free once their result
storage is backed, so a scheduler handoff would add latency and state without
removing any remaining blocking work.

**Consequences.** Table construction can now report arena exhaustion or an
allocation failure, and `user::spawn` preserves that reason. `fill` contains no
reserve/grow loop. Table teardown releases objects one at a time outside the
arena guard rather than allocating a temporary vector in order to drop them.

The C transcription names the entry version, all three field offsets and its
24-byte extent. The host ABI check compares those values with the kernel's
writer and inspects the emitted program's actual entry loads, so moving a word
or accepting the wrong version cannot leave two agreeing private
transcriptions.

The former growth check is replaced by one that chooses a quota larger than
the arena's uncommitted spare capacity, observes the commitment and capacity
grow at construction, fills the table without either changing, and observes
the commitment return on drop. The emitted-program check reaches Duplicate in
an actual ring-3 trap and requires the process to retire with two handles and a
successful authority-bearing completion. OpenAt's attenuated-directory check
covers both the File and Directory result branches. Each new branch was broken
once before acceptance. The quota-undercharge mutation replaces D-128's
obsolete arena-growth mutation; two new mutations change the ring-3 request
away from Duplicate and restore universal rights on a directory result. The
ring-3 mutation targets the rebuilt kernel launch route rather than the
already-running image builder; the mutation driver now refuses any entry aimed
at its own `xtask/` source.

D-163's promise-lifetime mutation also preserves the allocation-free trap arm
while giving a kernel-thread reservation the wrong table. Constructing the
wrong table unconditionally would now panic on the earlier ring-3 Duplicate
check and hide the named lifetime failure, so it would test D-182 twice and
D-163 not at all.
