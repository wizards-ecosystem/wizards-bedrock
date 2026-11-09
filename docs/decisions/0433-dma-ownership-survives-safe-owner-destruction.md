# D-433 DMA ownership survives safe owner destruction

Status: Accepted (2026-10-23), qualification OPEN; amends D-357, D-367 and D-389; advances PW-714

Ordinary `IommuTable` and `DmaSegment` ownership allowed safe destruction to
return frames that active or uncertain hardware could still reach. NVMe's
careful cleanup did not protect these safe abstractions against another safe
caller. Admission proofs also did not borrow the unit that minted them.

After final read-only preflight and before the first unit write, all five VT-d
table allocations move into `PublishedIommuTable`. Its destructor permanently
retains root, context, interrupt, invalidation-queue and second-level frames.
Unpublished allocations still reclaim normally. Destruction does not attempt
a fallible hardware shutdown.

`DmaSegment` carries a private mapping owner keyed by the never-recycled root
allocation. It acquires that owner before leaf publication and refuses a second
mapping, including one by the same unit. Successful mapping keeps the owner.
Clean refusal before any leaf is published releases it without a hardware
drain, preserving D-367. Once publication may have occurred, only proved
rollback or exact-owner revocation after clearing the leaves and completing
the required DMA drain releases it. Uncertain outcomes permanently retain
frames;
destruction while mapped retains them too. This introduces no heap allocation
into candidate selection.

The first integrated debug boot passed 293 of 294 checks but measured 25,072
bytes of candidate stack use, leaving 7,696 bytes instead of the required
8,192. The initial optional raw-address owner added a separate discriminant
to each DMA segment. The owner now wraps a nonzero frame identity: the pure,
host-tested `frame_space::frame_identity` rejects unaligned addresses and
encodes an aligned address as its frame number plus one. Physical frame zero
and the highest aligned `u64` address remain distinct and representable; the
optional identity occupies one word. The target owner-drop witness checks
that bound on the actual wrapper. This changes neither ownership transitions
nor the no-allocation rule. Both actual stack profiles must pass before the
representation change is accepted as sufficient. Boxing, padding, omitting
frame zero and widening the stack limit are rejected.

The compact representation's first debug run passed all 294 checks with
24,432 bytes used, 8,336 bytes free and a 23,280-byte deepest register-access
leaf. That is a 640-byte reduction from the failed run; it is a focused debug
result, not complete qualification. During the stack review, mutation 1,570
was found to call the nested admission stage with a nonexistent `context`
local. Its replacement now acquires the same safe thread context used by the
sequencer, preserving the intended nested-stage defect while making it
compilable. Its named host red and actual mutated-kernel compilation remain
required.

DMA and interrupt admission proofs borrow their exact `Vtd`. NVMe keeps the
installed-unit guard through the existing fresh BAR observation and PCI
admission. That path enters only the bounded PCI configuration leaf and does
not reenter the installed-unit lock or allocate.

The shipped preparation outcome passes the enum-size lint after compaction.
The self-test-only observations still make its success variant 376 bytes
against a 160-byte failure variant. Only that feature's `PrepareOutcome`
expects the size lint: transferring the inline owner avoids an additional
allocation and failure during preparation, while the existing complete
candidate stack checks retain their unchanged bound in debug and release.
The expectation must remain fulfilled, so a later layout reduction also
requires removing it.

Alternatives rejected: caller-only cleanup does not make the exports sound;
unsafe mapping APIs move an existing invariant to callers; `Arc`/`Vec` pin
storage changes the candidate allocation window; and hardware shutdown from
Drop cannot promise safe reclamation after a refusal or uncertain transition.

Four machine checks cover actual mapped-owner destruction, actual published
table destruction, live exact-owner revocation and proved rollback/remapping.
The inert Drop drivers publish no leaf in the installed unit. Compiler probes
cover forgery, owner destruction, mutable reuse and escaped proof lifetimes;
positive probes retain live paired admission. Parsed production contracts bind
all five fields, constructors, mapping ownership and drain-before-release.
The API audit reviews the exact `DmaSegment` and `PublishedIommuTable` Drop
types and their `From::from` and `new` producers. Existing forgery
mutations 1,554 and 1,728 now expose both private fields so their original
unit-free construction defect remains reachable.

Host boundary checks and retained mutations additionally cover misalignment,
frame zero, high-address identity truncation and a coherently expanded target
owner representation. The layout mutation remains a compiling defect so the
machine witness, rather than a compiler error, must reject it.

These checks and their retained mutations must execute. Candidate resource
accounting, both stack profiles, current QEMU and fresh VMware remain required;
the limits are not widened to accommodate a regression. The expected machine
count becomes 294 on both sides of the verdict protocol.
