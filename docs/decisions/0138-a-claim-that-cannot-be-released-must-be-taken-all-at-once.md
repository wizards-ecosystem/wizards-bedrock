# D-138 A claim that cannot be released must be taken all at once

Status: Accepted, 2026-08-25.

**Decision.** A caller wanting several port spans asks for all of them in one
transaction. `PortSpace::reserve_all(&[(port, len)])` resolves every span
against the bitmap *and against the others in the same request* before it
writes a single bit, and records all or none. The pairwise half is not
decoration: `span` only sees what is already **recorded**, so without it both
halves of `[(0x60, 4), (0x62, 2)]` pass and the caller is told it exclusively
owns a range it holds twice.

`wost` gains `claim_pair` and `claim_block_pair` over it, used by
`pic::silence` and `i8042::Controller::new`. Single-port `claim` and
`claim_block` remain correct for callers wanting one. `MAX_SPANS = 8` is a
ceiling because the crate is `no_std` with no allocator; a request past it is
**refused**, not truncated, since a truncated reservation is the silent version
of the bug being fixed.

**Rationale.** A claim is **permanent**, because a driver owns its registers
for the life of the kernel, so taking one span and failing on the next strands
the first and the thing that needed both can never be built on that boot. A
permanent claim and a per-span interface cannot be combined safely by any
amount of care at the call site; moving the question to where the lock and the
bitmap are makes it answerable, where leaving it with the caller makes it a
rule (D-121). Checking availability first is a TOCTOU with a permanent loss at
the end. Releasable claims would be the general fix and the wrong one *here*:
every unsafe block in `port.rs` is justified by permanence (`Port::new` is
sound because nothing else can ever cover the port), a `Drop` turns that into a
lifetime argument about a value safe code can `mem::forget`, and no driver in
this kernel gives a register window back.

**Consequences.**

- `libs/port-space` host-tests that a refused pair strands nothing, a granted
  one takes every span, a request overlapping itself is refused, and empty or
  oversized is refused.
- A machine check drives the property against a port the serial driver really
  owns, so the refusal is real rather than arranged: the pair is refused, the
  free half is still claimable afterwards, and claiming it succeeds exactly
  once.
- D-129's partial-claim debt is discharged. Its two-caller race in
  `pic::silence` is not: that one needs a second CPU to be reachable and
  belongs with D-56's per-CPU pass, where D-126 also owes `LEAVES_HELD`.

**Left owed.** Three or more spans are supported and nothing asks for them. The
pairwise overlap check is O(n²) over at most eight entries, which is the right
trade at this size and would not be at a hundred.
