# D-84 The ring protocol is modelled before it is written

Status: Accepted, 2026-08-24.

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

**Rationale.** The hardware to show a ring protocol's ordering bugs arrives in
Phase 12, and the one bug class a single CPU cannot show, Store→Load
reordering, is the shape of the lost-wakeup re-check the ring depends on; only
a model can see it today. Rust does not prevent this: Converos (ATC'25) found
twenty bugs in safe Asterinas concurrency code. The data layout decides proof
cost later, and it is chosen when the ring and handle table are written, not
afterwards.

**Alternatives.** Write the protocol and test it (rejected: one CPU cannot
observe the reordering that matters, and the first caller freezes the
interface). Verus on the ring (rejected for now: its proof ratios are for
page-table logic with a settled layout, and a protocol still in design is the
wrong first target). Nothing, with the tier recorded as debt (rejected: after
Phase 5 the layout is fixed).

**Consequences.**

- `cargo xtask model-check` is a new task. It joins `ci` only if fast enough not
  to damage the loop. Otherwise it is a named task the slice runs, and this ADR
  is amended to say so.
- The two `proptest` pins are a recorded tax under D-16: rustup's GNU toolchain
  ships no `as.exe`, so `getrandom >= 0.3.2` fails to build. Slice 10's
  vendored clang retires them by supplying the missing assembler.
- Verus stays unadopted, with `010` §9's corrected cost on record so no later
  pass re-derives it.
