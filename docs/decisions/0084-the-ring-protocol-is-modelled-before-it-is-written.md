# D-84 The ring protocol is modelled before it is written

Status: Accepted, 2026-08-24.

**Context.** `010` §9 reported that the verification tier is closer than `008`
assumed: Verus runs on this machine on stable today, `shuttle` 0.9 builds
unpinned, and `proptest` builds with two version pins. It also found the wall
and named it precisely — rustup's GNU toolchain ships no `as.exe`, so any crate
reaching `getrandom >= 0.3.2` or `windows-sys >= 0.61` fails to build. And §12
#35 put the sharper point: NrOS paid 13.3:1 and Atmosphere 4.3:1 in
proof-to-code for the same page-table logic, and the difference was the data
layout — a choice made when the ring and handle table are written, not
afterwards.

**Decision.** Two verification tiers are adopted in Phase 5, both scoped to the
ring protocol:

1. **A TLA+ model, checked before the protocol is implemented.**
   `tla2tools.jar` is vendored and checksum-pinned under D-25, and
   `cargo xtask model-check` runs it.
2. **`libs/ring-proto` under `shuttle` and `proptest`** on the host, with
   `proptest` taking `default-features = false` and `getrandom = "=0.3.1"`.

Both tiers get a break-test, and the break is the point:

- The model's break is a deliberately weakened invariant that **must** produce
  a counterexample.
- The concurrency tier's break is a variant using `Relaxed` where the protocol
  needs `Acquire`/`Release`, which **must** fail under shuttle.

**Rationale.** The Scalable Commutativity Rule's decisive property is that it
is analysable "before an implementation exists and before the necessary
hardware is available", and that is the situation this project is in: the
hardware that would show a ring protocol's ordering bugs is in Phase 12. The
one bug class a single CPU genuinely cannot show is Store→Load reordering,
whose shape is exactly the lost-wakeup re-check the ring depends on. A model is
the only thing that can see it today.

Rust does not save us here, and there is a measurement: Converos (ATC'25)
model-checked twelve Asterinas concurrency modules and found twenty bugs — in
the safe code.

The cost is small and already paid for. Java 21 is on the development machine,
so the model tier costs a vendored jar and an xtask. The pins are known and
measured rather than guessed.

**Alternatives.** Write the protocol and test it (rejected: single-CPU testing
cannot observe the one reordering that matters, and the interface is frozen by
its first caller); Verus on the ring (rejected for now: it runs here, but the
proof-to-code ratios above are for page-table logic with a settled layout, and
a protocol still being designed is the wrong first target); nothing, with the
tier recorded as debt (rejected: the whole argument for doing this in Phase 5
is that afterwards the layout is fixed).

**Consequences.**

- `cargo xtask model-check` is a new task. It joins `ci` only if it runs fast
  enough not to damage the loop; if it does not, it is a named task the slice
  runs and this ADR is amended to say so.
- The two `proptest` pins are a recorded tax under D-16. Slice 10's vendored
  clang is what eventually retires them, by supplying the missing assembler.
- A third and fourth check location join `libs/` and `selftest.rs`. `005`'s
  test-placement table grows two rows.
- Verus stays unadopted, with `010` §9's corrected cost on the record so the
  next pass does not re-derive it.
