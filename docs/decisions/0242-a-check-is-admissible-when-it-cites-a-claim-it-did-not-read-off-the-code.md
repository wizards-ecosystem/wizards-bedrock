# D-242 A check is admissible when it cites a claim it did not read off the code

Status: Accepted, 2026-08-29. Extends D-84 and D-112; advances PW-516 and PW-517.

**Decision.** The tiers reach every pure crate, and one rule decides what may
join them:

> A check is admissible when its claim cites a stated invariant — a doc
> comment, an ADR, or a format specification — that it did not derive from the
> code. A check whose text could be recovered by reading the function body is a
> restatement of the implementation, not a check on it.

The technique follows from the shape of the domain, not from the tier, and
three are admitted equally. **Generated inputs** where the domain is too large
to walk. **Bounded exhaustion** where it is finite, which needs no dependency at
all and makes a strictly stronger claim than sampling. **Derived quantities**,
which compare a constant against a number read from another place and involve no
inputs, the shape of `the_two_tables_still_bound_this_probe`, and the only one
of the three that could have reached `MAX_CHAIN`.

Two obligations travel with the rule. A generated-input check **states the
measured rate at which it reaches the accepting path**, because a property
guarded by an early return on a refused constructor passes vacuously when the
guard almost always fires. And a shrunk counterexample is **committed**, not
ignored: it is the only "break it once" artifact this tier produces on its own,
and discarding it leaves the tier unable to re-run its own findings. A red
property writes `tests/*.proptest-regressions` into the tree, which makes the
next `cargo xtask mutate` refuse on an unclean checkout; that file is
committed, which is both proptest's intent and D-28's. Inline format capture
does not work inside `prop_assert!`, whose format string is expanded through
`concat!`, so positional arguments are required.

**Rationale.** D-84 adopted `proptest` and `shuttle` "for `libs/ring-proto`
only", and D-112 made it three tiers, each asked only the question it can
answer, after a model whose only fault was `Relaxed` where `Release` belonged
passed ten thousand shuttle schedules under sequential consistency. The other
twenty-four crates carried example tests only, with every input chosen by a
person, and D-241's three contradicting `libs/memory-map` contracts were
reachable only by transcribing the doc sentence and checking the code against
it. Six of the eight defects repaired on 2026-08-29 were bounds or arbiters
whose doc comment stated the correct rule while the code did something else.

**Alternatives.** Restricting the tier to `proptest` was rejected: it cannot
reach the derived-quantity class, where six of those eight defects lived, and
adopting a tier aimed at the wrong class while claiming it was working is worse
than not adopting one. Pinning the RNG seed for a reproducible gate was rejected
because it degrades the tier to 256 fixed inputs in perpetuity; D-216 binds
source bytes and tool identities, not test-run entropy, and a shrunk
counterexample committed as a regression is the durable half. Measuring line
coverage as the meter was rejected on evidence: `-C instrument-coverage` cannot
link on this toolchain, `rustc` answers `E0463 can't find crate for
profiler_builtins` and no `rust-std` this project can install on
`x86_64-pc-windows-gnu` carries the runtime, so the check would be permanently
red on the only host that exists. Differential oracles against `object` or a
host FAT implementation were rejected for now as a D-220 inventory change
bought before the cheaper tier had been spent.

**Consequences.** Six crates carry the tier. `libs/memory-map` failed three of
four properties and is D-241. The other five hold, and the negative results are
the point rather than a disappointment: `libs/path-walk` carries the crate's own
blockquote property, `libs/stack-layout` the "no two slots touch" the module doc
promises as host tests, `libs/port-space` the "false — and nothing recorded"
atomicity claim, `libs/frame-space` the ledger that returns to its baseline, and
`libs/slab-layout` walks its whole finite domain rather than sampling it.

This tier verifies code against claims. It is blind to a defect in the reading
of a verdict, such as reading a pipeline's exit status, which is the filter's
and not the gate's. What can catch that is a mechanism that refuses to proceed
on a bad premise, as `prove_green_baselines` did when it declined to credit
mutations against a red tree.
