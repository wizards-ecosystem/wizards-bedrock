# D-84 The ring protocol is modelled before it is written

Status: Accepted, 2026-08-24.

**Context.** `010` §9 reported the verification tier closer than `008` assumed.
Verus runs on this machine on stable today, `shuttle` 0.9 builds unpinned, and
`proptest` builds with two version pins. It also named the wall: rustup's GNU
toolchain ships no `as.exe`, so any crate reaching `getrandom >= 0.3.2` or
`windows-sys >= 0.61` fails to build. §12 #35 made the sharper point. NrOS paid
13.3:1 and Atmosphere 4.3:1 in proof-to-code for the same page-table logic, and
the data layout made the difference. That layout is chosen when the ring and
handle table are written, not afterwards.

**Decision.** Phase 5 adopts two verification tiers, both scoped to the ring
protocol:

1. **A TLA+ model, checked before the protocol is implemented.**
   `tla2tools.jar` is vendored and checksum-pinned under D-25, and
   `cargo xtask model-check` runs it.
2. **`libs/ring-proto` under `shuttle` and `proptest`** on the host, with
   `proptest` taking `default-features = false` and `getrandom = "=0.3.1"`.

Each tier gets a break-test that proves it can fail:

- The model's break is a weakened invariant that **must** produce a
  counterexample.
- The concurrency tier's break is a variant using `Relaxed` where the protocol
  needs `Acquire`/`Release`, which **must** fail under shuttle.

**Rationale.** The Scalable Commutativity Rule's decisive property is that it is
analysable "before an implementation exists and before the necessary hardware is
available". That is this project's situation: the hardware to show a ring
protocol's ordering bugs arrives in Phase 12. The one bug class a single CPU
cannot show is Store→Load reordering. Its shape is the lost-wakeup re-check the
ring depends on, and only a model can see it today.

Rust does not prevent this, by measurement: Converos (ATC'25) model-checked
twelve Asterinas concurrency modules and found twenty bugs in the safe code.

The cost is small and already paid. Java 21 is on the development machine, so
the model tier costs a vendored jar and an xtask. The pins are measured, not
guessed.

**Alternatives.** Write the protocol and test it (rejected: one CPU cannot
observe the reordering that matters, and the first caller freezes the
interface). Verus on the ring (rejected for now: it runs here, but the ratios
above are for page-table logic with a settled layout, and a protocol still in
design is the wrong first target). Nothing, with the tier recorded as debt
(rejected: after Phase 5 the layout is fixed).

**Consequences.**

- `cargo xtask model-check` is a new task. It joins `ci` only if fast enough not
  to damage the loop. Otherwise it is a named task the slice runs, and this ADR
  is amended to say so.
- The two `proptest` pins are a recorded tax under D-16. Slice 10's vendored
  clang retires them by supplying the missing assembler.
- A third and fourth check location join `libs/` and `selftest.rs`. `005`'s
  test-placement table grows two rows.
- Verus stays unadopted, with `010` §9's corrected cost on record so no later
  pass re-derives it.
