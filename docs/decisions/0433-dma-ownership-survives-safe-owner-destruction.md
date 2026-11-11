# D-433 DMA ownership survives safe owner destruction

Status: Accepted, 2026-10-23. Qualification OPEN; amends D-357, D-367 and D-389; advances PW-714.

**Decision.** After final read-only preflight and before the first unit write,
all five VT-d table allocations move into `PublishedIommuTable`. Its destructor
permanently retains root, context, interrupt, invalidation-queue and
second-level frames. Unpublished allocations still reclaim normally.
Destruction does not attempt a fallible hardware shutdown.

`DmaSegment` carries a private mapping owner keyed by the never-recycled root
allocation. It acquires that owner before leaf publication and refuses a second
mapping, including one by the same unit. Successful mapping keeps the owner.
Clean refusal before any leaf is published releases it without a hardware
drain, preserving D-367. Once publication may have occurred, only proved
rollback or exact-owner revocation after clearing the leaves and completing
the required DMA drain releases it. Uncertain outcomes permanently retain
frames; destruction while mapped retains them too. This introduces no heap
allocation into candidate selection.

The owner wraps a nonzero frame identity: the pure, host-tested
`frame_space::frame_identity` rejects unaligned addresses and encodes an
aligned address as its frame number plus one. Physical frame zero and the
highest aligned `u64` address remain distinct and representable; the optional
identity occupies one word. The target owner-drop witness checks that bound on
the actual wrapper. Both actual stack profiles must pass before the
representation is accepted as sufficient. Boxing, padding, omitting frame zero
and widening the stack limit are rejected.

DMA and interrupt admission proofs borrow their exact `Vtd`. NVMe keeps the
installed-unit guard through the existing fresh BAR observation and PCI
admission. That path enters only the bounded PCI configuration leaf and does
not reenter the installed-unit lock or allocate.

Only the self-test feature's `PrepareOutcome` expects the enum-size lint: its
success variant is 376 bytes against a 160-byte failure variant, and
transferring the inline owner avoids an additional allocation and failure
during preparation, while the complete candidate stack checks keep their bound
in debug and release. The expectation must remain fulfilled, so a later layout
reduction also requires removing it.

Four machine checks cover actual mapped-owner destruction, actual published
table destruction, live exact-owner revocation and proved rollback/remapping.
The inert Drop drivers publish no leaf in the installed unit. Compiler probes
cover forgery, owner destruction, mutable reuse and escaped proof lifetimes;
positive probes retain live paired admission. Parsed production contracts bind
all five fields, constructors, mapping ownership and drain-before-release.
The API audit reviews the exact `DmaSegment` and `PublishedIommuTable` Drop
types and their `From::from` and `new` producers. Existing forgery mutations
1,554 and 1,728 expose both private fields so their original unit-free
construction defect remains reachable, and mutation 1,570 acquires the same
safe thread context as the sequencer so its nested-stage defect compiles. Host
boundary checks and retained mutations cover misalignment, frame zero,
high-address identity truncation and a coherently expanded target owner
representation; the layout mutation remains a compiling defect so the machine
witness, rather than a compiler error, must reject it.

These checks and their retained mutations must execute. Candidate resource
accounting, both stack profiles, current QEMU and fresh VMware remain required;
the limits are not widened to accommodate a regression.

**Rationale.** Ordinary `IommuTable` and `DmaSegment` ownership let safe
destruction return frames that active or uncertain hardware could still reach,
NVMe's careful cleanup did not protect them against another safe caller, and
admission proofs did not borrow the unit that minted them. The first
integrated debug boot used 25,072 bytes of candidate stack, leaving 7,696
instead of the required 8,192, because the raw-address owner added a separate
discriminant to each segment; the compact identity's first debug run used
24,432 bytes, leaving 8,336, a focused debug result rather than complete
qualification. Caller-only cleanup does not make the exports sound; unsafe
mapping APIs move an existing invariant to callers; `Arc`/`Vec` pin storage
changes the candidate allocation window; and hardware shutdown from Drop
cannot promise safe reclamation after a refusal or uncertain transition.
