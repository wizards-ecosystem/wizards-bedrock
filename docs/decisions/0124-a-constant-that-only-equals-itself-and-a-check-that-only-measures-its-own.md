# D-124 A constant that only equals itself, and a check that only measures its own description

Status: Accepted, 2026-08-24.

**Decision.** **Write the check from the claim, not the implementation**, and
make each class of failure visible:

- *Constants get a second source.* Each is asserted against the number from the
  document that fixes it (the SDM, the ELF specification, the architecture),
  written as a literal at the assertion. `PORT_COUNT` is checked against
  65,536, not against `PORT_COUNT`. A literal beside the constant is a
  transcription error away from wrong, which is the smaller cost: a wrong
  transcription fails loudly the first time it is compared, and a
  self-comparison never fails at all.
- *Checks measure the thing, not its description.* A contiguous run is written
  across its whole span, a frame taken from underneath it must land outside,
  and the marks must survive; the allocator's reported address, count and
  tally hold for one that hands back scattered frames. The heap's reuse is the
  **same address back**, since the free list is LIFO, with `black_box` on each
  allocation so LLVM cannot elide the pair. The clock sweep seeds from a real
  reading. A check is named for what it looks at.
- *The log gets a gate.* `xtask/src/decisions.rs` checks that the entries run
  from `D-01` upward with no gap and no repeat, and that **every `D-NNN` written
  anywhere in the tree names an entry that exists**. It runs in `cargo test` at
  every commit, so a new ADR must be written before it is cited: D-19's
  charter-before-code, one level down.

**Rationale.** A check written from the *implementation* rather than from the
*claim* passes by construction: `GENERATION_BITS == 32 - INDEX_BITS -
RESERVED_BITS` is what the code says, and the claim is that a handle is
thirty-two bits wide because that is what fits a register. D-28's break-once rule catches this when the check is
written and never again, while these checks *became* unfailable as the code
moved; `cargo xtask mutate` cannot reach them when the mutation is a `const`
and the verdict is a compile. A citation is how a future reader finds out
*why*, and D-121 was cited in eight places before it existed.

**Consequences.** D-121 was written from its eight citation sites and says what
they say: a `lock_leaf` static may not expose a closure form.

**Left owed.** The citation check proves a number exists, not that the entry
says what the citing comment claims. Nothing can check that but reading.
Recorded so a later session does not mistake a green gate for a verified
rationale.
