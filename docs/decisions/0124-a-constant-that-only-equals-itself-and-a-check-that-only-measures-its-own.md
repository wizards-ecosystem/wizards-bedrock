# D-124 A constant that only equals itself, and a check that only measures its own description

Status: Accepted (2026-08-24)

**Context.** The sixth adversarial sweep over Phases 0–6 went looking for the
same failure D-28 exists to prevent, and found it in two shapes neither the
rule nor `cargo xtask mutate` reaches.

*The first shape: a constant pinned to itself.* Roughly fifteen `libs/`
constants were asserted only against expressions that named them. `assert_eq!(
GENERATION_BITS, 32 - INDEX_BITS - RESERVED_BITS)` is a definition restated;
`assert!(bit & bit != 0)` is true of every single bit; a sweep whose ceiling is
spelled `MAX_WEIGHT` cancels the constant on both sides. Every one was changed
and the suite stayed green. `x86-port-space`'s `PORT_COUNT` was halved and
nothing failed, because x86's I/O space being sixteen bits appeared nowhere but
in the constant. `SPEC_CTRL_MSR` was referenced by no test at all.

*The second shape: a check that measures a description.* The frame allocator's
run check asserted the run's *address* was aligned, its *count* was eight, and
the allocator's *tally* moved by eight — three properties of what the allocator
reported, all of which hold for an allocator that hands back eight scattered
frames and describes the first. The heap's reuse check compared frame counts,
and a slab carves hundreds of slots per frame, so leaking every free never
reached a refill. The clock sweep seeded `previous = 0`, which made the first
comparison vacuous — and attempt zero is the one sample taken under a lock,
which is the whole reason it is special. The console's check was named "scrolls
without reading the device" and read a boolean saying a buffer had been
allocated.

*And a third, found by the fix to the first two.* `D-121` was cited in eight
places across `wost` and `kernel`, and no such entry existed in this log. Every
one of those comments reads as a pointer to a rationale and every one led
nowhere. Nothing in the gate looked at this file.

**Options.**

1. **Fix the instances.** What the previous five sweeps did. It works once, and
   the sixth sweep is evidence about how long it lasts.
2. **Extend `cargo xtask mutate` to `libs/`.** It boots a kernel per mutation,
   which is the wrong instrument for a host test — and a mutated constant is a
   *compile*, not a boot, so the cost is entirely wasted.
3. **Fix the instances, and add the checks that make each class visible.**

**Decision.** Option 3, with the classes handled differently because they fail
differently.

*Constants get a second source.* Each is now asserted against the number from
the document that fixes it — the SDM, the ELF specification, the architecture —
written as a literal at the assertion. `PORT_COUNT` is checked against 65,536,
not against `PORT_COUNT`. A literal beside the constant is a transcription
error away from wrong, which is a real cost and the smaller one: a wrong
transcription fails loudly the first time it is compared, and a self-comparison
never fails at all.

*Checks measure the thing.* A contiguous run is written across its whole span,
a frame taken from underneath it must land outside, and the marks must survive.
The heap's reuse is the **same address back** — the free list is LIFO, so a
freed slot is the next slot of that class, and a leak makes it a different one.
`black_box` on each allocation is load-bearing and was found the hard way:
without it LLVM elided the pair, both addresses were equal because neither
allocation happened, and the check went red with the heap correct. The clock
sweep seeds from a real reading. The console's check is renamed to what it
looks at.

*The log gets a gate.* `xtask/src/decisions.rs`, two host tests: the entries run
from `D-01` upward with no gap and no repeat, and **every `D-NNN` written
anywhere in the tree names an entry that exists**. Milliseconds, in `cargo
test`, so it runs at every commit.

**Rationale.** The two shapes share a root, which is why they are one decision:
a check written from the *implementation* rather than from the *claim* passes
by construction. `GENERATION_BITS == 32 - INDEX_BITS - RESERVED_BITS` is what
the code says; the claim is that a handle is thirty-two bits wide because that
is what fits a register. The run check asserted what `allocate_contiguous`
returned; the claim is that the frames are consecutive. Restating the
implementation always passes, and reads exactly like a check.

D-28's rule — break it once and watch it go red — catches this at the moment
the check is written and never again. Both classes here were written correctly
and *became* unfailable: `MAX_WEIGHT` moved and the sweep followed it;
`allocate_contiguous` grew a tally and the check started reading it. That drift
is what the mutation task was built for and what a task cannot reach when the
mutation is a `const` and the verdict is a compile.

The decision-log gate is the same argument at the level of the record. Two
hundred comments cite this file. The citation is load-bearing — it is how a
future session finds out *why* — and one in eight places pointed at nothing.

**Consequences.**

- Fifteen `libs/` constants now carry the number from the manual beside them,
  with the source named. Changing one fails a host test.
- Six kernel checks measure what their names claim. Each was broken once and
  watched go red, per D-28, and the breaks are in the session record.
- `xtask/src/decisions.rs` is in the gate. It failed on its first run, on the
  citations to this entry written before this entry existed — which is the
  behaviour, not an accident of ordering.
- D-121 is written, reconstructed from its eight citation sites. It says what
  they say it says: a `lock_leaf` static may not expose a closure form.
- A new ADR must now be written before it is cited, or `cargo test` refuses.
  That is a small, welcome friction: it is the same rule as D-19's
  charter-before-code, one level down.

**Left owed.** The citation check proves a number exists, not that the entry
says what the citing comment claims. Nothing can check that but reading, and
the sweep that found this one read all eight. Recorded so a later session does
not mistake a green gate for a verified rationale.
