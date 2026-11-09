# D-138 A claim that cannot be released must be taken all at once

Status: Accepted, 2026-08-25.

**Context.** D-129 recorded a property of the I/O port registry and did not fix
it: a claim is **permanent**. Nothing releases, because a driver owns its
registers for the life of the kernel, and that is right. What it makes wrong is
a caller wanting *two* spans.

Take the first, fail on the second, and the first is claimed by a caller that
no longer exists. Nothing can ever claim it again, so the thing that needed
both can never be built — on that boot or any later attempt within it.

D-129 said `pic::silence` was the one caller with that shape. D-137 found the
second, and it is worse: `i8042::Controller::new` chains `DATA` and `CONTROL`,
so a refused `CONTROL` strands `DATA` and **the keyboard controller can never
be built again**. Both were written as `?`-chains; D-129 changed one to
`.zip()`, which reaches the same outcome more reliably.

Neither could be fixed where it stood. The check and the record are two
moments, and the registry has no undo — so any single-span API leaves the
caller holding something it cannot give back.

**Options.**

1. **Make claims releasable.** `Port`/`PortBlock` gain a `Drop` that frees the
   span. It removes the trap and the reason the registry is simple: a claim
   that can be released is a claim whose exclusivity holds only while someone
   is looking, and every `// SAFETY:` comment in `port.rs` rests on "no claim
   is ever released, so nothing else can cover them".
2. **Have each caller check availability first.** `is_available` then `claim`
   is a TOCTOU with a permanent loss at the end of it, and on one CPU it is
   merely usually right.
3. **Ask for all of them in one transaction.**

**Decision.** Option 3. `PortSpace::reserve_all(&[(port, len)])` resolves every
span against the bitmap *and against the others in the same request* before it
writes a single bit, and records all or none.

The pairwise half is not decoration: `span` only sees what is already
**recorded**, so without it both halves of `[(0x60, 4), (0x62, 2)]` pass and the
caller is told it exclusively owns a range it holds twice.

`wost` gains `claim_pair` and `claim_block_pair` over it, and both callers use
them. `MAX_SPANS = 8` is a ceiling because the crate is `no_std` with no
allocator; a request past it is **refused**, not truncated, since a truncated
reservation is the silent version of the bug being fixed.

**Rationale.** The trap is not in either caller — it is in the shape of the
API. A permanent claim and a per-span interface cannot be combined safely by
any amount of care at the call site, which is why two independent authors wrote
the same defect and why D-129 could name it and not fix it. Moving the question
to where the lock and the bitmap already are makes it answerable; leaving it
with the caller makes it a rule, and D-121 is this project's record of what
rules are worth against shapes.

Why not option 1, at more length: releasing would be the general fix and it is
the wrong one *here*. Every unsafe block in `port.rs` is justified by
permanence — `Port::new` is sound because nothing else can ever cover the port.
A `Drop` turns that into a lifetime argument about a value that safe code can
`mem::forget`, and buys nothing this kernel needs: no driver in it ever gives a
register window back.

**Consequences.**

- `libs/port-space` gains `reserve_all` and `MAX_SPANS`, with four host tests:
  a refused pair strands nothing, a granted one takes every span, a request
  overlapping itself is refused, and empty or oversized is refused.
- `wost::port` gains `claim_pair` and `claim_block_pair`. Single-port `claim`
  and `claim_block` are unchanged and remain correct for callers wanting one.
- A machine check drives the property against a port the serial driver really
  owns, so the refusal is real rather than arranged: the pair is refused, the
  free half is still claimable afterwards, and claiming it succeeds exactly
  once. Reverting `claim_pair` to chained reservations turns it red.
- D-129's "Left owed" is discharged. Its other owed item — the two-caller race
  in `pic::silence` — is not: that one needs a second CPU to be reachable and
  belongs with D-56's per-CPU pass, where D-126 also owes `LEAVES_HELD`.

**Left owed.** Three or more spans are supported and nothing asks for them. The
pairwise overlap check is O(n²) over at most eight entries, which is the right
trade at this size and would not be at a hundred.
